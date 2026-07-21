//! Exact annotation-handler and comptime-helper authority for prepass execution.

use super::BytecodeCompiler;
use shape_ast::ast::{
    Annotation, AnnotationHandler, AnnotationHandlerType, Expr, FunctionDef, FunctionParameter,
    TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError};
use std::collections::{HashMap, HashSet};

/// ADR-009 C3 #14 (slice 4): derive the def-param carrier — `(name,
/// declared type annotation)` pairs — from annotation config parameter
/// definitions. Legacy (untyped) definitions always yield `None`
/// annotations, so every downstream seam is classification-free and
/// byte-equivalent for legacy defs; TypedConfig definitions carry their
/// declared types into the comptime-handler param injection
/// (`execute_comptime_with_annotation_handler`).
///
/// The flat_map over `get_identifiers()` preserves the exact flattening the
/// former `Vec<String>` names carrier used (annotation config params are
/// always plain identifiers, so this is 1:1 in practice).
///
/// NOTE (serde): `CompiledAnnotation.param_defs` is `#[serde(skip)]`, like
/// the comptime handlers that consume this carrier — a serialization-crossed
/// carrier loses both together, so the injection seam never sees a
/// names-without-defs carrier.
pub(in crate::compiler) fn annotation_def_params(
    params: &[FunctionParameter],
) -> Vec<(String, Option<TypeAnnotation>)> {
    params
        .iter()
        .flat_map(|param| {
            param
                .get_identifiers()
                .into_iter()
                .map(|name| (name, param.type_annotation.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Seed every direct function-call spelling before lexical authority resolves
/// it. Bare calls are intentionally retained: dropping them here would make a
/// defining-module helper look unavailable and tempt callers to reintroduce a
/// global fallback. Explicit qualified calls retain their exact key.
pub(super) fn seed_function_call(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::FunctionCall { name, .. } => {
            names.insert(name.clone());
        }
        Expr::QualifiedFunctionCall {
            namespace,
            function,
            ..
        } => {
            names.insert(format!("{namespace}::{function}"));
        }
        _ => {}
    }
}

/// One annotation's comptime handlers, keyed by its exact semantic name.
#[derive(Clone, Debug)]
pub(super) struct ComptimeAnnotationHandlers {
    /// User comptime handlers in declaration order, with a TypedConfig
    /// definition's SYNTHESIZED sugar post handler (S4c) appended AFTER them
    /// — coexistence is allowed and ordered: user handlers first.
    pub(super) handlers: Vec<AnnotationHandler>,
    /// ADR-009 C3 #14 (slice 4): `(name, declared type annotation)` pairs
    /// (see [`annotation_def_params`]); legacy defs carry `None` throughout.
    pub(super) def_params: Vec<(String, Option<TypeAnnotation>)>,
    /// ADR-009 C3 #14 (slice 4, S4c): the sugar lowering's MINTED hook body
    /// fns for this entry. Threaded into `template_body_fn_lookup` at every
    /// executor call site; entry-minted defs resolve FIRST (their hygienic
    /// names are unspellable, so no user fn can collide). Empty for legacy
    /// definitions and TypedConfig definitions without declarative hooks.
    pub(super) sugar_body_fns: Vec<FunctionDef>,
    /// Canonical module that owns the handler body.
    pub(super) defining_module_path: Option<String>,
    pub(super) provenance: ComptimeAnnotationHandlerProvenance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComptimeAnnotationHandlerProvenance {
    /// Current-unit declaration observed before pass 2 has compiled its carrier.
    LocalAst,
    /// Exact carrier from `BytecodeProgram::compiled_annotations`.
    Compiled,
}

/// Authority for resolving helper function references in one handler body.
///
/// Root-local access is explicit. A module-owned handler never receives the
/// global helper catalog: a missing `module::helper` therefore stays missing
/// instead of binding to a same-spelled root helper.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum ComptimeHandlerHelperAuthority {
    RootLocalAst,
    RootCompiled,
    DefiningModule(String),
    Unavailable,
}

impl ComptimeAnnotationHandlers {
    pub(super) fn helper_authority(&self) -> ComptimeHandlerHelperAuthority {
        match (&self.provenance, self.defining_module_path.as_deref()) {
            (_, Some(module)) => ComptimeHandlerHelperAuthority::DefiningModule(module.to_string()),
            (ComptimeAnnotationHandlerProvenance::LocalAst, None) => {
                ComptimeHandlerHelperAuthority::RootLocalAst
            }
            (ComptimeAnnotationHandlerProvenance::Compiled, None) => {
                ComptimeHandlerHelperAuthority::RootCompiled
            }
        }
    }
}

impl ComptimeHandlerHelperAuthority {
    pub(super) fn for_compiled_name(exact_name: Option<&str>) -> Self {
        match exact_name {
            Some(name) => match name.rsplit_once("::") {
                Some((module, _)) => Self::DefiningModule(module.to_string()),
                None => Self::RootCompiled,
            },
            None => Self::Unavailable,
        }
    }

    fn key(&self) -> Option<&str> {
        match self {
            Self::RootLocalAst => Some("root-local-ast"),
            Self::RootCompiled => Some("root-compiled"),
            Self::DefiningModule(module) => Some(module),
            Self::Unavailable => None,
        }
    }
}

impl BytecodeCompiler {
    /// Gather local pre-pass rows and exact compiled carriers without walking
    /// dependency ASTs by spelling.
    pub(super) fn collect_comptime_annotation_handlers(
        &self,
        program: &shape_ast::ast::Program,
    ) -> Result<HashMap<String, ComptimeAnnotationHandlers>> {
        use shape_ast::ast::{ExportItem, Item};

        fn defining_module_path(exact_name: &str) -> Option<String> {
            exact_name
                .rsplit_once("::")
                .map(|(module_path, _)| module_path.to_string())
        }

        fn local_exact_name(name: &str, module_path: Option<&str>) -> String {
            if name.contains("::") {
                name.to_string()
            } else if let Some(module_path) = module_path {
                BytecodeCompiler::qualify_module_symbol(module_path, name)
            } else {
                name.to_string()
            }
        }

        fn ingest_local(
            compiler: &BytecodeCompiler,
            map: &mut HashMap<String, ComptimeAnnotationHandlers>,
            ann_def: &shape_ast::ast::AnnotationDef,
            module_path: Option<&str>,
        ) {
            let mut handlers: Vec<_> = ann_def
                .handlers
                .iter()
                .filter(|handler| {
                    matches!(
                        handler.handler_type,
                        AnnotationHandlerType::ComptimePre | AnnotationHandlerType::ComptimePost
                    )
                })
                .cloned()
                .collect();
            // ADR-009 C3 #14 (slice 4, S4c): the LocalAst provenance derives
            // the sugar lowering from the local AST definition (same ONE
            // producer the planner/installer use) and appends the synthesized
            // handler AFTER any user comptime handlers. R2/R3 rejections are
            // NOT fired here — `sugar_lowering_for_def` defers them to the
            // declaration-site planner (one attribution site); a rejecting
            // definition simply contributes no sugar to the pre-pass.
            let sugar = crate::compiler::statements::annotation_declarations::sugar_lowering::
                sugar_lowering_for_def(compiler, ann_def);
            let (sugar_post_handler, sugar_body_fns) = match sugar {
                Some(sugar) => (Some(sugar.post_handler), sugar.body_fns),
                None => (None, Vec::new()),
            };
            handlers.extend(sugar_post_handler);
            if handlers.is_empty() {
                return;
            }
            let exact_name = local_exact_name(&ann_def.name, module_path);
            let def_params = annotation_def_params(&ann_def.params);
            map.entry(exact_name.clone())
                .or_insert(ComptimeAnnotationHandlers {
                    handlers,
                    def_params,
                    sugar_body_fns,
                    defining_module_path: defining_module_path(&exact_name),
                    provenance: ComptimeAnnotationHandlerProvenance::LocalAst,
                });
        }

        fn ingest_local_items(
            compiler: &BytecodeCompiler,
            map: &mut HashMap<String, ComptimeAnnotationHandlers>,
            items: &[Item],
            parent_module_path: Option<&str>,
        ) {
            for item in items {
                match item {
                    Item::AnnotationDef(ann_def, _) => {
                        ingest_local(compiler, map, ann_def, parent_module_path)
                    }
                    Item::Export(export, _) => {
                        if let ExportItem::Annotation(ann_def) = &export.item {
                            ingest_local(compiler, map, ann_def, parent_module_path);
                        }
                    }
                    Item::Module(module, _) => {
                        let module_path = match parent_module_path {
                            Some(parent) => {
                                BytecodeCompiler::qualify_module_symbol(parent, &module.name)
                            }
                            None => module.name.clone(),
                        };
                        ingest_local_items(compiler, map, &module.items, Some(&module_path));
                    }
                    _ => {}
                }
            }
        }

        let mut map = HashMap::new();
        ingest_local_items(
            self,
            &mut map,
            &program.items,
            self.module_scope_stack.last().map(String::as_str),
        );

        let mut compiled_names: Vec<_> = self.program.compiled_annotations.keys().collect();
        compiled_names.sort();
        for key in compiled_names {
            let compiled = &self.program.compiled_annotations[key];
            if key != &compiled.name {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "Internal error: compiled annotation registry key '{}' does not match carrier name '{}'",
                        key, compiled.name
                    ),
                    location: None,
                });
            }

            // ADR-009 C3 #14 (slice 4, S4c): the Compiled provenance reads
            // the sugar lowering's STORED artifacts (installer-attached) and
            // appends the synthesized handler AFTER the user comptime
            // handlers — same order as the LocalAst provenance.
            let handlers: Vec<_> = [
                compiled.comptime_pre_handler.clone(),
                compiled.comptime_post_handler.clone(),
                compiled.sugar_post_handler.clone(),
            ]
            .into_iter()
            .flatten()
            .collect();
            if handlers.is_empty() {
                continue;
            }
            map.entry(key.clone())
                .or_insert(ComptimeAnnotationHandlers {
                    handlers,
                    // Compiled provenance reads the FULL param definitions
                    // (`param_defs`), never the flattened `param_names` — the
                    // declared type annotations ride along (slice 4).
                    def_params: annotation_def_params(&compiled.param_defs),
                    sugar_body_fns: compiled.sugar_body_fns.clone(),
                    defining_module_path: defining_module_path(key),
                    provenance: ComptimeAnnotationHandlerProvenance::Compiled,
                });
        }

        Ok(map)
    }

    /// Resolve an applied annotation without suffix or all-module search.
    pub(super) fn resolve_comptime_annotation_handlers<'a>(
        &self,
        handler_map: &'a HashMap<String, ComptimeAnnotationHandlers>,
        annotation: &Annotation,
        lexical_module_path: Option<&str>,
    ) -> Option<(&'a str, &'a ComptimeAnnotationHandlers)> {
        let exact = |name: &str| {
            handler_map
                .get_key_value(name)
                .map(|(key, row)| (key.as_str(), row))
        };
        let local = |name: &str| {
            exact(name)
                .filter(|(_, row)| row.provenance == ComptimeAnnotationHandlerProvenance::LocalAst)
        };

        if annotation.name.contains("::") {
            return self
                .resolve_compiled_annotation_name(annotation)
                .and_then(|name| exact(&name))
                .or_else(|| local(&annotation.name));
        }

        let local_name = match lexical_module_path {
            Some(module_path) => Self::qualify_module_symbol(module_path, &annotation.name),
            None => annotation.name.clone(),
        };
        if let Some(found) = local(&local_name) {
            return Some(found);
        }

        self.resolve_compiled_annotation_name(annotation)
            .and_then(|name| exact(&name))
    }

    /// Build the flat mini-VM helper list from explicit lexical authority.
    pub(super) fn collect_authorized_comptime_helpers(
        &self,
        expr: &Expr,
        authority: ComptimeHandlerHelperAuthority,
    ) -> Vec<FunctionDef> {
        let mut seed_names = HashSet::new();
        Self::collect_scoped_names_in_expr(expr, &mut seed_names);
        let mut pending: Vec<_> = seed_names
            .into_iter()
            .map(|name| (name, authority.clone()))
            .collect();
        let mut visited = HashSet::new();
        let mut helpers = Vec::new();

        while let Some((name, lexical_authority)) = pending.pop() {
            let Some(authority_key) = lexical_authority.key() else {
                continue;
            };
            if !visited.insert((name.clone(), authority_key.to_string())) {
                continue;
            }

            let definition = if name.contains("::") {
                self.function_defs.get(&name).cloned()
            } else {
                match &lexical_authority {
                    ComptimeHandlerHelperAuthority::RootLocalAst
                    | ComptimeHandlerHelperAuthority::RootCompiled => {
                        self.function_defs.get(&name).cloned()
                    }
                    ComptimeHandlerHelperAuthority::DefiningModule(module) => {
                        let qualified = Self::qualify_module_symbol(module, &name);
                        self.function_defs.get(&qualified).cloned()
                    }
                    ComptimeHandlerHelperAuthority::Unavailable => None,
                }
            };
            let Some(definition) = definition else {
                continue;
            };

            let nested_authority =
                ComptimeHandlerHelperAuthority::for_compiled_name(Some(definition.name.as_str()));
            let mut exposed = definition.clone();
            if !name.contains("::") && exposed.name != name {
                exposed.name = name.clone();
            }
            helpers.push(exposed);

            for statement in &definition.body {
                let mut nested = HashSet::new();
                Self::collect_scoped_names_in_statement(statement, &mut nested);
                pending.extend(
                    nested
                        .into_iter()
                        .map(|name| (name, nested_authority.clone())),
                );
            }
        }

        helpers.sort_by(|left, right| left.name.cmp(&right.name));
        helpers.dedup_by(|left, right| left.name == right.name);
        helpers
    }

    /// Keep inline-module lexical scope balanced across success and error.
    pub(super) fn with_comptime_annotation_module_scope<T>(
        &mut self,
        module_path: String,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.module_scope_stack.push(module_path);
        let result = operation(self);
        self.module_scope_stack.pop();
        result
    }
}
