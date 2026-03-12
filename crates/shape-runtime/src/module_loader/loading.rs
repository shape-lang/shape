//! Module loading and compilation
//!
//! Handles parsing module files, compiling AST, and processing exports.

use shape_ast::ast::{
    AnnotationDef, BuiltinFunctionDecl, ExportItem, ExportStmt, FunctionDef, Item, Program, Span,
};
use shape_ast::error::{Result, ShapeError};
use std::collections::HashMap;
use std::sync::Arc;

use super::{Export, Module, ModuleExportKind, ModuleExportSymbol};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeSymbolKind {
    Function,
    BuiltinFunction,
    TypeAlias,
    BuiltinType,
    Interface,
    Enum,
    Annotation,
    Value,
}

/// Module scope tracking for export resolution
#[derive(Debug)]
pub(super) struct ModuleScope {
    functions: HashMap<String, FunctionDef>,
    builtin_functions: HashMap<String, BuiltinFunctionDecl>,
    annotations: HashMap<String, AnnotationDef>,
    type_aliases: HashMap<String, shape_ast::ast::TypeAliasDef>,
    symbols: HashMap<String, (ScopeSymbolKind, Span)>,
}

impl ModuleScope {
    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            builtin_functions: HashMap::new(),
            annotations: HashMap::new(),
            type_aliases: HashMap::new(),
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
            Item::Interface(interface, span) => {
                let alias =
                    alias_for_named_type(interface.name.clone(), interface.type_params.clone());
                module_scope.add_type_alias(
                    interface.name.clone(),
                    alias,
                    ScopeSymbolKind::Interface,
                    *span,
                );
            }
            Item::Trait(trait_def, span) => {
                let alias =
                    alias_for_named_type(trait_def.name.clone(), trait_def.type_params.clone());
                module_scope.add_type_alias(
                    trait_def.name.clone(),
                    alias,
                    ScopeSymbolKind::Interface,
                    *span,
                );
            }
            Item::VariableDecl(var_decl, span) => {
                // Variables are added to scope but need runtime evaluation for their values
                if let Some(name) = var_decl.pattern.as_identifier() {
                    module_scope.add_variable(name.to_string(), *span);
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

enum NamedExportResolution<'a> {
    Function(&'a FunctionDef),
    BuiltinFunction(&'a BuiltinFunctionDecl),
    TypeAlias(&'a shape_ast::ast::TypeAliasDef),
    Annotation(&'a AnnotationDef),
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
    } else if matches!(
        scope.resolve_kind_and_span(name),
        Some((ScopeSymbolKind::Value, _))
    ) {
        NamedExportResolution::Variable
    } else {
        NamedExportResolution::Missing
    }
}

fn scope_symbol_kind_to_module(kind: ScopeSymbolKind) -> ModuleExportKind {
    match kind {
        ScopeSymbolKind::Function => ModuleExportKind::Function,
        ScopeSymbolKind::BuiltinFunction => ModuleExportKind::BuiltinFunction,
        ScopeSymbolKind::TypeAlias => ModuleExportKind::TypeAlias,
        ScopeSymbolKind::BuiltinType => ModuleExportKind::BuiltinType,
        ScopeSymbolKind::Interface => ModuleExportKind::Interface,
        ScopeSymbolKind::Enum => ModuleExportKind::Enum,
        ScopeSymbolKind::Annotation => ModuleExportKind::Annotation,
        ScopeSymbolKind::Value => ModuleExportKind::Value,
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
                    NamedExportResolution::Variable => {
                        // Variable exports are not yet supported. Variables require
                        // runtime evaluation which the module loader cannot perform
                        // at load time. Only functions and types can be exported.
                        return Err(ShapeError::ModuleError {
                            message: format!(
                                "Cannot export variable '{}': variable exports are not yet supported. \
                             Only functions and types can be exported.",
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
        ExportItem::Interface(iface_def) => {
            let alias = shape_ast::ast::TypeAliasDef {
                name: iface_def.name.clone(),
                doc_comment: None,
                type_params: iface_def.type_params.clone(),
                type_annotation: shape_ast::ast::TypeAnnotation::Basic(iface_def.name.clone()),
                meta_param_overrides: None,
            };
            exports.insert(iface_def.name.clone(), Export::TypeAlias(Arc::new(alias)));
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
                })),
            );
        }
    }

    Ok(())
}

/// Collect exported symbol metadata from a parsed module AST.
pub(super) fn collect_exported_symbols(ast: &Program) -> Result<Vec<ModuleExportSymbol>> {
    let module_scope = collect_module_scope(ast);
    let mut symbols = Vec::new();

    for item in &ast.items {
        let Item::Export(export, _) = item else {
            continue;
        };

        match &export.item {
            ExportItem::Function(function) => {
                symbols.push(ModuleExportSymbol {
                    name: function.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::Function,
                    span: function.name_span,
                });
            }
            ExportItem::BuiltinFunction(function) => {
                symbols.push(ModuleExportSymbol {
                    name: function.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::BuiltinFunction,
                    span: function.name_span,
                });
            }
            ExportItem::BuiltinType(type_decl) => {
                let span = module_scope
                    .resolve_kind_and_span(&type_decl.name)
                    .map(|(_, span)| span)
                    .unwrap_or_default();
                symbols.push(ModuleExportSymbol {
                    name: type_decl.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::BuiltinType,
                    span,
                });
            }
            ExportItem::TypeAlias(alias) => {
                let span = module_scope
                    .resolve_kind_and_span(&alias.name)
                    .map(|(_, span)| span)
                    .unwrap_or_default();
                symbols.push(ModuleExportSymbol {
                    name: alias.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::TypeAlias,
                    span,
                });
            }
            ExportItem::Enum(enum_def) => {
                let span = module_scope
                    .resolve_kind_and_span(&enum_def.name)
                    .map(|(_, span)| span)
                    .unwrap_or_default();
                symbols.push(ModuleExportSymbol {
                    name: enum_def.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::Enum,
                    span,
                });
            }
            ExportItem::Struct(struct_def) => {
                let span = module_scope
                    .resolve_kind_and_span(&struct_def.name)
                    .map(|(_, span)| span)
                    .unwrap_or_default();
                symbols.push(ModuleExportSymbol {
                    name: struct_def.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::TypeAlias,
                    span,
                });
            }
            ExportItem::Interface(interface_def) => {
                let span = module_scope
                    .resolve_kind_and_span(&interface_def.name)
                    .map(|(_, span)| span)
                    .unwrap_or_default();
                symbols.push(ModuleExportSymbol {
                    name: interface_def.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::Interface,
                    span,
                });
            }
            ExportItem::Trait(trait_def) => {
                let span = module_scope
                    .resolve_kind_and_span(&trait_def.name)
                    .map(|(_, span)| span)
                    .unwrap_or_default();
                symbols.push(ModuleExportSymbol {
                    name: trait_def.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::Interface,
                    span,
                });
            }
            ExportItem::Annotation(annotation) => {
                symbols.push(ModuleExportSymbol {
                    name: annotation.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::Annotation,
                    span: annotation.name_span,
                });
            }
            ExportItem::ForeignFunction(function) => {
                symbols.push(ModuleExportSymbol {
                    name: function.name.clone(),
                    alias: None,
                    kind: ModuleExportKind::Function,
                    span: function.name_span,
                });
            }
            ExportItem::Named(specs) => {
                for spec in specs {
                    let kind = match resolve_named_export(&module_scope, &spec.name) {
                        NamedExportResolution::Function(_)
                        | NamedExportResolution::BuiltinFunction(_)
                        | NamedExportResolution::TypeAlias(_) => module_scope
                            .resolve_kind_and_span(&spec.name)
                            .map(|(kind, _)| scope_symbol_kind_to_module(kind))
                            .unwrap_or(ModuleExportKind::TypeAlias),
                        NamedExportResolution::Annotation(_) => ModuleExportKind::Annotation,
                        NamedExportResolution::Variable => {
                            return Err(ShapeError::ModuleError {
                                message: format!(
                                    "Cannot export variable '{}': variable exports are not yet supported. \
                                     Only functions and types can be exported.",
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
                    };
                    let span = module_scope
                        .resolve_kind_and_span(&spec.name)
                        .map(|(_, span)| span)
                        .unwrap_or_default();
                    symbols.push(ModuleExportSymbol {
                        name: spec.name.clone(),
                        alias: spec.alias.clone(),
                        kind,
                        span,
                    });
                }
            }
        }
    }

    Ok(symbols)
}
