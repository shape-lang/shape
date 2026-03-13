//! Shared module resolution utilities.
//!
//! Types and functions used by both `shape-runtime` (module loader) and
//! `shape-vm` (import inlining) to inspect module exports and manipulate
//! AST item lists during import resolution.

use crate::ast::{ExportItem, Item, Program, Span};
use crate::error::{Result, ShapeError};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// High-level kind of an exported symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleExportKind {
    Function,
    BuiltinFunction,
    TypeAlias,
    BuiltinType,
    Interface,
    Enum,
    Annotation,
    Value,
}

/// Exported symbol metadata discovered from a module's AST.
#[derive(Debug, Clone)]
pub struct ModuleExportSymbol {
    /// Original symbol name in module scope.
    pub name: String,
    /// Alias if exported as `name as alias`.
    pub alias: Option<String>,
    /// High-level symbol kind.
    pub kind: ModuleExportKind,
    /// Source span for navigation/diagnostics.
    pub span: Span,
}

// ---------------------------------------------------------------------------
// direct_export_target
// ---------------------------------------------------------------------------

/// Map a direct (non-`Named`) export item to its name and kind.
///
/// Returns `None` for `ExportItem::Named`, which requires scope-level
/// resolution handled by [`collect_exported_symbols`].
pub fn direct_export_target(export_item: &ExportItem) -> Option<(String, ModuleExportKind)> {
    match export_item {
        ExportItem::Function(function) => {
            Some((function.name.clone(), ModuleExportKind::Function))
        }
        ExportItem::BuiltinFunction(function) => {
            Some((function.name.clone(), ModuleExportKind::BuiltinFunction))
        }
        ExportItem::BuiltinType(type_decl) => {
            Some((type_decl.name.clone(), ModuleExportKind::BuiltinType))
        }
        ExportItem::TypeAlias(alias) => Some((alias.name.clone(), ModuleExportKind::TypeAlias)),
        ExportItem::Enum(enum_def) => Some((enum_def.name.clone(), ModuleExportKind::Enum)),
        ExportItem::Struct(struct_def) => {
            Some((struct_def.name.clone(), ModuleExportKind::TypeAlias))
        }
        ExportItem::Interface(interface) => {
            Some((interface.name.clone(), ModuleExportKind::Interface))
        }
        ExportItem::Trait(trait_def) => {
            Some((trait_def.name.clone(), ModuleExportKind::Interface))
        }
        ExportItem::Annotation(annotation) => {
            Some((annotation.name.clone(), ModuleExportKind::Annotation))
        }
        ExportItem::ForeignFunction(function) => {
            Some((function.name.clone(), ModuleExportKind::Function))
        }
        ExportItem::Named(_) => None,
    }
}

// ---------------------------------------------------------------------------
// strip_import_items
// ---------------------------------------------------------------------------

/// Remove all `Item::Import` entries from a list of AST items.
///
/// Used when inlining module contents into a consumer program — the module's
/// own imports have already been resolved and should not pollute the
/// consumer's import set.
pub fn strip_import_items(items: Vec<Item>) -> Vec<Item> {
    items
        .into_iter()
        .filter(|item| !matches!(item, Item::Import(..)))
        .collect()
}

// ---------------------------------------------------------------------------
// collect_exported_symbols
// ---------------------------------------------------------------------------

/// Internal scope-symbol kind mirroring [`ModuleExportKind`] for scope
/// resolution of `export { name }` statements.
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

