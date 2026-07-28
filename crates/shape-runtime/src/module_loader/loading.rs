//! Module loading and compilation
//!
//! Handles parsing module files, compiling AST, and processing exports.

use shape_ast::ast::{
    AnnotationDef, BuiltinFunctionDecl, ExportItem, ExportStmt, Expr, FunctionDef, Item, Literal,
    Program, Span, UnaryOp, VarKind, VariableDecl,
};
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;
use std::collections::HashMap;
use std::sync::Arc;

use super::{Export, Module};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeSymbolKind {
    Function,
    BuiltinFunction,
    TypeAlias,
    BuiltinType,
    Trait,
    Enum,
    Annotation,
    Value,
    /// R8 W8 Cluster A (2026-05-24): module-level `const` bindings. Tracked
    /// distinctly from `Value` so the Named-export resolver can produce a
    /// `NamedExportResolution::Const(...)` instead of the residual
    /// `Variable` rejection arm.
    Const,
}

/// Module scope tracking for export resolution
#[derive(Debug)]
pub(super) struct ModuleScope {
    functions: HashMap<String, FunctionDef>,
    builtin_functions: HashMap<String, BuiltinFunctionDecl>,
    annotations: HashMap<String, AnnotationDef>,
    type_aliases: HashMap<String, shape_ast::ast::TypeAliasDef>,
    /// R8 W8 Cluster A: module-level `const NAME = expr` bindings, plus
    /// `pub const NAME = expr` (whose source_decl is unwrapped here).
    /// The full `VariableDecl` is retained so the export-resolution step
    /// can run the comptime evaluator over the initializer.
    consts: HashMap<String, VariableDecl>,
    symbols: HashMap<String, (ScopeSymbolKind, Span)>,
}

impl ModuleScope {
    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            builtin_functions: HashMap::new(),
            annotations: HashMap::new(),
            type_aliases: HashMap::new(),
            consts: HashMap::new(),
            symbols: HashMap::new(),
        }
    }

    fn add_function(&mut self, name: String, function: FunctionDef, span: Span) {
        self.symbols
            .insert(name.clone(), (ScopeSymbolKind::Function, span));
        self.functions.insert(name, function);
    }

    fn add_builtin_function(&mut self, name: String, function: BuiltinFunctionDecl, span: Span) {
        self.symbols
            .insert(name.clone(), (ScopeSymbolKind::BuiltinFunction, span));
        self.builtin_functions.insert(name, function);
    }

    fn add_type_alias(
        &mut self,
        name: String,
        alias: shape_ast::ast::TypeAliasDef,
        kind: ScopeSymbolKind,
        span: Span,
    ) {
        self.symbols.insert(name.clone(), (kind, span));
        self.type_aliases.insert(name, alias);
    }

    fn add_variable(&mut self, name: String, span: Span) {
        self.symbols.insert(name, (ScopeSymbolKind::Value, span));
    }

    fn add_const(&mut self, name: String, decl: VariableDecl, span: Span) {
        self.symbols
            .insert(name.clone(), (ScopeSymbolKind::Const, span));
        self.consts.insert(name, decl);
    }

    fn get_const(&self, name: &str) -> Option<&VariableDecl> {
        self.consts.get(name)
    }

    fn add_annotation(&mut self, name: String, annotation: AnnotationDef, span: Span) {
        self.symbols
            .insert(name.clone(), (ScopeSymbolKind::Annotation, span));
        self.annotations.insert(name, annotation);
    }

    fn get_function(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.get(name)
    }

    fn get_builtin_function(&self, name: &str) -> Option<&BuiltinFunctionDecl> {
        self.builtin_functions.get(name)
    }

    fn get_type_alias(&self, name: &str) -> Option<&shape_ast::ast::TypeAliasDef> {
        self.type_aliases.get(name)
    }

    fn get_annotation(&self, name: &str) -> Option<&AnnotationDef> {
        self.annotations.get(name)
    }

    fn resolve_kind_and_span(&self, name: &str) -> Option<(ScopeSymbolKind, Span)> {
        self.symbols.get(name).copied()
    }
}

