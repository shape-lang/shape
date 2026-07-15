//! Module-scoped graph annotation-import staging.

use super::AnnotationImportSemanticSnapshot;
use crate::compiler::{BytecodeCompiler, ImportedAnnotationSymbol};
use crate::module_graph::{ModuleGraph, ModuleId, ResolvedImport};
use shape_ast::ast::{ImportItems, Item, Program};
use shape_ast::error::{Result, ShapeError};
use std::collections::{BTreeMap, BTreeSet};

type AnnotationIdentity = (String, String);

impl BytecodeCompiler {
    /// Build one module's annotation-import view without publishing it.
    ///
    /// The source AST proves which rows are explicit. Explicit bindings own
    /// their local spelling; resolved rows absent from that source are
    /// synthetic/prelude vacancy fills. The returned snapshot can therefore
    /// be installed for exactly one module's declaration-discovery window.
    pub(in crate::compiler) fn stage_graph_annotation_imports_for_module(
        &self,
        program: &Program,
        module_id: ModuleId,
        graph: &ModuleGraph,
    ) -> Result<AnnotationImportSemanticSnapshot> {
        let local_annotations = Self::root_local_annotation_names(program);
        let mut snapshot = self.annotation_import_semantic_snapshot();
        snapshot.shadowed_annotation_imports.clear();
        snapshot
            .shadowed_annotation_imports
            .extend(local_annotations.iter().cloned());
        for local_name in &local_annotations {
            snapshot.imported_annotations.remove(local_name);
        }

        let mut explicit_annotations: BTreeMap<String, BTreeSet<AnnotationIdentity>> =
            BTreeMap::new();
        let mut explicit_annotation_locals = BTreeSet::new();
        let mut explicit_namespaces: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut explicit_namespace_locals = BTreeSet::new();

        // Pure authorization happens before any imported handler can run.
        // Recording remains in the late graph registrar, when a blob exists.
        for item in &program.items {
            let Item::Import(import_stmt, _) = item else {
                continue;
            };
            match &import_stmt.items {
                ImportItems::Named(specs) => {
                    if !specs.iter().any(|spec| spec.is_annotation) {
                        continue;
                    }
                    if let Some(pset) = self.permission_set.as_ref() {
                        self.authorize_import_permissions(import_stmt, pset)?;
                    }
                    for spec in specs.iter().filter(|spec| spec.is_annotation) {
                        let local_name = spec.alias.clone().unwrap_or_else(|| spec.name.clone());
                        explicit_annotation_locals.insert(local_name.clone());
                        if !local_annotations.contains(&local_name) {
                            explicit_annotations
                                .entry(local_name)
                                .or_default()
                                .insert((import_stmt.from.clone(), spec.name.clone()));
                        }
                    }
                }
                ImportItems::Namespace { name, alias } => {
                    if let Some(pset) = self.permission_set.as_ref() {
                        self.authorize_import_permissions(import_stmt, pset)?;
                    }
                    let local_name = alias.clone().unwrap_or_else(|| name.clone());
                    let module_path = if import_stmt.from.is_empty() {
                        name.clone()
                    } else {
                        import_stmt.from.clone()
                    };
                    explicit_namespace_locals.insert(local_name.clone());
                    explicit_namespaces
                        .entry(local_name)
                        .or_default()
                        .insert(module_path);
                }
            }
        }

        let mut synthetic_annotations: BTreeMap<String, BTreeSet<AnnotationIdentity>> =
            BTreeMap::new();
        let mut synthetic_namespaces: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for resolved in &graph.node(module_id).resolved_imports {
            match resolved {
                ResolvedImport::Named {
                    canonical_path,
                    symbols,
                    ..
                } => {
                    if symbols.iter().any(|symbol| symbol.is_annotation) {
                        if let Some(pset) = self.permission_set.as_ref() {
                            for symbol in symbols {
                                self.authorize_import_symbol_permissions(
                                    canonical_path,
                                    &symbol.original_name,
                                    pset,
                                )?;
                            }
                        }
                    }
                    for symbol in symbols.iter().filter(|symbol| symbol.is_annotation) {
                        if local_annotations.contains(&symbol.local_name)
                            || explicit_annotation_locals.contains(&symbol.local_name)
                        {
                            continue;
                        }
                        synthetic_annotations
                            .entry(symbol.local_name.clone())
                            .or_default()
                            .insert((canonical_path.clone(), symbol.original_name.clone()));
                    }
                }
                ResolvedImport::Namespace {
                    local_name,
                    canonical_path,
                    ..
                } => {
                    if let Some(pset) = self.permission_set.as_ref() {
                        self.authorize_import_module_permissions(canonical_path, pset)?;
                    }
                    if explicit_namespace_locals.contains(local_name) {
                        continue;
                    }
                    synthetic_namespaces
                        .entry(local_name.clone())
                        .or_default()
                        .insert(canonical_path.clone());
                }
            }
        }

        Self::install_staged_annotation_candidates(
            &mut snapshot,
            &local_annotations,
            explicit_annotations,
            synthetic_annotations,
        )?;
        Self::install_staged_namespace_candidates(
            &mut snapshot,
            explicit_namespaces,
            synthetic_namespaces,
        )?;
        Ok(snapshot)
    }

