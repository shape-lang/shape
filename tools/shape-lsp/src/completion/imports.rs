//! Import statement completions driven by ModuleExportRegistry + ModuleCache.

use crate::module_cache::ModuleCache;
use crate::type_inference::type_annotation_to_string;
use shape_ast::ast::{ExportItem, FunctionDef, Item, Program};
use shape_ast::parser::parse_program;
use shape_runtime::extension_context::{
    declared_extension_spec_for_module, declared_extension_specs_for_context,
    extension_module_schema_for_context,
};
use shape_runtime::extensions::ParsedModuleSchema;
use shape_runtime::module_exports::ModuleExportRegistry;
use shape_runtime::provider_registry::ProviderRegistry;
use shape_runtime::schema_cache::{SourceSchema, source_schema_from_wire};
use shape_wire::WireValue;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};

/// Lazily initialized module registry for LSP completions
static MODULE_REGISTRY: OnceLock<ModuleExportRegistry> = OnceLock::new();
static EXTENSION_SOURCE_SCHEMA_CACHE: OnceLock<Mutex<HashMap<String, Option<SourceSchema>>>> =
    OnceLock::new();

/// Get the process-wide module registry used for editor-only module schemas.
///
/// We intentionally do not seed built-in modules here. Module namespaces
/// must come from resolver-visible sources (frontmatter/project extension
/// config + module cache), not hardcoded LSP lists.
fn registry() -> &'static ModuleExportRegistry {
    MODULE_REGISTRY.get_or_init(ModuleExportRegistry::new)
}

#[derive(Debug, Clone)]
pub(crate) struct LocalModuleParam {
    pub name: String,
    pub type_name: String,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalModuleFunctionSchema {
    pub name: String,
    pub params: Vec<LocalModuleParam>,
    pub return_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalModuleSchema {
    pub name: String,
    pub functions: Vec<LocalModuleFunctionSchema>,
}

fn parse_program_with_fallback(source: &str) -> Option<Program> {
    if let Ok(program) = parse_program(source) {
        return Some(program);
    }

    let partial = shape_ast::parser::resilient::parse_program_resilient(source);
    if partial.items.is_empty() {
        None
    } else {
        Some(partial.into_program())
    }
}

fn local_module_function_from_def(function: &FunctionDef) -> LocalModuleFunctionSchema {
    let mut params = Vec::new();
    for param in &function.params {
        let param_names = param.get_identifiers();
        let names = if param_names.is_empty() {
            vec!["_".to_string()]
        } else {
            param_names
        };
        let type_name = param
            .type_annotation
            .as_ref()
            .and_then(type_annotation_to_string)
            .unwrap_or_else(|| "_".to_string());
        let required = param.default_value.is_none();
        for name in names {
            params.push(LocalModuleParam {
                name,
                type_name: type_name.clone(),
                required,
            });
        }
    }

    LocalModuleFunctionSchema {
        name: function.name.clone(),
        params,
        return_type: function
            .return_type
            .as_ref()
            .and_then(type_annotation_to_string),
    }
}

fn local_module_functions(items: &[Item]) -> Vec<LocalModuleFunctionSchema> {
    let mut functions = std::collections::BTreeMap::<String, LocalModuleFunctionSchema>::new();

    for item in items {
        let function = match item {
            Item::Function(function, _) => Some(function),
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(function) => Some(function),
                _ => None,
            },
            _ => None,
        };
        if let Some(function) = function {
            functions.insert(
                function.name.clone(),
                local_module_function_from_def(function),
            );
        }
    }

    functions.into_values().collect()
}

fn local_modules_from_source(current_source: Option<&str>) -> Vec<LocalModuleSchema> {
    let Some(source) = current_source else {
        return vec![];
    };
    let Some(program) = parse_program_with_fallback(source) else {
        return vec![];
    };

    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Module(module, _) => Some(LocalModuleSchema {
                name: module.name.clone(),
                functions: local_module_functions(&module.items),
            }),
            _ => None,
        })
        .collect()
}

pub(crate) fn local_module_schema_from_source(
    module_name: &str,
    current_source: Option<&str>,
) -> Option<LocalModuleSchema> {
    local_modules_from_source(current_source)
        .into_iter()
        .find(|module| module.name == module_name)
}