fn alias_for_named_type(
    name: String,
    type_params: Option<Vec<shape_ast::ast::TypeParam>>,
) -> shape_ast::ast::TypeAliasDef {
    shape_ast::ast::TypeAliasDef {
        name: name.clone(),
        doc_comment: None,
        type_params,
        type_annotation: shape_ast::ast::TypeAnnotation::Basic(name),
        meta_param_overrides: None,
    }
}

fn function_stub_for_builtin(function: &BuiltinFunctionDecl) -> FunctionDef {
    FunctionDef {
        name: function.name.clone(),
        name_span: function.name_span,
        declaring_module_path: None,
        doc_comment: function.doc_comment.clone(),
        type_params: function.type_params.clone(),
        params: function.params.clone(),
        return_type: Some(function.return_type.clone()),
        where_clause: None,
        body: vec![],
        annotations: vec![],
        is_async: false,
        is_comptime: false,
        effect_row: None,
    }
}

fn collect_module_scope(ast: &Program) -> ModuleScope {
    let mut module_scope = ModuleScope::new();

    // First pass: collect all top-level declarations
    for item in &ast.items {
        match item {
            Item::Function(function, span) => {
                module_scope.add_function(function.name.clone(), function.clone(), *span);
            }
            Item::BuiltinFunctionDecl(function, span) => {
                module_scope.add_builtin_function(function.name.clone(), function.clone(), *span);
            }
            Item::BuiltinTypeDecl(type_decl, span) => {
                let alias =
                    alias_for_named_type(type_decl.name.clone(), type_decl.type_params.clone());
                module_scope.add_type_alias(
                    type_decl.name.clone(),
                    alias,
                    ScopeSymbolKind::BuiltinType,
                    *span,
                );
            }
            Item::TypeAlias(alias, span) => {
                module_scope.add_type_alias(
                    alias.name.clone(),
                    alias.clone(),
                    ScopeSymbolKind::TypeAlias,
                    *span,
                );
            }
            Item::Enum(enum_def, span) => {
                let alias =
                    alias_for_named_type(enum_def.name.clone(), enum_def.type_params.clone());
                module_scope.add_type_alias(
                    enum_def.name.clone(),
                    alias,
                    ScopeSymbolKind::Enum,
                    *span,
                );
            }
            Item::StructType(struct_def, span) => {
                let alias =
                    alias_for_named_type(struct_def.name.clone(), struct_def.type_params.clone());
                module_scope.add_type_alias(
                    struct_def.name.clone(),
                    alias,
                    ScopeSymbolKind::TypeAlias,
                    *span,
                );
            }
            Item::Trait(trait_def, span) => {
                let alias =
                    alias_for_named_type(trait_def.name.clone(), trait_def.type_params.clone());
                module_scope.add_type_alias(
                    trait_def.name.clone(),
                    alias,
                    ScopeSymbolKind::Trait,
                    *span,
                );
            }
            Item::VariableDecl(var_decl, span) => {
                // Variables are added to scope but need runtime evaluation for their values.
                // R8 W8 Cluster A: `const` decls are tracked separately so the
                // Named-export resolver can comptime-evaluate the initializer.
                if let Some(name) = var_decl.pattern.as_identifier() {
                    if var_decl.kind == VarKind::Const {
                        module_scope.add_const(name.to_string(), var_decl.clone(), *span);
                    } else {
                        module_scope.add_variable(name.to_string(), *span);
                    }
                }
            }
            Item::Export(export, span) => {
                // `pub const NAME = ...` parses as ExportItem::Named([NAME]) with
                // a `source_decl: Some(VariableDecl)` carrying the const decl.
                // Pick those up here so resolve_named_export can find them.
                if let Some(decl) = export.source_decl.as_ref() {
                    if let Some(name) = decl.pattern.as_identifier() {
                        if decl.kind == VarKind::Const {
                            module_scope.add_const(name.to_string(), decl.clone(), *span);
                        } else {
                            module_scope.add_variable(name.to_string(), *span);
                        }
                    }
                }
            }
            Item::AnnotationDef(annotation, span) => {
                module_scope.add_annotation(annotation.name.clone(), annotation.clone(), *span);
            }
            _ => {}
        }
    }

    module_scope
}

