//! Transactional early registration for imported annotation bindings.

use crate::compiler::{BytecodeCompiler, ImportedAnnotationSymbol};
use shape_ast::ast::{ExportItem, Item};
use shape_ast::error::{Result, ShapeError};
use std::collections::{BTreeMap, BTreeSet};

impl BytecodeCompiler {
    fn root_local_annotation_names(program: &shape_ast::ast::Program) -> BTreeSet<String> {
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
    fn stage_annotation_import_bindings<I>(
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

    fn commit_annotation_import_bindings(&mut self, staged: BTreeMap<String, (String, String)>) {
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

    /// Install the root graph's annotation bindings without emitting runtime
    /// namespace bytecode. Explicit source imports are identified from the
    /// original AST and own their local spellings; resolved rows not present
    /// in that AST are synthetic/prelude vacancy fills. Namespace imports add
    /// only qualified prefix resolution, never bare annotation aliases.
    pub(in crate::compiler) fn pre_register_root_graph_annotation_imports(
        &mut self,
        root_program: &shape_ast::ast::Program,
        graph: &crate::module_graph::ModuleGraph,
    ) -> Result<()> {
        use crate::module_graph::ResolvedImport;
        use shape_ast::ast::ImportItems;

        let local_annotations = Self::root_local_annotation_names(root_program);
        let mut explicit_annotation_locals = BTreeSet::new();
        let mut requested = Vec::new();
        let mut explicit_namespace_locals = BTreeSet::new();
        let mut namespace_candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        // Source AST is the provenance authority for explicit imports. This
        // pass authorizes the complete relevant import statements before any
        // imported handler can execute, without recording blob metadata early.
        for item in &root_program.items {
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
                            requested.push((
                                local_name,
                                spec.name.clone(),
                                import_stmt.from.clone(),
                            ));
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
                    namespace_candidates
                        .entry(local_name)
                        .or_default()
                        .insert(module_path);
                }
            }
        }

        let root = graph.node(graph.root_id());
        for resolved in &root.resolved_imports {
            match resolved {
                ResolvedImport::Named {
                    canonical_path,
                    symbols,
                    ..
                } => {
                    for symbol in symbols.iter().filter(|symbol| symbol.is_annotation) {
                        // Explicit source imports were staged above. Any other
                        // resolved row is a synthetic/prelude vacancy fill and
                        // cannot displace an explicit local spelling.
                        if local_annotations.contains(&symbol.local_name)
                            || explicit_annotation_locals.contains(&symbol.local_name)
                        {
                            continue;
                        }
                        requested.push((
                            symbol.local_name.clone(),
                            symbol.original_name.clone(),
                            canonical_path.clone(),
                        ));
                    }
                }
                ResolvedImport::Namespace {
                    local_name,
                    canonical_path,
                    ..
                } => {
                    if explicit_namespace_locals.contains(local_name) {
                        continue;
                    }
                    namespace_candidates
                        .entry(local_name.clone())
                        .or_default()
                        .insert(canonical_path.clone());
                }
            }
        }

        // Validate both semantic tables before committing either one.
        let staged_annotations = self.stage_annotation_import_bindings(requested)?;
        for (local_name, existing) in &self.graph_namespace_map {
            namespace_candidates
                .entry(local_name.clone())
                .or_default()
                .insert(existing.clone());
        }
        if let Some((local_name, identities)) = namespace_candidates
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

        self.commit_annotation_import_bindings(staged_annotations);
        for (local_name, mut identities) in namespace_candidates {
            let canonical_path = identities
                .pop_first()
                .expect("every staged annotation namespace has one identity");
            self.graph_namespace_map
                .entry(local_name.clone())
                .or_insert_with(|| canonical_path.clone());
            self.module_scope_sources
                .entry(local_name)
                .or_insert(canonical_path);
        }
        Ok(())
    }
}