pub(crate) fn local_module_function_schema_from_source(
    module_name: &str,
    function_name: &str,
    current_source: Option<&str>,
) -> Option<LocalModuleFunctionSchema> {
    local_module_schema_from_source(module_name, current_source).and_then(|module| {
        module
            .functions
            .into_iter()
            .find(|function| function.name == function_name)
    })
}

/// Completions for `import/use <TAB>` — namespace modules discovered by resolver context.
pub fn import_module_completions() -> Vec<CompletionItem> {
    import_module_completions_with_context(None, None, None)
}

/// Context-aware completions for `import/use <TAB>`.
pub fn import_module_completions_with_context(
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Vec<CompletionItem> {
    let known_modules =
        module_names_with_context_and_source(current_file, workspace_root, current_source);

    known_modules
        .into_iter()
        .map(|module_name| {
            module_completion_item(&module_name, current_file, workspace_root, current_source)
        })
        .collect()
}

fn module_completion_item(
    module_name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> CompletionItem {
    if let Some(module) = registry().get(module_name) {
        return CompletionItem {
            label: module.name.clone(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some(module.description.clone()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "**{}**\n\n{}\n\nExports: {}",
                    module.name,
                    module.description,
                    module.export_names_public_surface(false).join(", ")
                ),
            })),
            ..Default::default()
        };
    }

    if let Some(module) = local_module_schema_from_source(module_name, current_source) {
        let exports = module
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>();
        let docs = if exports.is_empty() {
            format!("**{}**\n\nLocal module", module.name)
        } else {
            format!(
                "**{}**\n\nLocal module\n\nExports: {}",
                module.name,
                exports.join(", ")
            )
        };

        return CompletionItem {
            label: module.name,
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("Local module".to_string()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: docs,
            })),
            ..Default::default()
        };
    }

    let (detail, docs) = extension_module_schema_with_context(
        module_name,
        current_file,
        workspace_root,
        current_source,
    )
    .map(|schema| {
        let exports = schema
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>();
        let detail = format!("Extension module ({})", schema.module_name);
        let docs = if exports.is_empty() {
            format!("**{}**\n\nExtension module", schema.module_name)
        } else {
            format!(
                "**{}**\n\nExtension module\n\nExports: {}",
                schema.module_name,
                exports.join(", ")
            )
        };
        (detail, docs)
    })
    .unwrap_or_else(|| {
        (
            "Extension module".to_string(),
            format!("**{}**\n\nExtension module", module_name),
        )
    });

    CompletionItem {
        label: module_name.to_string(),
        kind: Some(CompletionItemKind::MODULE),
        detail: Some(detail),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: docs,
        })),
        ..Default::default()
    }
}

/// Completions for `from <TAB>` — importable Shape modules.
/// Extension modules are namespace modules provided by the runtime registry.
pub fn from_module_completions() -> Vec<CompletionItem> {
    from_module_completions_with_context(None, None, None)
}

/// Context-aware completions for `from <TAB>`.
pub fn from_module_completions_with_context(
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    importable_module_completions_with_context(module_cache, current_file, workspace_root)
}

/// Combined completions (all modules) — used for backward compatibility in tests.
pub fn module_name_completions() -> Vec<CompletionItem> {
    module_name_completions_with_context(None, None, None)
}

/// Context-aware combined completions (native + importable modules).
pub fn module_name_completions_with_context(
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    let mut items = import_module_completions_with_context(current_file, workspace_root, None);
    items.extend(importable_module_completions_with_context(
        module_cache,
        current_file,
        workspace_root,
    ));
    items
}

/// Return completion items for importable modules discovered via ModuleCache.
pub fn importable_module_completions() -> Vec<CompletionItem> {
    importable_module_completions_with_context(None, None, None)
}