/// R8 W8 Cluster A (2026-05-24): comptime-evaluate a module-level `const`
/// initializer to a [`KindedSlot`].
///
/// Bounded to expressions whose value is statically known without invoking
/// the VM:
///   - `Literal::Int` / `Literal::Number` / `Literal::Bool`
///   - `Literal::String` (interned via `KindedSlot::from_string`)
///   - `UnaryOp::Neg` applied to a comptime-evaluable numeric initializer
///   - `UnaryOp::Not` applied to a comptime-evaluable bool initializer
///
/// Any other expression shape (function call, identifier, binary op, etc.)
/// surfaces a clean compile error per the dispatch's reject-runtime-init
/// requirement. ADR-006 §2.7.5 stamp-at-compile-time invariant: the kind is
/// stamped here from the literal's shape, never fabricated from raw bits.
fn comptime_eval_const_initializer(
    name: &str,
    expr: &Expr,
) -> std::result::Result<KindedSlot, String> {
    match expr {
        Expr::Literal(lit, _) => match lit {
            Literal::Int(i) => Ok(KindedSlot::from_int(*i)),
            Literal::Number(n) => Ok(KindedSlot::from_number(*n)),
            Literal::Bool(b) => Ok(KindedSlot::from_bool(*b)),
            Literal::String(s) => Ok(KindedSlot::from_string(s)),
            other => Err(format!(
                "const '{}' initializer literal kind `{:?}` is not comptime-evaluable in this dispatch (R8 W8 Cluster A). \
                 Supported literal kinds: Int, Number, Bool, String. \
                 Extending the comptime evaluator is v0.4-concurrency-design-pass territory per docs/v0.3-close-summary.md \u{a7}5.15.",
                name, other
            )),
        },
        Expr::UnaryOp { op, operand, .. } => {
            let inner = comptime_eval_const_initializer(name, operand)?;
            match op {
                UnaryOp::Neg => {
                    if let Some(n) = inner.as_f64() {
                        Ok(KindedSlot::from_number(-n))
                    } else if let Some(i) = inner.as_i64() {
                        Ok(KindedSlot::from_int(-i))
                    } else {
                        Err(format!(
                            "const '{}' unary `-` initializer's operand is not a numeric literal",
                            name
                        ))
                    }
                }
                UnaryOp::Not => {
                    if let Some(b) = inner.as_bool() {
                        Ok(KindedSlot::from_bool(!b))
                    } else {
                        Err(format!(
                            "const '{}' unary `!` initializer's operand is not a bool literal",
                            name
                        ))
                    }
                }
                UnaryOp::BitNot => Err(format!(
                    "const '{}' unary `~` (BitNot) is not supported by the R8 W8 Cluster A comptime evaluator",
                    name
                )),
            }
        }
        _ => Err(format!(
            "const '{}' initializer is not comptime-evaluable in this dispatch (R8 W8 Cluster A). \
             Supported initializer shapes: literal (int / number / bool / string) and unary `-`/`!` on a literal. \
             Function calls, identifiers, binary ops, and other runtime-dependent expressions are rejected per the \
             v0.3 const-only bounded feature; extending the evaluator is v0.4-concurrency-design-pass territory \
             per docs/v0.3-close-summary.md \u{a7}5.15.",
            name
        )),
    }
}

