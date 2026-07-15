//! Transactional early registration for imported annotation bindings.

mod graph;

use crate::compiler::{BytecodeCompiler, ImportedAnnotationSymbol};
use shape_ast::ast::{ExportItem, Item};
use shape_ast::error::{Result, ShapeError};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Module-scoped annotation-import semantics. Graph dependency compilation
/// temporarily installs one snapshot; the validated root snapshot is restored
/// immediately before root declaration discovery.
#[derive(Clone)]
pub(in crate::compiler) struct AnnotationImportSemanticSnapshot {
    imported_annotations: HashMap<String, ImportedAnnotationSymbol>,
    graph_namespace_map: HashMap<String, String>,
    module_scope_sources: HashMap<String, String>,
}

impl BytecodeCompiler {
    pub(super) fn root_local_annotation_names(
        program: &shape_ast::ast::Program,
    ) -> BTreeSet<String> {
        program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::AnnotationDef(definition, _) => Some(definition.name.clone()),
                Item::Export(export, _) => match &export.item {
                    ExportItem::Annotation(definition) => Some(definition.name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Stage and validate annotation-import aliases as one transaction.
    ///
    /// The compiler must know root annotation imports before comptime
    /// declaration discovery, but source order cannot decide which of two
    /// distinct annotations owns one local spelling. Build the complete
    /// candidate set first, reject a collision deterministically, and only
    /// then let the caller commit it.
    pub(super) fn stage_annotation_import_bindings<I>(
        &self,
        requested: I,
    ) -> Result<BTreeMap<String, (String, String)>>
    where
        I: IntoIterator<Item = (String, String, String)>,
    {
        // local name -> {(module path, original annotation name)}
        let mut candidates: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        for (local_name, existing) in &self.imported_annotations {
            candidates.entry(local_name.clone()).or_default().insert((
                existing._module_path.clone(),
                existing.original_name.clone(),
            ));
        }
        for (local_name, original_name, module_path) in requested {
            candidates
                .entry(local_name)
                .or_default()
                .insert((module_path, original_name));
        }

        if let Some((local_name, identities)) = candidates
            .iter()
            .find(|(_, identities)| identities.len() > 1)
        {
            let rendered = identities
                .iter()
                .map(|(module_path, original_name)| format!("{module_path}::{original_name}"))
                .collect::<Vec<_>>()
                .join("`, `");
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Conflicting annotation imports for '@{local_name}': `{rendered}` bind the same local annotation name"
                ),
                // The compared identities, not whichever declaration happened
                // to be visited second, are the diagnostic authority.
                location: None,
            });
        }

        Ok(candidates
            .into_iter()
            .map(|(local_name, mut identities)| {
                let (module_path, original_name) = identities
                    .pop_first()
                    .expect("every staged annotation binding has one identity");
                (local_name, (original_name, module_path))
            })
            .collect())
    }

    pub(super) fn commit_annotation_import_bindings(
        &mut self,
        staged: BTreeMap<String, (String, String)>,
    ) {
        for (local_name, (original_name, module_path)) in staged {
            self.module_scope_sources
                .entry(module_path.clone())
                .or_insert_with(|| module_path.clone());
            self.imported_annotations
                .entry(local_name)
                .or_insert_with(|| ImportedAnnotationSymbol {
                    original_name,
                    _module_path: module_path.clone(),
                    hidden_module_name: module_path,
                });
        }
    }

    pub(super) fn register_annotation_import_bindings<I>(&mut self, requested: I) -> Result<()>
    where
        I: IntoIterator<Item = (String, String, String)>,
    {
        let staged = self.stage_annotation_import_bindings(requested)?;
        self.commit_annotation_import_bindings(staged);
        Ok(())
    }

    pub(in crate::compiler) fn annotation_import_semantic_snapshot(
        &self,
    ) -> AnnotationImportSemanticSnapshot {
        AnnotationImportSemanticSnapshot {
            imported_annotations: self.imported_annotations.clone(),
            graph_namespace_map: self.graph_namespace_map.clone(),
            module_scope_sources: self.module_scope_sources.clone(),
        }
    }

    pub(in crate::compiler) fn restore_annotation_import_semantics(
        &mut self,
        snapshot: AnnotationImportSemanticSnapshot,
    ) {
        self.imported_annotations = snapshot.imported_annotations;
        self.graph_namespace_map = snapshot.graph_namespace_map;
        self.module_scope_sources = snapshot.module_scope_sources;
    }

    /// Pass 2 may only consume the whole-set decision made before declaration
    /// discovery. It must never incrementally publish a new annotation alias.
    pub(super) fn consume_staged_annotation_import_binding(
        &self,
        local_name: &str,
        original_name: &str,
        module_path: &str,
    ) -> Result<()> {
        let locally_shadowed = self
            .directive_reanalysis_program
            .as_ref()
            .is_some_and(|program| Self::root_local_annotation_names(program).contains(local_name));
        if locally_shadowed {
            return Ok(());
        }

        match self.imported_annotations.get(local_name) {
            Some(binding)
                if binding.original_name == original_name
                    && binding._module_path == module_path =>
            {
                Ok(())
            }
            Some(binding) => Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal error: late annotation import '@{local_name}' resolved to '{}::{}' after staging chose '{}::{}'",
                    module_path, original_name, binding._module_path, binding.original_name
                ),
                location: None,
            }),
            None => Err(ShapeError::RuntimeError {
                message: format!(
                    "Internal error: late annotation import '@{local_name}' was not staged before declaration discovery"
                ),
                location: None,
            }),
        }
    }

    /// Pre-register only root named annotation imports before comptime
    /// declaration discovery. Permission authorization is intentionally pure
    /// here; the ordinary late import pass records blob permission metadata
    /// once the `__main__` blob exists.
    pub(in crate::compiler) fn pre_register_root_annotation_imports(
        &mut self,
        program: &shape_ast::ast::Program,
    ) -> Result<()> {
        use shape_ast::ast::ImportItems;

        let local_annotations = Self::root_local_annotation_names(program);
        let mut requested = Vec::new();
        for item in &program.items {
            let Item::Import(import_stmt, _) = item else {
                continue;
            };
            let ImportItems::Named(specs) = &import_stmt.items else {
                continue;
            };
            if !specs.iter().any(|spec| spec.is_annotation) {
                continue;
            }

            if let Some(pset) = self.permission_set.as_ref() {
                self.authorize_import_permissions(import_stmt, pset)?;
            }
            requested.extend(
                specs
                    .iter()
                    .filter(|spec| spec.is_annotation)
                    .filter_map(|spec| {
                        let local_name = spec.alias.clone().unwrap_or_else(|| spec.name.clone());
                        (!local_annotations.contains(&local_name))
                            .then(|| (local_name, spec.name.clone(), import_stmt.from.clone()))
                    }),
            );
        }

        self.register_annotation_import_bindings(requested)
    }
}