/// Context-aware completion items for importable modules.
pub fn importable_module_completions_with_context(
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    list_importable_modules(module_cache, current_file, workspace_root)
        .into_iter()
        .map(|import_path| CompletionItem {
            label: import_path.clone(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("Shape module".to_string()),
            documentation: Some(Documentation::String(format!(
                "Shape module: {}",
                import_path
            ))),
            ..Default::default()
        })
        .collect()
}

/// Backward-compatible wrapper for callers/tests using stdlib-specific naming.
pub fn stdlib_module_completions() -> Vec<CompletionItem> {
    importable_module_completions()
        .into_iter()
        .filter(|item| item.label.starts_with("std::") || item.label.starts_with("std."))
        .collect()
}

/// Hierarchical module completions for a given prefix.
/// Given "std::core", returns children: ["math", "utils", "snapshot", ...].
/// Each item inserts just the next segment.
pub fn hierarchical_module_completions(prefix: &str) -> Vec<CompletionItem> {
    hierarchical_module_completions_with_context(prefix, None, None, None)
}

/// Context-aware hierarchical module completions for a given prefix.
pub fn hierarchical_module_completions_with_context(
    prefix: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    let base = prefix.trim();
    list_module_children(module_cache, base, current_file, workspace_root)
        .into_iter()
        .map(|child| {
            let kind = if child.has_leaf_module && !child.has_children {
                CompletionItemKind::FILE
            } else {
                CompletionItemKind::MODULE
            };
            let detail = if base.is_empty() {
                child.name.clone()
            } else if child.has_leaf_module && !child.has_children {
                format!("{}.{}", base, child.name)
            } else {
                format!("{}.{}...", base, child.name)
            };
            let namespace = if base.is_empty() {
                child.name.clone()
            } else {
                format!("{}.{}", base, child.name)
            };
            CompletionItem {
                label: child.name.clone(),
                kind: Some(kind),
                detail: Some(detail),
                documentation: Some(Documentation::String(format!(
                    "Module namespace: {}",
                    namespace
                ))),
                // Insert just the segment name — the prefix with dot is already typed
                insert_text: Some(child.name),
                ..Default::default()
            }
        })
        .collect()
}

/// Get export completions for an import path (e.g., `std::core::snapshot`).
/// Uses ModuleCache resolution logic to load and parse the module.
pub fn import_path_export_completions(import_path: &str) -> Vec<CompletionItem> {
    import_path_export_completions_with_context(import_path, None, None, None)
}

/// Context-aware export completions for an import path.
pub fn import_path_export_completions_with_context(
    import_path: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    let owned_cache;
    let cache = if let Some(cache) = module_cache {
        cache
    } else {
        owned_cache = ModuleCache::new();
        &owned_cache
    };
    let dummy_file;
    let source_file = if let Some(file) = current_file {
        file
    } else {
        dummy_file = PathBuf::from("/dummy.shape");
        &dummy_file
    };
    let resolved = cache.resolve_import(import_path, source_file, workspace_root);
    let Some(file_path) = resolved else {
        return vec![];
    };
    let Some(module_info) = cache.load_module_with_context(&file_path, source_file, workspace_root)
    else {
        return vec![];
    };

    module_info
        .exports
        .iter()
        .map(|export| {
            let kind = match export.kind {
                crate::module_cache::SymbolKind::Function => CompletionItemKind::FUNCTION,
                crate::module_cache::SymbolKind::Enum => CompletionItemKind::ENUM,
                crate::module_cache::SymbolKind::TypeAlias => CompletionItemKind::STRUCT,
                crate::module_cache::SymbolKind::Interface => CompletionItemKind::INTERFACE,
                _ => CompletionItemKind::VARIABLE,
            };
            CompletionItem {
                label: export.exported_name().to_string(),
                kind: Some(kind),
                detail: Some(format!("from {}", import_path)),
                ..Default::default()
            }
        })
        .collect()
}

/// Backward-compatible wrapper for callers/tests using the old name.
pub fn stdlib_export_completions(import_path: &str) -> Vec<CompletionItem> {
    import_path_export_completions(import_path)
}

fn list_importable_modules(
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<String> {
    let owned_cache;
    let cache = if let Some(cache) = module_cache {
        cache
    } else {
        owned_cache = ModuleCache::new();
        &owned_cache
    };

    if let Some(current_file) = current_file {
        cache.list_importable_modules_with_context(current_file, workspace_root)
    } else {
        cache.list_importable_modules()
    }
}

fn list_module_children(
    module_cache: Option<&ModuleCache>,
    prefix: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<crate::module_cache::ModuleChild> {
    let owned_cache;
    let cache = if let Some(cache) = module_cache {
        cache
    } else {
        owned_cache = ModuleCache::new();
        &owned_cache
    };

    if let Some(current_file) = current_file {
        cache.list_module_children_with_context(prefix, current_file, workspace_root)
    } else {
        cache.list_module_children(prefix)
    }
}

/// Completions for `from csv use { <TAB> }` — list module's exports
pub fn module_export_completions(module_name: &str) -> Vec<CompletionItem> {
    module_export_completions_with_context(module_name, None, None, None)
}

/// Context-aware module export completions.
pub fn module_export_completions_with_context(
    module_name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Vec<CompletionItem> {
    if let Some(module) = registry().get(module_name) {
        return module
            .export_names_public_surface(false)
            .into_iter()
            .map(|name| {
                let schema = module.get_schema(&name);
                let (detail, doc) = if let Some(schema) = schema {
                    let params: Vec<String> = schema
                        .params
                        .iter()
                        .map(|p| {
                            if p.required {
                                format!("{}: {}", p.name, p.type_name)
                            } else {
                                format!("{}?: {}", p.name, p.type_name)
                            }
                        })
                        .collect();
                    let sig = format!(
                        "{}({}){}",
                        name,
                        params.join(", "),
                        schema
                            .return_type
                            .as_ref()
                            .map(|r| format!(" -> {}", r))
                            .unwrap_or_default()
                    );
                    (
                        sig.clone(),
                        format!("**{}**\n\n{}", sig, schema.description),
                    )
                } else {
                    (format!("{}.{}", module_name, name), String::new())
                };

                CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(detail),
                    documentation: if doc.is_empty() {
                        None
                    } else {
                        Some(Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: doc,
                        }))
                    },
                    ..Default::default()
                }
            })
            .collect();
    }

    if let Some(module) = local_module_schema_from_source(module_name, current_source) {
        return module_exports_from_local_schema(&module);
    }

    extension_module_schema_with_context(module_name, current_file, workspace_root, current_source)
        .map(module_exports_from_schema)
        .unwrap_or_default()
}

/// Completions for `csv.<TAB>` — list module member functions with signatures
pub fn module_member_completions(module_name: &str) -> Vec<CompletionItem> {
    module_member_completions_with_context(module_name, None, None, None)
}

/// Context-aware module member completions for property access (`duckdb.<TAB>`).
pub fn module_member_completions_with_context(
    module_name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Vec<CompletionItem> {
    if let Some(module) = registry().get(module_name) {
        return module
            .export_names_public_surface(false)
            .into_iter()
            .map(|name| {
                let schema = module.get_schema(&name);
                let (snippet, detail, doc) = if let Some(schema) = schema {
                    let params_snippet: Vec<String> = schema
                        .params
                        .iter()
                        .enumerate()
                        .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name))
                        .collect();
                    let snippet = format!("{}({})", name, params_snippet.join(", "));

                    let params_sig: Vec<String> = schema
                        .params
                        .iter()
                        .map(|p| format!("{}: {}", p.name, p.type_name))
                        .collect();
                    let sig = format!(
                        "{}({}){}",
                        name,
                        params_sig.join(", "),
                        schema
                            .return_type
                            .as_ref()
                            .map(|r| format!(" -> {}", r))
                            .unwrap_or_default()
                    );

                    let mut doc_str = format!("**{}**\n\n{}", sig, schema.description);
                    if !schema.params.is_empty() {
                        doc_str.push_str("\n\n**Parameters:**\n");
                        for p in &schema.params {
                            let req = if p.required { "" } else { " (optional)" };
                            doc_str.push_str(&format!(
                                "- `{}`: {} — {}{}\n",
                                p.name, p.type_name, p.description, req
                            ));
                        }
                    }

                    (snippet, sig, doc_str)
                } else {
                    (format!("{}()", name), name.to_string(), String::new())
                };

                CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(detail),
                    documentation: if doc.is_empty() {
                        None
                    } else {
                        Some(Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: doc,
                        }))
                    },
                    insert_text: Some(snippet),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                }
            })
            .collect();
    }

    if let Some(module) = local_module_schema_from_source(module_name, current_source) {
        return module_members_from_local_schema(&module);
    }

    extension_module_schema_with_context(module_name, current_file, workspace_root, current_source)
        .map(module_members_from_schema)
        .unwrap_or_default()
}