enum NamedExportResolution<'a> {
    Function(&'a FunctionDef),
    BuiltinFunction(&'a BuiltinFunctionDecl),
    TypeAlias(&'a shape_ast::ast::TypeAliasDef),
    Annotation(&'a AnnotationDef),
    /// R8 W8 Cluster A: module-level `const`, carrying the decl for
    /// comptime-evaluating the initializer at export-build time.
    Const(&'a VariableDecl),
    Variable,
    Missing,
}

fn resolve_named_export<'a>(scope: &'a ModuleScope, name: &str) -> NamedExportResolution<'a> {
    if let Some(function) = scope.get_function(name) {
        NamedExportResolution::Function(function)
    } else if let Some(function) = scope.get_builtin_function(name) {
        NamedExportResolution::BuiltinFunction(function)
    } else if let Some(alias) = scope.get_type_alias(name) {
        NamedExportResolution::TypeAlias(alias)
    } else if let Some(annotation) = scope.get_annotation(name) {
        NamedExportResolution::Annotation(annotation)
    } else if let Some(decl) = scope.get_const(name) {
        NamedExportResolution::Const(decl)
    } else if matches!(
        scope.resolve_kind_and_span(name),
        Some((ScopeSymbolKind::Value, _))
    ) {
        NamedExportResolution::Variable
    } else {
        NamedExportResolution::Missing
    }
}

/// Compile a parsed module
pub(super) fn compile_module(module_path: &str, ast: Program) -> Result<Module> {
    let mut exports = HashMap::new();
    let module_name = module_path.to_string();
    let module_scope = collect_module_scope(&ast);

    // Second pass: process exports
    for item in &ast.items {
        if let Item::Export(export, _) = item {
            process_export_with_scope(export, &mut exports, &module_scope)?;
        }
    }

    Ok(Module {
        name: module_name,
        path: module_path.to_string(),
        exports,
        ast,
        artifact_origin: super::ModuleArtifactOrigin::Direct,
    })
}

/// Process an export statement with module scope
pub(super) fn process_export_with_scope(
    export: &ExportStmt,
    exports: &mut HashMap<String, Export>,
    scope: &ModuleScope,
) -> Result<()> {
    match &export.item {
        ExportItem::Function(function) => {
            exports.insert(
                function.name.clone(),
                Export::Function(Arc::new(function.clone())),
            );
        }
        ExportItem::BuiltinFunction(function) => {
            exports.insert(
                function.name.clone(),
                Export::Function(Arc::new(function_stub_for_builtin(function))),
            );
        }
        ExportItem::BuiltinType(type_decl) => {
            let alias = alias_for_named_type(type_decl.name.clone(), type_decl.type_params.clone());
            exports.insert(type_decl.name.clone(), Export::TypeAlias(Arc::new(alias)));
        }

        ExportItem::TypeAlias(alias) => {
            exports.insert(
                alias.name.clone(),
                Export::TypeAlias(Arc::new(alias.clone())),
            );
        }

        ExportItem::Named(specs) => {
            // Look up named exports in module scope
            for spec in specs {
                let export_name = spec.alias.as_ref().unwrap_or(&spec.name);

                match resolve_named_export(scope, &spec.name) {
                    NamedExportResolution::Function(function) => {
                        exports.insert(
                            export_name.clone(),
                            Export::Function(Arc::new(function.clone())),
                        );
                    }
                    NamedExportResolution::BuiltinFunction(function) => {
                        exports.insert(
                            export_name.clone(),
                            Export::Function(Arc::new(function_stub_for_builtin(function))),
                        );
                    }
                    NamedExportResolution::TypeAlias(alias) => {
                        exports.insert(
                            export_name.clone(),
                            Export::TypeAlias(Arc::new(alias.clone())),
                        );
                    }
                    NamedExportResolution::Annotation(annotation) => {
                        exports.insert(
                            export_name.clone(),
                            Export::Annotation(Arc::new(annotation.clone())),
                        );
                    }
                    NamedExportResolution::Const(decl) => {
                        // R8 W8 Cluster A: comptime-evaluate the const
                        // initializer to a KindedSlot and export it as
                        // Export::Value. Reject runtime-only initializers
                        // with a clean error (no panic).
                        let initializer = decl.value.as_ref().ok_or_else(|| {
                            ShapeError::ModuleError {
                                message: format!(
                                    "Cannot export const '{}': initializer is missing (R8 W8 Cluster A)",
                                    spec.name
                                ),
                                module_path: None,
                            }
                        })?;
                        let slot = comptime_eval_const_initializer(&spec.name, initializer)
                            .map_err(|message| ShapeError::ModuleError {
                                message,
                                module_path: None,
                            })?;
                        exports.insert(export_name.clone(), Export::Value(slot));
                    }
                    NamedExportResolution::Variable => {
                        // `let`/`var` at module scope remain non-exportable;
                        // module-level mutable state is v0.4-concurrency-
                        // design-pass territory per docs/v0.3-close-summary.md
                        // §5.15. Only `const` bindings (plus functions and
                        // types) can be exported in v0.3 (R8 W8 Cluster A).
                        return Err(ShapeError::ModuleError {
                            message: format!(
                                "Cannot export variable '{}': variable exports are not yet supported. \
                             Only `const` bindings (plus functions and types) can be exported.",
                                spec.name
                            ),
                            module_path: None,
                        });
                    }
                    NamedExportResolution::Missing => {
                        return Err(ShapeError::ModuleError {
                            message: format!(
                                "Cannot export '{}': not found in module scope",
                                spec.name
                            ),
                            module_path: None,
                        });
                    }
                }
            }
        }

        ExportItem::Enum(enum_def) => {
            let alias = shape_ast::ast::TypeAliasDef {
                name: enum_def.name.clone(),
                doc_comment: None,
                type_params: enum_def.type_params.clone(),
                type_annotation: shape_ast::ast::TypeAnnotation::Basic(enum_def.name.clone()),
                meta_param_overrides: None,
            };
            exports.insert(enum_def.name.clone(), Export::TypeAlias(Arc::new(alias)));
        }
        ExportItem::Struct(struct_def) => {
            let alias = shape_ast::ast::TypeAliasDef {
                name: struct_def.name.clone(),
                doc_comment: None,
                type_params: struct_def.type_params.clone(),
                type_annotation: shape_ast::ast::TypeAnnotation::Basic(struct_def.name.clone()),
                meta_param_overrides: None,
            };
            exports.insert(struct_def.name.clone(), Export::TypeAlias(Arc::new(alias)));
        }
        ExportItem::Trait(trait_def) => {
            let alias = shape_ast::ast::TypeAliasDef {
                name: trait_def.name.clone(),
                doc_comment: None,
                type_params: trait_def.type_params.clone(),
                type_annotation: shape_ast::ast::TypeAnnotation::Basic(trait_def.name.clone()),
                meta_param_overrides: None,
            };
            exports.insert(trait_def.name.clone(), Export::TypeAlias(Arc::new(alias)));
        }
        ExportItem::Annotation(annotation) => {
            exports.insert(
                annotation.name.clone(),
                Export::Annotation(Arc::new(annotation.clone())),
            );
        }
        ExportItem::ForeignFunction(function) => {
            exports.insert(
                function.name.clone(),
                Export::Function(Arc::new(shape_ast::ast::FunctionDef {
                    name: function.name.clone(),
                    name_span: function.name_span,
                    declaring_module_path: None,
                    doc_comment: function.doc_comment.clone(),
                    type_params: function.type_params.clone(),
                    params: function.params.clone(),
                    return_type: function.return_type.clone(),
                    where_clause: None,
                    body: vec![],
                    annotations: function.annotations.clone(),
                    is_async: function.is_async,
                    is_comptime: false,
                    effect_row: None,
                })),
            );
        }
    }

    Ok(())
}