fn scope_symbol_kind_to_export(kind: ScopeSymbolKind) -> ModuleExportKind {
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

/// Lightweight scope used to resolve `export { name }` to the right kind.
struct ScopeTable {
    symbols: std::collections::HashMap<String, (ScopeSymbolKind, Span)>,
}

impl ScopeTable {
    fn from_program(program: &Program) -> Self {
        let mut symbols = std::collections::HashMap::new();
        for item in &program.items {
            match item {
                Item::Function(f, span) => {
                    symbols.insert(f.name.clone(), (ScopeSymbolKind::Function, *span));
                }
                Item::BuiltinFunctionDecl(f, span) => {
                    symbols.insert(f.name.clone(), (ScopeSymbolKind::BuiltinFunction, *span));
                }
                Item::BuiltinTypeDecl(t, span) => {
                    symbols.insert(t.name.clone(), (ScopeSymbolKind::BuiltinType, *span));
                }
                Item::TypeAlias(a, span) => {
                    symbols.insert(a.name.clone(), (ScopeSymbolKind::TypeAlias, *span));
                }
                Item::Enum(e, span) => {
                    symbols.insert(e.name.clone(), (ScopeSymbolKind::Enum, *span));
                }
                Item::StructType(s, span) => {
                    symbols.insert(s.name.clone(), (ScopeSymbolKind::TypeAlias, *span));
                }
                Item::Interface(i, span) => {
                    symbols.insert(i.name.clone(), (ScopeSymbolKind::Interface, *span));
                }
                Item::Trait(t, span) => {
                    symbols.insert(t.name.clone(), (ScopeSymbolKind::Interface, *span));
                }
                Item::VariableDecl(decl, span) => {
                    if let Some(name) = decl.pattern.as_identifier() {
                        symbols.insert(name.to_string(), (ScopeSymbolKind::Value, *span));
                    }
                }
                Item::AnnotationDef(a, span) => {
                    symbols.insert(a.name.clone(), (ScopeSymbolKind::Annotation, *span));
                }
                _ => {}
            }
        }
        Self { symbols }
    }

    fn resolve(&self, name: &str) -> Option<(ScopeSymbolKind, Span)> {
        self.symbols.get(name).copied()
    }
}

/// Collect exported symbol metadata from a parsed module AST.
///
/// This is the canonical implementation shared by both the runtime module
/// loader and the VM import inliner. It handles both direct exports
/// (`pub fn`, `pub type`, etc.) and named re-exports (`export { a, b }`).
pub fn collect_exported_symbols(program: &Program) -> Result<Vec<ModuleExportSymbol>> {
    let scope = ScopeTable::from_program(program);
    let mut symbols = Vec::new();

    for item in &program.items {
        let Item::Export(export, _) = item else {
            continue;
        };

        // Direct exports: the ExportItem already carries name + kind.
        if let Some((name, kind)) = direct_export_target(&export.item) {
            let span = match &export.item {
                ExportItem::Function(f) => f.name_span,
                ExportItem::BuiltinFunction(f) => f.name_span,
                ExportItem::Annotation(a) => a.name_span,
                ExportItem::ForeignFunction(f) => f.name_span,
                _ => scope
                    .resolve(&name)
                    .map(|(_, span)| span)
                    .unwrap_or_default(),
            };
            symbols.push(ModuleExportSymbol {
                name,
                alias: None,
                kind,
                span,
            });
            continue;
        }

        // Named re-exports: resolve through scope table.
        if let ExportItem::Named(specs) = &export.item {
            for spec in specs {
                match scope.resolve(&spec.name) {
                    Some((kind, span)) => {
                        if kind == ScopeSymbolKind::Value {
                            return Err(ShapeError::ModuleError {
                                message: format!(
                                    "Cannot export variable '{}': variable exports are not yet supported. \
                                     Only functions and types can be exported.",
                                    spec.name
                                ),
                                module_path: None,
                            });
                        }
                        symbols.push(ModuleExportSymbol {
                            name: spec.name.clone(),
                            alias: spec.alias.clone(),
                            kind: scope_symbol_kind_to_export(kind),
                            span,
                        });
                    }
                    None => {
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
    }

    Ok(symbols)
}

// ---------------------------------------------------------------------------
// export_kind_description
// ---------------------------------------------------------------------------

/// Human-readable description of an export kind for diagnostics.
pub fn export_kind_description(kind: ModuleExportKind) -> &'static str {
    match kind {
        ModuleExportKind::Function => "a function",
        ModuleExportKind::BuiltinFunction => "a builtin function",
        ModuleExportKind::TypeAlias => "a type",
        ModuleExportKind::BuiltinType => "a builtin type",
        ModuleExportKind::Interface => "an interface",
        ModuleExportKind::Enum => "an enum",
        ModuleExportKind::Annotation => "an annotation",
        ModuleExportKind::Value => "a value",
    }
}