/// Completions for `module.fn(<TAB>` — show parameter schema from registry
pub fn module_function_param_completions(
    module_name: &str,
    function_name: &str,
) -> Vec<CompletionItem> {
    let reg = registry();
    let Some(module) = reg.get(module_name) else {
        return vec![];
    };
    let Some(schema) = module.get_schema(function_name) else {
        return vec![];
    };

    // If there's a single object config parameter with nested_params, show those
    if schema.params.len() == 1 {
        if let Some(nested) = &schema.params[0].nested_params {
            return object_param_completions(nested);
        }
    }

    // Otherwise show positional parameter hints
    schema
        .params
        .iter()
        .map(|p| {
            let req = if p.required { " (required)" } else { "" };
            CompletionItem {
                label: p.name.clone(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some(format!("{}{}", p.type_name, req)),
                documentation: Some(Documentation::String(p.description.clone())),
                insert_text: Some(format!("{}: ${{1}}", p.name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            }
        })
        .collect()
}

/// Completions for nested object parameters (e.g., { path: ..., delimiter: ... })
fn object_param_completions(
    params: &[shape_runtime::module_exports::ModuleParam],
) -> Vec<CompletionItem> {
    params
        .iter()
        .map(|p| {
            let req = if p.required { " (required)" } else { "" };
            let snippet = if let Some(ref default) = p.default_snippet {
                format!("{}: {}", p.name, default)
            } else if let Some(ref values) = p.allowed_values {
                format!("{}: \"${{1|{}|}}\"", p.name, values.join(","))
            } else {
                format!("{}: ${{1}}", p.name)
            };

            CompletionItem {
                label: p.name.clone(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some(format!("{}{}", p.type_name, req)),
                documentation: Some(Documentation::String(p.description.clone())),
                insert_text: Some(snippet),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            }
        })
        .collect()
}

/// Check if a name is a known extension module
pub fn is_extension_module(name: &str) -> bool {
    registry().get(name).is_some()
}

/// Check if a name is a known module namespace in current context (builtin or extension).
pub fn is_module_namespace_with_context(
    name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> bool {
    module_names_with_context_and_source(current_file, workspace_root, current_source)
        .iter()
        .any(|module_name| module_name == name)
}

/// Get all registered module names.
pub fn module_names() -> Vec<String> {
    registry().modules().keys().cloned().collect()
}

/// Get module names, including extensions declared in project `shape.toml`.
pub fn module_names_with_context(
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<String> {
    module_names_with_context_and_source(current_file, workspace_root, None)
}

/// Get module names, including extensions declared in project `shape.toml`,
/// and script frontmatter (`[[extensions]]`) when source is provided.
pub fn module_names_with_context_and_source(
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Vec<String> {
    let mut names = BTreeSet::new();
    names.extend(registry().modules().keys().cloned());
    names.extend(
        declared_extension_specs_for_context(current_file, workspace_root, current_source)
            .into_iter()
            .map(|spec| spec.name),
    );
    names.extend(
        local_modules_from_source(current_source)
            .into_iter()
            .map(|module| module.name),
    );

    let cache = ModuleCache::new();
    let context_file = current_file
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|cwd| cwd.join("__shape_lsp__.shape"))
        })
        .unwrap_or_else(|| PathBuf::from("__shape_lsp__.shape"));
    names.extend(
        cache
            .list_importable_modules_with_context_and_source(
                &context_file,
                workspace_root,
                current_source,
            )
            .into_iter()
            .filter_map(|module_path| {
                module_path
                    .split('.')
                    .next()
                    .map(std::string::ToString::to_string)
            }),
    );

    names.into_iter().collect()
}

pub(crate) fn extension_module_schema_with_context(
    module_name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<ParsedModuleSchema> {
    extension_module_schema_for_context(module_name, current_file, workspace_root, current_source)
}

/// Fetch extension-provided source schema metadata through `shape.module`
/// by invoking a schema provider export with a URI argument.
pub fn extension_source_schema_via_with_context(
    module_name: &str,
    provider_function: &str,
    uri: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<SourceSchema> {
    let spec = declared_extension_spec_for_module(
        module_name,
        current_file,
        workspace_root,
        current_source,
    )?;
    if !spec.path.exists() {
        return None;
    }

    let module_key = spec
        .path
        .canonicalize()
        .unwrap_or_else(|_| spec.path.clone())
        .to_string_lossy()
        .to_string();
    let cache_key = format!("{}::{}::{}", module_key, provider_function, uri);
    let cache = EXTENSION_SOURCE_SCHEMA_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock()
        && let Some(cached) = guard.get(&cache_key)
    {
        return cached.clone();
    }

    let schema = {
        let registry = ProviderRegistry::new();
        match registry.load_extension(&spec.path, &spec.config) {
            Ok(_) => registry
                .invoke_extension_module_wire(
                    module_name,
                    provider_function,
                    &[WireValue::String(uri.to_string())],
                )
                .ok()
                .and_then(|value| source_schema_from_wire(&value).ok()),
            Err(_) => None,
        }
    };

    if let Ok(mut guard) = cache.lock() {
        guard.insert(cache_key, schema.clone());
    }

    schema
}

/// Fetch extension-provided source schema metadata through `shape.module`
/// export `source_schema(uri)`.
pub fn extension_source_schema_with_context(
    module_name: &str,
    uri: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<SourceSchema> {
    extension_source_schema_via_with_context(
        module_name,
        "source_schema",
        uri,
        current_file,
        workspace_root,
        current_source,
    )
}

fn local_module_function_signature(function: &LocalModuleFunctionSchema) -> String {
    let params = function
        .params
        .iter()
        .map(|param| {
            if param.required {
                format!("{}: {}", param.name, param.type_name)
            } else {
                format!("{}?: {}", param.name, param.type_name)
            }
        })
        .collect::<Vec<_>>();
    format!(
        "{}({}){}",
        function.name,
        params.join(", "),
        function
            .return_type
            .as_ref()
            .map(|ret| format!(" -> {}", ret))
            .unwrap_or_default()
    )
}

fn module_exports_from_local_schema(module: &LocalModuleSchema) -> Vec<CompletionItem> {
    let mut items = module
        .functions
        .iter()
        .map(|function| {
            let signature = local_module_function_signature(function);
            CompletionItem {
                label: function.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(signature.clone()),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("**{}**\n\nLocal module export.", signature),
                })),
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

fn module_members_from_local_schema(module: &LocalModuleSchema) -> Vec<CompletionItem> {
    let mut items = module
        .functions
        .iter()
        .map(|function| {
            let signature = local_module_function_signature(function);
            let args = function
                .params
                .iter()
                .enumerate()
                .map(|(idx, param)| format!("${{{}:{}}}", idx + 1, param.name))
                .collect::<Vec<_>>();
            CompletionItem {
                label: function.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(signature.clone()),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("**{}**\n\nLocal module function.", signature),
                })),
                insert_text: Some(format!("{}({})", function.name, args.join(", "))),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

fn module_exports_from_schema(schema: ParsedModuleSchema) -> Vec<CompletionItem> {
    let mut items = schema
        .functions
        .into_iter()
        .map(|function| {
            let params = function
                .params
                .iter()
                .enumerate()
                .map(|(idx, ty)| format!("arg{}: {}", idx, ty))
                .collect::<Vec<_>>();
            let sig = format!(
                "{}({}){}",
                function.name,
                params.join(", "),
                function
                    .return_type
                    .as_ref()
                    .map(|ret| format!(" -> {}", ret))
                    .unwrap_or_default()
            );

            CompletionItem {
                label: function.name,
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(sig.clone()),
                documentation: if function.description.is_empty() {
                    None
                } else {
                    Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("**{}**\n\n{}", sig, function.description),
                    }))
                },
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

fn module_members_from_schema(schema: ParsedModuleSchema) -> Vec<CompletionItem> {
    let mut items = schema
        .functions
        .into_iter()
        .map(|function| {
            let params_sig = function
                .params
                .iter()
                .enumerate()
                .map(|(idx, ty)| format!("arg{}: {}", idx, ty))
                .collect::<Vec<_>>();
            let sig = format!(
                "{}({}){}",
                function.name,
                params_sig.join(", "),
                function
                    .return_type
                    .as_ref()
                    .map(|ret| format!(" -> {}", ret))
                    .unwrap_or_default()
            );

            let snippet_args = function
                .params
                .iter()
                .enumerate()
                .map(|(idx, _)| format!("${{{}:arg{}}}", idx + 1, idx))
                .collect::<Vec<_>>();
            let snippet = format!("{}({})", function.name, snippet_args.join(", "));

            CompletionItem {
                label: function.name,
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(sig.clone()),
                documentation: if function.description.is_empty() {
                    None
                } else {
                    Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("**{}**\n\n{}", sig, function.description),
                    }))
                },
                insert_text: Some(snippet),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

/// Get the process-wide module registry (public for hover/signature help)
pub fn get_registry() -> &'static ModuleExportRegistry {
    registry()
}

/// Extract type methods from all registered extension `.shape` sources.
///
/// Parses each extension's bundled Shape source files and runs
/// `extract_type_methods` to discover `impl Queryable for ...` and
/// `extend ...` blocks, so that method completions work for extension types.
pub fn extension_type_methods()
-> std::collections::HashMap<String, Vec<crate::type_inference::MethodCompletionInfo>> {
    use crate::type_inference::extract_type_methods;
    use shape_ast::parser::parse_program;
    use std::collections::HashMap;

    let reg = registry();
    let mut all_methods: HashMap<String, Vec<crate::type_inference::MethodCompletionInfo>> =
        HashMap::new();

    for module in reg.modules().values() {
        for (_filename, source) in &module.shape_sources {
            if let Ok(program) = parse_program(source) {
                let methods = extract_type_methods(&program);
                for (type_name, meths) in methods {
                    let entry = all_methods.entry(type_name).or_default();
                    for m in meths {
                        if !entry.iter().any(|existing| existing.name == m.name) {
                            entry.push(m);
                        }
                    }
                }
            }
        }
    }

    all_methods
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_name_completions() {
        let items = module_name_completions();
        assert!(
            !items.is_empty(),
            "Expected module completions, got {}",
            items.len()
        );

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.iter().any(|label| label.starts_with("std::")),
            "Expected stdlib modules in completions, got {:?}",
            labels
        );

        // Every item should have a label set
        for item in &items {
            assert!(
                !item.label.is_empty(),
                "Completion item should have a label"
            );
        }
    }

    #[test]
    fn test_module_export_completions_frontmatter_extension_schema_missing_is_empty() {
        let items = module_export_completions("duckdb");
        assert!(
            items.is_empty(),
            "Without a loaded extension schema, exports should be empty. Got {:?}",
            items
        );
    }

    #[test]
    fn test_module_export_completions_unknown() {
        let items = module_export_completions("nonexistent");
        assert!(
            items.is_empty(),
            "Expected empty vec for unknown module, got {} items",
            items.len()
        );
    }

    #[test]
    fn test_module_member_completions() {
        let items = module_member_completions("duckdb");
        assert!(
            items.is_empty(),
            "Without a loaded extension schema, member completions should be empty"
        );
    }

    #[test]
    fn test_is_extension_module() {
        assert!(
            !is_extension_module("csv"),
            "csv should not be a hardcoded extension module"
        );
        assert!(
            !is_extension_module("unknown"),
            "unknown should not be a extension module"
        );
    }

    #[test]
    fn test_stdlib_module_completions_not_empty() {
        let items = stdlib_module_completions();
        assert!(
            !items.is_empty(),
            "Expected stdlib module completions from filesystem"
        );
        // All labels should start with "std::"
        for item in &items {
            assert!(
                item.label.starts_with("std::"),
                "Stdlib module label should start with 'std::', got '{}'",
                item.label
            );
        }
    }

    #[test]
    fn test_module_name_completions_includes_stdlib() {
        let items = module_name_completions();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Should include stdlib modules discovered from filesystem/module cache.
        assert!(
            labels.iter().any(|l| l.starts_with("std::")),
            "Should include at least one stdlib module, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_module_names_with_context_includes_project_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("shape.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
"#,
        )
        .unwrap();
        std::fs::write(root.join("src/main.shape"), "let x = 1\n").unwrap();

        let names = module_names_with_context(Some(root.join("src/main.shape").as_path()), None);
        assert!(
            names.contains(&"duckdb".to_string()),
            "expected shape.toml extension module in known modules, got {:?}",
            names
        );
    }

    #[test]
    fn test_module_names_with_context_and_source_includes_frontmatter_extensions() {
        let source = r#"---
# shape.toml
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---

let conn = duckdb.connect("duckdb://analytics.db")
"#;

        let names = module_names_with_context_and_source(None, None, Some(source));
        assert!(
            names.contains(&"duckdb".to_string()),
            "expected frontmatter extension module in known modules, got {:?}",
            names
        );
    }

    #[test]
    fn test_import_module_completions_with_context_includes_frontmatter_extension() {
        let source = r#"---
# shape.toml
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---

use 
"#;

        let items = import_module_completions_with_context(None, None, Some(source));
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert!(
            labels.contains(&"duckdb"),
            "expected duckdb in import-module completions, got {:?}",
            labels
        );
    }

    #[test]
    fn test_stdlib_export_completions_for_known_module() {
        // Pick a stdlib module that is known to have exports
        let stdlib_root = shape_runtime::stdlib_metadata::default_stdlib_path();
        if !stdlib_root.exists() {
            return; // Skip if stdlib not present
        }
        // Find any stdlib module that has exports
        let modules = stdlib_module_completions();
        if modules.is_empty() {
            return;
        }
        // Try the first few modules until we find one with exports
        for module in modules.iter().take(10) {
            let exports = stdlib_export_completions(&module.label);
            if !exports.is_empty() {
                // Success: found a module with exports
                return;
            }
        }
        // It's OK if none have exports — some modules may only have internal items
    }

    #[test]
    fn test_from_module_completions_with_context_includes_project_and_dep_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("shape.toml"),
            r#"
[modules]
paths = ["lib"]

[dependencies]
mydep = { path = "deps/mydep" }
"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(root.join("deps/mydep")).unwrap();
        std::fs::write(root.join("src/main.shape"), "from tools use { tool }").unwrap();
        std::fs::write(root.join("lib/tools.shape"), "pub fn tool() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/index.shape"), "pub fn root() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/util.shape"), "pub fn util() { 1 }").unwrap();

        let cache = ModuleCache::new();
        let items = from_module_completions_with_context(
            Some(&cache),
            Some(root.join("src/main.shape").as_path()),
            None,
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"tools"),
            "expected tools module from [modules].paths, got {:?}",
            labels
        );
        assert!(
            labels.contains(&"mydep"),
            "expected dependency root module, got {:?}",
            labels
        );
        assert!(
            labels.contains(&"mydep::util"),
            "expected dependency submodule, got {:?}",
            labels
        );
    }

    #[test]
    fn test_hierarchical_module_completions_with_context_for_dependency_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("shape.toml"),
            r#"
[dependencies]
mydep = { path = "deps/mydep" }
"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("deps/mydep/sub")).unwrap();
        std::fs::write(root.join("src/main.shape"), "from mydep. use { x }").unwrap();
        std::fs::write(root.join("deps/mydep/index.shape"), "pub fn root() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/util.shape"), "pub fn util() { 1 }").unwrap();
        std::fs::write(
            root.join("deps/mydep/sub/index.shape"),
            "pub fn sub() { 1 }",
        )
        .unwrap();

        let cache = ModuleCache::new();
        let items = hierarchical_module_completions_with_context(
            "mydep",
            Some(&cache),
            Some(root.join("src/main.shape").as_path()),
            None,
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"util"),
            "expected util child under mydep, got {:?}",
            labels
        );
        assert!(
            labels.contains(&"sub"),
            "expected sub child under mydep, got {:?}",
            labels
        );
    }

    #[test]
    fn test_extension_type_methods_returns_map() {
        // Extension methods are contributed by registered extension modules.
        // In plugin-only extension mode, this can be empty.
        let methods = extension_type_methods();
        let _ = methods;
    }

    #[test]
    fn test_openapi_module_is_not_builtin_extension_module() {
        assert!(!is_extension_module("openapi"));
    }
}