    fn install_staged_annotation_candidates(
        snapshot: &mut AnnotationImportSemanticSnapshot,
        local_annotations: &BTreeSet<String>,
        explicit: BTreeMap<String, BTreeSet<AnnotationIdentity>>,
        synthetic: BTreeMap<String, BTreeSet<AnnotationIdentity>>,
    ) -> Result<()> {
        let explicit_locals = explicit.keys().cloned().collect::<BTreeSet<_>>();
        for local_name in explicit_locals.iter().chain(local_annotations) {
            snapshot.imported_annotations.remove(local_name);
        }

        let mut candidates: BTreeMap<String, BTreeSet<AnnotationIdentity>> = snapshot
            .imported_annotations
            .iter()
            .map(|(local_name, binding)| {
                (
                    local_name.clone(),
                    BTreeSet::from([(binding._module_path.clone(), binding.original_name.clone())]),
                )
            })
            .collect();
        for (local_name, identities) in explicit.into_iter().chain(synthetic) {
            candidates.entry(local_name).or_default().extend(identities);
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
                location: None,
            });
        }

        snapshot.imported_annotations.clear();
        for (local_name, mut identities) in candidates {
            let (module_path, original_name) = identities
                .pop_first()
                .expect("every staged annotation binding has one identity");
            snapshot
                .module_scope_sources
                .entry(module_path.clone())
                .or_insert_with(|| module_path.clone());
            snapshot.imported_annotations.insert(
                local_name,
                ImportedAnnotationSymbol {
                    original_name,
                    _module_path: module_path.clone(),
                    hidden_module_name: module_path,
                },
            );
        }
        Ok(())
    }

    fn install_staged_namespace_candidates(
        snapshot: &mut AnnotationImportSemanticSnapshot,
        explicit: BTreeMap<String, BTreeSet<String>>,
        synthetic: BTreeMap<String, BTreeSet<String>>,
    ) -> Result<()> {
        for local_name in explicit.keys() {
            snapshot.graph_namespace_map.remove(local_name);
            snapshot.module_scope_sources.remove(local_name);
        }

        let mut candidates: BTreeMap<String, BTreeSet<String>> = snapshot
            .graph_namespace_map
            .iter()
            .map(|(local_name, path)| (local_name.clone(), BTreeSet::from([path.clone()])))
            .collect();
        for (local_name, identities) in explicit.into_iter().chain(synthetic) {
            candidates.entry(local_name).or_default().extend(identities);
        }

        if let Some((local_name, identities)) = candidates
            .iter()
            .find(|(_, identities)| identities.len() > 1)
        {
            let rendered = identities.iter().cloned().collect::<Vec<_>>().join("`, `");
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Conflicting annotation namespace imports for '{local_name}': `{rendered}` bind the same qualified prefix"
                ),
                location: None,
            });
        }

        snapshot.graph_namespace_map.clear();
        for (local_name, mut identities) in candidates {
            let canonical_path = identities
                .pop_first()
                .expect("every staged annotation namespace has one identity");
            snapshot
                .graph_namespace_map
                .insert(local_name.clone(), canonical_path.clone());
            snapshot
                .module_scope_sources
                .insert(local_name, canonical_path);
        }
        Ok(())
    }

    /// Test-facing compatibility wrapper for staging plus publication.
    #[cfg(test)]
    pub(in crate::compiler) fn pre_register_root_graph_annotation_imports(
        &mut self,
        root_program: &Program,
        graph: &ModuleGraph,
    ) -> Result<()> {
        let staged =
            self.stage_graph_annotation_imports_for_module(root_program, graph.root_id(), graph)?;
        self.restore_annotation_import_semantics(&staged);
        Ok(())
    }
}
