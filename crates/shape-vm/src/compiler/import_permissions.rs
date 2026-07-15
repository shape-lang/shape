//! Authenticated permission ownership for module-graph compilation.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::bytecode::Function;
use crate::module_graph::{ModuleGraph, ModuleId, ResolvedImport};
use shape_ast::error::{Result, ShapeError};
use shape_abi_v1::PermissionSet;

use super::{BytecodeCompiler, FunctionBlobBuilder};

const MODULE_PERMISSION_BLOB_DOMAIN: &str = "\0shape.module-import-permissions::";

#[derive(Debug)]
struct PendingModuleImportPermissions {
    owner: ModuleId,
    canonical_path: String,
    required: PermissionSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GraphModulePermissionKind {
    User,
    StdlibBootstrap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveGraphPermissionOwner {
    module_id: ModuleId,
    canonical_path: String,
    kind: GraphModulePermissionKind,
}

#[derive(Debug)]
pub(super) struct GraphPermissionOwnerToken {
    owner: ActiveGraphPermissionOwner,
}

#[derive(Debug, Default)]
pub(super) struct GraphPermissionState {
    pending: HashMap<ModuleId, PendingModuleImportPermissions>,
    active_owner: Option<ActiveGraphPermissionOwner>,
}

impl BytecodeCompiler {
    pub(super) fn enter_graph_permission_owner(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
    ) -> Result<GraphPermissionOwnerToken> {
        if let Some(active) = &self.graph_permission_state.active_owner {
            return Err(Self::permission_state_error(format!(
                "cannot enter graph module {:?} while {:?} owns permission derivation",
                module_id, active.module_id
            )));
        }

        let node = graph.node(module_id);
        let kind = if graph.is_stdlib_bootstrap(module_id) {
            GraphModulePermissionKind::StdlibBootstrap
        } else {
            GraphModulePermissionKind::User
        };
        let owner = ActiveGraphPermissionOwner {
            module_id,
            canonical_path: node.canonical_path.clone(),
            kind,
        };
        self.graph_permission_state.active_owner = Some(owner.clone());
        Ok(GraphPermissionOwnerToken { owner })
    }

    pub(super) fn leave_graph_permission_owner(
        &mut self,
        token: GraphPermissionOwnerToken,
    ) -> Result<()> {
        match self.graph_permission_state.active_owner.take() {
            Some(active) if active == token.owner => Ok(()),
            Some(active) => {
                self.graph_permission_state.active_owner = Some(active.clone());
                Err(Self::permission_state_error(format!(
                    "graph permission owner mismatch: active {:?}, leaving {:?}",
                    active.module_id, token.owner.module_id
                )))
            }
            None => Err(Self::permission_state_error(format!(
                "graph permission owner {:?} was already cleared",
                token.owner.module_id
            ))),
        }
    }

    /// Derive the complete resolved-import permission union, authorize it when
    /// the compiler is bound to an explicit grant, then stage it for the sole
    /// authenticated owner. An unbound compiler still emits exact metadata.
    pub(super) fn authorize_and_stage_graph_import_permissions(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
        resolved_imports: &[ResolvedImport],
    ) -> Result<()> {
        let mut required = PermissionSet::pure();
        for resolved in resolved_imports {
            match resolved {
                ResolvedImport::Namespace { canonical_path, .. } => {
                    required = required.union(
                        &shape_runtime::stdlib::capability_tags::module_permissions(canonical_path),
                    );
                }
                ResolvedImport::Named {
                    canonical_path,
                    symbols,
                    ..
                } => {
                    for symbol in symbols {
                        required = required.union(
                            &shape_runtime::stdlib::capability_tags::required_permissions(
                                canonical_path,
                                &symbol.original_name,
                            ),
                        );
                    }
                }
            }
        }
        if let Some(granted) = self.permission_set.clone() {
            for resolved in resolved_imports {
                match resolved {
                    ResolvedImport::Namespace { canonical_path, .. } => {
                        self.authorize_import_module_permissions(canonical_path, &granted)?;
                    }
                    ResolvedImport::Named {
                        canonical_path,
                        symbols,
                        ..
                    } => {
                        for symbol in symbols {
                            self.authorize_import_symbol_permissions(
                                canonical_path,
                                &symbol.original_name,
                                &granted,
                            )?;
                        }
                    }
                }
            }
        }

        let canonical_path = graph.node(module_id).canonical_path.clone();
        if module_id != graph.root_id() {
            self.validate_active_dependency_owner(module_id, &canonical_path)?;
        }
        if self.graph_permission_state.pending.contains_key(&module_id) {
            return Err(Self::permission_state_error(format!(
                "module {:?} already has pending import permissions",
                module_id
            )));
        }
        self.graph_permission_state.pending.insert(
            module_id,
            PendingModuleImportPermissions {
                owner: module_id,
                canonical_path,
                required,
            },
        );

        if module_id == graph.root_id() {
            let pending = self.take_pending_module_permissions(module_id, graph)?;
            self.attach_root_import_permissions(pending)
        } else {
            Ok(())
        }
    }

    /// Consume one dependency's staged permission state only after all of its
    /// functions and module initialization have compiled.
    pub(super) fn complete_graph_import_permissions(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
    ) -> Result<()> {
        if module_id == graph.root_id() {
            return Err(Self::permission_state_error(
                "root import permissions complete on the __main__ path".to_string(),
            ));
        }
        let pending = self.take_pending_module_permissions(module_id, graph)?;
        self.publish_dependency_permission_blob(pending)
    }

    /// Remove staged state after a dependency compilation failure. Absence is
    /// valid when the failure happened before import authorization completed.
    pub(super) fn discard_graph_import_permissions(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
    ) -> Result<()> {
        let Some(pending) = self.graph_permission_state.pending.remove(&module_id) else {
            return Ok(());
        };
        let expected_path = &graph.node(module_id).canonical_path;
        if pending.owner != module_id || pending.canonical_path != *expected_path {
            return Err(Self::permission_state_error(format!(
                "discarded import-permission owner mismatch for {:?}",
                module_id
            )));
        }
        Ok(())
    }

    /// Derive capability requirements on the function that actually issues the
    /// call. Only graph construction's typed embedded-stdlib provenance may
    /// suppress bootstrap restamping.
    pub(super) fn record_owned_capability_call_permissions(
        &mut self,
        module: &str,
        function: &str,
    ) {
        if matches!(
            self.graph_permission_state
                .active_owner
                .as_ref()
                .map(|owner| &owner.kind),
            Some(GraphModulePermissionKind::StdlibBootstrap)
        ) {
            return;
        }
        self.record_blob_permissions(module, function);
    }

    fn take_pending_module_permissions(
        &mut self,
        module_id: ModuleId,
        graph: &ModuleGraph,
    ) -> Result<PendingModuleImportPermissions> {
        let pending = self
            .graph_permission_state
            .pending
            .remove(&module_id)
            .ok_or_else(|| {
                Self::permission_state_error(format!(
                    "module {:?} has no pending import permissions",
                    module_id
                ))
            })?;
        let expected_path = &graph.node(module_id).canonical_path;
        if pending.owner != module_id || pending.canonical_path != *expected_path {
            return Err(Self::permission_state_error(format!(
                "pending import-permission owner mismatch for {:?}",
                module_id
            )));
        }
        Ok(pending)
    }

    fn attach_root_import_permissions(
        &mut self,
        pending: PendingModuleImportPermissions,
    ) -> Result<()> {
        if self.graph_permission_state.active_owner.is_some() {
            return Err(Self::permission_state_error(
                "root import permissions cannot be consumed by a dependency owner".to_string(),
            ));
        }
        let builder = self.current_blob_builder.as_mut().ok_or_else(|| {
            Self::permission_state_error(
                "root import permissions require the active __main__ blob".to_string(),
            )
        })?;
        if builder.name != "__main__" {
            return Err(Self::permission_state_error(format!(
                "root import permissions cannot attach to blob '{}'",
                builder.name
            )));
        }
        builder.record_permissions(&pending.required);
        Ok(())
    }

    fn publish_dependency_permission_blob(
        &mut self,
        pending: PendingModuleImportPermissions,
    ) -> Result<()> {
        let active = self
            .graph_permission_state
            .active_owner
            .as_ref()
            .ok_or_else(|| {
                Self::permission_state_error(format!(
                    "dependency {:?} has no active permission owner",
                    pending.owner
                ))
            })?;
        if active.module_id != pending.owner || active.canonical_path != pending.canonical_path {
            return Err(Self::permission_state_error(format!(
                "dependency permission owner mismatch for {:?}",
                pending.owner
            )));
        }

        let blob_name = Self::module_permission_blob_name(&pending.canonical_path);
        if self.blob_name_to_hash.contains_key(&blob_name)
            || self.completed_blobs.iter().any(|blob| blob.name == blob_name)
        {
            return Err(Self::permission_state_error(format!(
                "duplicate authenticated module-permission carrier for '{}'",
                pending.canonical_path
            )));
        }
        if let Some(builder) = &self.current_blob_builder {
            return Err(Self::permission_state_error(format!(
                "module-permission carrier cannot overlap active blob '{}'",
                builder.name
            )));
        }

        // The builder's interval is deliberately zero-length. It is created only
        // after module compilation is complete and never spans dependency functions.
        let offset = self.program.current_offset();
        let mut builder = FunctionBlobBuilder::new(
            blob_name.clone(),
            offset,
            self.program.constants.len(),
            self.program.strings.len(),
        );
        builder.record_permissions(&pending.required);
        let function = Function {
            name: blob_name.clone(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            entry_point: offset,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        };
        let blob = builder.finalize(
            &self.program,
            &function,
            &self.blob_name_to_hash,
            offset,
            Vec::new(),
            Vec::new(),
        );
        if self
            .completed_blobs
            .iter()
            .any(|existing| existing.content_hash == blob.content_hash)
        {
            return Err(Self::permission_state_error(format!(
                "authenticated module-permission carrier hash collision for '{}'",
                pending.canonical_path
            )));
        }

        self.blob_name_to_hash
            .insert(blob_name, blob.content_hash);
        if let Some(cache) = self.blob_cache.as_mut() {
            cache.put_blob(&blob);
        }
        self.completed_blobs.push(blob);
        Ok(())
    }

    fn validate_active_dependency_owner(
        &self,
        module_id: ModuleId,
        canonical_path: &str,
    ) -> Result<()> {
        let active = self
            .graph_permission_state
            .active_owner
            .as_ref()
            .ok_or_else(|| {
                Self::permission_state_error(format!(
                    "dependency {:?} has no active permission owner",
                    module_id
                ))
            })?;
        if active.module_id != module_id || active.canonical_path != canonical_path {
            return Err(Self::permission_state_error(format!(
                "dependency permission owner mismatch for {:?}",
                module_id
            )));
        }
        Ok(())
    }

    fn module_permission_blob_name(canonical_path: &str) -> String {
        format!("{MODULE_PERMISSION_BLOB_DOMAIN}{canonical_path}")
    }

    fn permission_state_error(message: String) -> ShapeError {
        ShapeError::RuntimeError {
            message: format!("Internal permission-state error: {message}"),
            location: None,
        }
    }

    #[cfg(test)]
    fn pending_module_permission_count(&self) -> usize {
        self.graph_permission_state.pending.len()
    }
}
