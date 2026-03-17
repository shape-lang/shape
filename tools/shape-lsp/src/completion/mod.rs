//! Code completion provider for Shape
//!
//! Provides intelligent autocomplete suggestions based on context.
//! All language information comes from shape-core metadata API (single source of truth).

// Module declarations
pub mod annotations;
pub mod docs;
pub mod functions;
pub mod imports;
pub mod inference;
pub mod methods;
pub mod providers;
pub mod snippets;
pub mod types;

// Re-exports for backward compatibility
pub use annotations::{annotation_completions, enum_value_completions, symbols_with_annotation};
pub use functions::{
    builtin_function_completions, comptime_builtin_function_completions,
    function_argument_completions, function_completion_item, keyword_completions,
    object_property_name_completions, object_property_value_completions,
};
pub use inference::{infer_param_types, infer_types, infer_types_with_context, type_to_string};
pub use methods::{
    extract_option_inner, extract_result_inner, method_completion_item, option_method_completions,
    result_method_completions,
};
pub use providers::provider_completions;
pub use snippets::{create_snippet, snippet_completions};
pub use types::{
    is_column_type, pipe_target_completions, property_completion_item, property_completions,
    resolve_base_type, resolve_object_type, resolve_property_type, type_completions,
};

use crate::annotation_discovery::AnnotationDiscovery;
use crate::context::{CompletionContext, analyze_context, is_inside_interpolation_expression};
use crate::grammar_completion::get_grammar_completions;
use crate::module_cache::ModuleCache;
use crate::symbols::{SymbolKind, extract_symbols, symbols_to_completions};
use crate::trait_lookup::resolve_trait_definition;
use crate::type_inference::{
    MethodCompletionInfo, extract_struct_fields, extract_type_methods, unified_metadata,
};
use crate::util::position_to_offset;
use shape_ast::ast::{Item, MethodDef, Program, Span, Statement, TypeName};
use shape_ast::parse_program_resilient;
use shape_ast::parser::parse_program;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind, Position};

/// Generate completion items for a given position in a document
/// Returns (completions, updated_symbols, updated_types) where updated_symbols is Some if parsing succeeded
pub fn get_completions(
    text: &str,
    position: Position,
    cached_symbols: &[crate::symbols::SymbolInfo],
    cached_types: &HashMap<String, String>,
) -> (
    Vec<CompletionItem>,
    Option<Vec<crate::symbols::SymbolInfo>>,
    Option<HashMap<String, String>>,
) {
    get_completions_with_context(
        text,
        position,
        cached_symbols,
        cached_types,
        None,
        None,
        None,
    )
}

/// Generate completion items with module-resolution context for imports/exports.
pub fn get_completions_with_context(
    text: &str,
    position: Position,
    cached_symbols: &[crate::symbols::SymbolInfo],
    cached_types: &HashMap<String, String>,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> (
    Vec<CompletionItem>,
    Option<Vec<crate::symbols::SymbolInfo>>,
    Option<HashMap<String, String>>,
) {
    let mut completions = Vec::new();
    let cursor_offset = position_to_offset(text, position);
    let mut parsed_program = None;

    // Analyze context to determine what completions to show
    let context = analyze_context(text, position);

    // Parse the document to extract user-defined symbols and annotations
    let (
        user_symbols,
        updated_symbols,
        updated_types,
        annotation_discovery,
        struct_fields,
        impl_methods,
        named_impls,
        receiver_type_at_cursor,
    ) = if let Ok(mut program) = parse_program(text) {
        let analysis = analyze_parsed_program(
            &mut program,
            module_cache,
            current_file,
            workspace_root,
            text,
            cursor_offset,
        );
        parsed_program = Some(program);
        analysis
    } else {
        // Strict parse failed — try resilient parse for partial recovery
        let partial = parse_program_resilient(text);
        if !partial.items.is_empty() {
            let mut program = partial.into_program();
            let analysis = analyze_parsed_program(
                &mut program,
                module_cache,
                current_file,
                workspace_root,
                text,
                cursor_offset,
            );
            parsed_program = Some(program);
            analysis
        } else {
            // No items recovered — fall back to cached state
            (
                cached_symbols.to_vec(),
                None,
                None,
                AnnotationDiscovery::new(),
                HashMap::new(),
                HashMap::new(),
                extract_named_impl_names_fallback(text),
                None,
            )
        }
    };

    let mut type_context = updated_types
        .clone()
        .unwrap_or_else(|| cached_types.clone());
    if is_inside_interpolation_expression(text, position) {
        if let Some(receiver_type) = receiver_type_at_cursor {
            type_context.insert("self".to_string(), receiver_type);
        }
    }

    if is_using_impl_selector_context(text, position) {
        completions.extend(named_impl_selector_completions(&named_impls));
        return (completions, updated_symbols, updated_types);
    }

    match context {
        CompletionContext::ImportModule => {
            completions.extend(imports::import_module_completions_with_context(
                current_file,
                workspace_root,
                Some(text),
            ));
        }
        CompletionContext::FromModule => {
            completions.extend(imports::from_module_completions_with_context(
                module_cache,
                current_file,
                workspace_root,
            ));
        }
        CompletionContext::FromModulePartial { prefix } => {
            completions.extend(imports::hierarchical_module_completions_with_context(
                &prefix,
                module_cache,
                current_file,
                workspace_root,
            ));
        }
        CompletionContext::ImportItems { module } => {
            let module_exports = imports::module_export_completions_with_context(
                &module,
                current_file,
                workspace_root,
                Some(text),
            );
            if !module_exports.is_empty() {
                completions.extend(module_exports);
            } else {
                // Fall back to import-path module exports via ModuleCache resolution.
                completions.extend(imports::import_path_export_completions_with_context(
                    &module,
                    module_cache,
                    current_file,
                    workspace_root,
                ));
            }
        }
        CompletionContext::PropertyAccess { object } => {
            // If the object is a extension module, show module exports
            if imports::is_module_namespace_with_context(
                &object,
                current_file,
                workspace_root,
                Some(text),
            ) {
                completions.extend(imports::module_member_completions_with_context(
                    &object,
                    current_file,
                    workspace_root,
                    Some(text),
                ));
                return (completions, updated_symbols, updated_types);
            }
            // Show properties based on object type
            completions.extend(property_completions(
                &object,
                &type_context,
                &struct_fields,
                &impl_methods,
            ));
        }
        CompletionContext::PatternReference => {
            // Show all user-defined functions as potential pattern references
            let function_symbols: Vec<_> = user_symbols
                .iter()
                .filter(|s| s.kind == SymbolKind::Function)
                .cloned()
                .collect();
            completions.extend(symbols_to_completions(&function_symbols));
        }
        CompletionContext::TypeAnnotation => {
            // Show type names
            completions.extend(type_completions());
        }
        CompletionContext::FunctionCall {
            function,
            arg_context,
        } => {
            // Inside function call - intelligent argument-specific completions
            completions.extend(function_argument_completions(
                &user_symbols,
                &function,
                &arg_context,
            ));
        }
        CompletionContext::Annotation => {
            // Show discovered annotations after "@"
            completions.extend(annotation_completions(
                &annotation_discovery,
                parsed_program.as_ref(),
                module_cache,
                current_file,
                workspace_root,
            ));
        }
        CompletionContext::AnnotationArgs { annotation } => {
            // Show annotation-specific argument completions
            // If the annotation is known, show its parameter names first
            if let Some(info) = annotation_discovery.get(&annotation) {
                for param in &info.params {
                    completions.push(CompletionItem {
                        label: param.clone(),
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: Some(format!("@{} parameter", annotation)),
                        ..Default::default()
                    });
                }
            }
            // Also show all symbols as fallback
            completions.extend(symbols_to_completions(&user_symbols));
        }
        CompletionContext::ComptimeBlock => {
            // Inside `comptime { }` — offer comptime builtins first, then normal completions
            completions.extend(comptime_builtin_function_completions());
            completions.extend(symbols_to_completions(&user_symbols));
            completions.extend(builtin_function_completions());
        }
        CompletionContext::ExprAnnotation => {
            // After `@` in expression position — same as item-level annotations
            completions.extend(annotation_completions(
                &annotation_discovery,
                parsed_program.as_ref(),
                module_cache,
                current_file,
                workspace_root,
            ));
        }
        CompletionContext::DocTag { prefix } => {
            completions.extend(docs::doc_tag_completions(&prefix));
        }
        CompletionContext::DocParamName { prefix } => {
            if let (Some(program), Some(offset)) = (parsed_program.as_ref(), cursor_offset) {
                completions.extend(docs::doc_param_completions(program, offset, &prefix));
            }
        }
        CompletionContext::DocTypeParamName { prefix } => {
            if let (Some(program), Some(offset)) = (parsed_program.as_ref(), cursor_offset) {
                completions.extend(docs::doc_type_param_completions(program, offset, &prefix));
            }
        }
        CompletionContext::DocLinkTarget { prefix } => {
            if let Some(program) = parsed_program.as_ref() {
                completions.extend(docs::doc_link_completions(
                    program,
                    &prefix,
                    module_cache,
                    current_file,
                    workspace_root,
                ));
            }
        }
        CompletionContext::PipeTarget { pipe_input_type } => {
            completions.extend(types::pipe_target_completions(
                pipe_input_type.as_deref(),
                &type_context,
                &impl_methods,
            ));
            // Also show user-defined functions
            let func_symbols: Vec<_> = user_symbols
                .iter()
                .filter(|s| s.kind == crate::symbols::SymbolKind::Function)
                .cloned()
                .collect();
            completions.extend(symbols_to_completions(&func_symbols));
            // Show builtin functions
            completions.extend(builtin_function_completions());
        }
        CompletionContext::ImplBlock {
            trait_name,
            target_type: _,
            existing_methods,
        } => {
            // Inside an impl block — suggest unimplemented trait methods
            completions.extend(impl_block_completions(
                text,
                &trait_name,
                &existing_methods,
                module_cache,
                current_file,
                workspace_root,
            ));
        }
        CompletionContext::TypeAliasOverride { base_type } => {
            // Inside `type X = Y { | }` — suggest comptime fields from base type
            completions.extend(comptime_field_override_completions(
                &struct_fields,
                &base_type,
            ));
        }
        CompletionContext::JoinStrategy => {
            // After `await join ` — suggest join strategies
            completions.extend(join_strategy_completions());
        }
        CompletionContext::JoinBody { strategy } => {
            // Inside `join <strategy> { | }` — suggest labeled branch snippets
            completions.extend(join_branch_completions(&strategy));
            // Also show user symbols and builtins for branch expressions
            completions.extend(symbols_to_completions(&user_symbols));
            completions.extend(builtin_function_completions());
        }
        CompletionContext::TraitBound => {
            // In trait bound position `<T: |>` — suggest known trait names
            completions.extend(trait_bound_completions(text));
        }
        CompletionContext::InterpolationFormatSpec { spec_prefix } => {
            completions.extend(interpolation_format_spec_completions(&spec_prefix));
        }
        _ => {
            // General context - show all completions
            // 1. Grammar-driven completions (based on parser expectations)
            // Truncate text at cursor to simulate "typing here"
            let byte_offset = position_to_offset(text, position);
            if let Some(offset) = byte_offset {
                let truncated = &text[..offset];
                completions.extend(get_grammar_completions(truncated));
            }

            // 2. Standard completions
            completions.extend(all_completions(&user_symbols));
        }
    }

    // De-duplicate cross-source completion labels before ranking.
    dedupe_completion_items(&mut completions);

    // Type-aware scoring: boost completions matching expected type
    if let Some(expected) = expected_type_at_cursor(text, position, &type_context) {
        boost_completions_by_type(&mut completions, &expected, &type_context);
    }

    (completions, updated_symbols, updated_types)
}

fn is_using_impl_selector_context(text: &str, position: Position) -> bool {
    let Some(offset) = position_to_offset(text, position) else {
        return false;
    };
    let prefix = &text[..offset];
    let line_prefix = prefix.rsplit('\n').next().unwrap_or(prefix);

    let Some(using_idx) = line_prefix.rfind("using") else {
        return false;
    };

    let before = &line_prefix[..using_idx];
    if before
        .chars()
        .last()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }

    let after = &line_prefix[using_idx + "using".len()..];
    if !after
        .chars()
        .all(|c| c.is_ascii_whitespace() || c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }

    after.chars().any(|c| c.is_ascii_whitespace())
}

fn named_impl_selector_completions(named_impls: &[String]) -> Vec<CompletionItem> {
    named_impls
        .iter()
        .map(|name| CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::REFERENCE),
            detail: Some("Named trait implementation".to_string()),
            documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                "Select self named implementation with `expr using ImplName`.".to_string(),
            )),
            ..Default::default()
        })
        .collect()
}

/// Shared analysis for a parsed program (used by both strict and resilient parse paths).
#[allow(clippy::type_complexity)]
fn analyze_parsed_program(
    program: &mut Program,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    text: &str,
    cursor_offset: Option<usize>,
) -> (
    Vec<crate::symbols::SymbolInfo>,
    Option<Vec<crate::symbols::SymbolInfo>>,
    Option<HashMap<String, String>>,
    AnnotationDiscovery,
    HashMap<String, Vec<(String, String)>>,
    HashMap<String, Vec<MethodCompletionInfo>>,
    Vec<String>,
    Option<String>,
) {
    // Desugar query syntax before analysis
    shape_ast::transform::desugar_program(program);
    let symbols = extract_symbols(program);
    let mut inferred_types =
        infer_types_with_context(program, current_file, workspace_root, Some(text));
    if let Some(types) = inferred_types.as_mut() {
        let param_types = infer_param_types(program, types);
        types.extend(param_types);
    }

    // Discover annotations from the program
    let mut ann_discovery = AnnotationDiscovery::new();
    ann_discovery.discover_from_program(program);
    if let (Some(cache), Some(file_path)) = (module_cache, current_file) {
        ann_discovery.discover_from_imports_with_cache(program, file_path, cache, workspace_root);
    } else {
        ann_discovery.discover_from_imports(program);
    }

    // Extract struct type fields for property completions
    let fields = extract_struct_fields(program);

    // Extract methods from impl/extend/trait blocks
    let mut impl_meths = extract_type_methods(program);
    let mut named_impl_names: Vec<String> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(impl_block, _) => impl_block.impl_name.clone(),
            _ => None,
        })
        .collect();
    named_impl_names.sort();
    named_impl_names.dedup();

    // Merge methods from extension .shape sources (e.g., duckdb.shape, openapi.shape)
    for (type_name, ext_methods) in imports::extension_type_methods() {
        let entry = impl_meths.entry(type_name).or_default();
        for m in ext_methods {
            if !entry.iter().any(|existing| existing.name == m.name) {
                entry.push(m);
            }
        }
    }

    (
        symbols.clone(),
        Some(symbols),
        inferred_types,
        ann_discovery,
        fields,
        impl_meths,
        named_impl_names,
        cursor_offset.and_then(|offset| receiver_type_for_offset(program, offset)),
    )
}

fn extract_named_impl_names_fallback(text: &str) -> Vec<String> {
    let tokens: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();

    for i in 0..tokens.len() {
        if tokens[i] != "impl" {
            continue;
        }
        // Scan a bounded lookahead for `as Name`.
        let upper = (i + 16).min(tokens.len());
        for j in (i + 1)..upper {
            if tokens[j] == "as" && j + 1 < tokens.len() {
                out.push(tokens[j + 1].to_string());
                break;
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

/// Determine the expected type at cursor position from surrounding context
fn expected_type_at_cursor(
    text: &str,
    position: Position,
    type_context: &HashMap<String, String>,
) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let line_idx = position.line as usize;
    if line_idx >= lines.len() {
        return None;
    }
    let line = lines[line_idx];
    let char_pos = (position.character as usize).min(line.len());
    let before = &line[..char_pos];
    let trimmed = before.trim();

    // Case 1: `let x: TypeName = |` — assignment to typed variable
    // Look for `: TypeName =` pattern before cursor
    if let Some(eq_pos) = trimmed.rfind('=') {
        // Make sure self isn't `==` or `!=` or `<=` or `>=`
        let is_comparison = eq_pos > 0
            && matches!(
                trimmed.as_bytes().get(eq_pos.wrapping_sub(1)),
                Some(b'=' | b'!' | b'<' | b'>')
            )
            || matches!(trimmed.as_bytes().get(eq_pos + 1), Some(b'='));
        if !is_comparison {
            let before_eq = trimmed[..eq_pos].trim();
            if let Some(colon_pos) = before_eq.rfind(':') {
                let type_str = before_eq[colon_pos + 1..].trim();
                if !type_str.is_empty()
                    && type_str.chars().next().map_or(false, |c| c.is_alphabetic())
                {
                    return Some(normalize_type(type_str));
                }
            }
        }
    }

    // Case 2: `return |` inside a function with return type annotation
    if trimmed.starts_with("return") {
        // Walk backward to find enclosing function with return type
        for i in (0..line_idx).rev() {
            let prev_line = lines[i].trim();
            // Look for function declaration with return type
            if prev_line.starts_with("fn ") || prev_line.starts_with("function ") {
                if let Some(arrow_pos) = prev_line.rfind("->") {
                    let ret_type = prev_line[arrow_pos + 2..]
                        .trim()
                        .trim_end_matches('{')
                        .trim();
                    if !ret_type.is_empty() {
                        return Some(normalize_type(ret_type));
                    }
                }
                // Also check for `: ReturnType {` pattern
                if let Some(colon_pos) = prev_line.rfind(')') {
                    let after_paren = prev_line[colon_pos + 1..].trim();
                    if let Some(rest) = after_paren.strip_prefix(':') {
                        let ret_type = rest.trim().trim_end_matches('{').trim();
                        if !ret_type.is_empty() {
                            return Some(normalize_type(ret_type));
                        }
                    }
                }
                break;
            }
        }
    }

    // Case 3: Binary operator with known left-hand type: `x + |` where x is number
    if let Some(op_pos) = trimmed.rfind(|c: char| matches!(c, '+' | '-' | '*' | '/')) {
        let before_op = trimmed[..op_pos].trim();
        // Extract the identifier before the operator
        let ident = before_op.split_whitespace().last().unwrap_or("");
        if let Some(t) = type_context.get(ident) {
            return Some(normalize_type(t));
        }
    }

    // Case 4: Function call argument `foo(|)` or `foo(a, |)`
    // Check if we're inside parens and can identify the function
    if let Some(paren_pos) = find_unclosed_paren(trimmed) {
        let before_paren = trimmed[..paren_pos].trim();
        let func_name = before_paren
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .last()
            .unwrap_or("");
        if !func_name.is_empty() {
            // Count commas to determine argument index
            let args_text = &trimmed[paren_pos + 1..];
            let arg_idx = args_text.chars().filter(|&c| c == ',').count();
            // Look up function parameter types from metadata
            let metadata = unified_metadata();
            if let Some(func) = metadata.get_function(func_name) {
                if arg_idx < func.parameters.len() {
                    let param = &func.parameters[arg_idx];
                    if !param.param_type.is_empty() {
                        return Some(normalize_type(&param.param_type));
                    }
                }
            }
        }
    }

    None
}

/// Find the position of the last unclosed '(' before cursor
fn find_unclosed_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut last_open = None;
    for (i, c) in text.char_indices() {
        match c {
            '(' => {
                depth += 1;
                last_open = Some(i);
            }
            ')' => {
                depth -= 1;
                if depth < 0 {
                    depth = 0;
                }
            }
            _ => {}
        }
    }
    if depth > 0 { last_open } else { None }
}

/// Normalize a type name for comparison (lowercase, strip whitespace)
fn normalize_type(t: &str) -> String {
    t.trim().to_lowercase()
}

/// Check if two type names are compatible
fn types_compatible(actual: &str, expected: &str) -> TypeMatch {
    let a = actual.to_lowercase();
    let e = expected.to_lowercase();

    if a == e {
        return TypeMatch::Exact;
    }

    // Compatible numeric types
    let numeric = ["number", "int", "decimal", "float", "f64", "i64"];
    if numeric.contains(&a.as_str()) && numeric.contains(&e.as_str()) {
        return TypeMatch::Compatible;
    }

    // Unknown/inferred types match everything
    if a == "_" || e == "_" || a == "unknown" || e == "unknown" {
        return TypeMatch::Compatible;
    }

    TypeMatch::Incompatible
}

#[derive(Debug, PartialEq)]
enum TypeMatch {
    Exact,
    Compatible,
    Incompatible,
}

/// Boost completion items whose result type matches the expected type
fn boost_completions_by_type(
    completions: &mut [CompletionItem],
    expected: &str,
    type_context: &HashMap<String, String>,
) {
    for item in completions.iter_mut() {
        // Determine the result type of self completion item
        let result_type = infer_completion_result_type(item, type_context);
        let priority = match result_type {
            Some(ref t) => match types_compatible(t, expected) {
                TypeMatch::Exact => "0", // highest priority
                TypeMatch::Compatible => "1",
                TypeMatch::Incompatible => "2",
            },
            None => "2", // unknown type = lowest priority
        };

        // Prepend priority to existing sort_text (or label)
        let base = item.sort_text.as_deref().unwrap_or(&item.label);
        item.sort_text = Some(format!("{}_{}", priority, base));
    }
}

/// Infer the result type of a completion item from its metadata
fn infer_completion_result_type(
    item: &CompletionItem,
    type_context: &HashMap<String, String>,
) -> Option<String> {
    // Check the detail field for type info (e.g., "-> number", ": string")
    if let Some(detail) = &item.detail {
        // Functions: look for "-> RetType" in detail
        if let Some(arrow_pos) = detail.rfind("->") {
            let ret = detail[arrow_pos + 2..].trim();
            if !ret.is_empty() {
                return Some(normalize_type(ret));
            }
        }
        // Variables: look for ": Type" in detail
        if let Some(colon_pos) = detail.rfind(':') {
            let t = detail[colon_pos + 1..].trim();
            if !t.is_empty() && !t.contains(' ') {
                return Some(normalize_type(t));
            }
        }
    }

    // For variable completions, check type_context
    if item.kind == Some(CompletionItemKind::VARIABLE) {
        return type_context.get(&item.label).map(|t| normalize_type(t));
    }

    // For function completions, check metadata
    if item.kind == Some(CompletionItemKind::FUNCTION) {
        let metadata = unified_metadata();
        if let Some(func) = metadata.get_function(&item.label) {
            if !func.return_type.is_empty() {
                return Some(normalize_type(&func.return_type));
            }
        }
    }

    None
}

/// Get completions for inside an impl block — suggest unimplemented trait methods
fn impl_block_completions(
    text: &str,
    trait_name: &str,
    existing_methods: &[String],
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    use tower_lsp_server::ls_types::{Documentation, InsertTextFormat, MarkupContent, MarkupKind};

    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let Some(resolved_trait) = resolve_trait_definition(
        &program,
        trait_name,
        module_cache,
        current_file,
        workspace_root,
    ) else {
        // Fallback for builtin traits not defined in user source
        if trait_name == "Content" && !existing_methods.contains(&"render".to_string()) {
            return vec![CompletionItem {
                label: "render".to_string(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some("trait Content method".to_string()),
                documentation: Some(tower_lsp_server::ls_types::Documentation::MarkupContent(
                    tower_lsp_server::ls_types::MarkupContent {
                        kind: tower_lsp_server::ls_types::MarkupKind::Markdown,
                        value: "Implement `render` from trait `Content`\n\n```\nrender(self): ContentNode\n```\n\nReturn a `ContentNode` representing this value's rich content.".to_string(),
                    },
                )),
                insert_text: Some("method render(self): ContentNode {\n    Content.text(f\"${1:self}\")\n}".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..CompletionItem::default()
            }];
        }
        return Vec::new();
    };

    // Extract unimplemented methods from trait members
    let mut completions = Vec::new();
    for member in &resolved_trait.trait_def.members {
        match member {
            shape_ast::ast::TraitMember::Required(shape_ast::ast::InterfaceMember::Method {
                name,
                params,
                return_type,
                ..
            }) => {
                // Skip methods already implemented
                if existing_methods.iter().any(|m| m == name) {
                    continue;
                }

                // Build parameter list for snippet
                let param_names: Vec<String> = params
                    .iter()
                    .map(|p| p.name.clone().unwrap_or_else(|| "_".to_string()))
                    .collect();

                let return_type_str = crate::type_inference::type_annotation_to_string(return_type)
                    .unwrap_or_else(|| "_".to_string());

                // Build snippet: method name(params) { $0 }
                let snippet_params: Vec<String> = param_names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| format!("${{{}:{}}}", i + 1, n))
                    .collect();

                let snippet = format!(
                    "method {}({}) {{\n    $0\n}}",
                    name,
                    snippet_params.join(", ")
                );

                let signature =
                    format!("{}({}): {}", name, param_names.join(", "), return_type_str);

                completions.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some(format!("trait {} method", trait_name)),
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!(
                            "Implement `{}` from trait `{}`\n\n```\n{}\n```",
                            name, trait_name, signature
                        ),
                    })),
                    insert_text: Some(snippet),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                });
            }
            shape_ast::ast::TraitMember::Default(method_def) => {
                // Default methods can be overridden — suggest them with "(default)" marker
                if existing_methods.iter().any(|m| m == &method_def.name) {
                    continue;
                }

                let param_names: Vec<String> = method_def
                    .params
                    .iter()
                    .map(|p| p.simple_name().unwrap_or("_").to_string())
                    .collect();

                let return_type_str = method_def
                    .return_type
                    .as_ref()
                    .and_then(|rt| crate::type_inference::type_annotation_to_string(rt))
                    .unwrap_or_else(|| "_".to_string());

                let snippet_params: Vec<String> = param_names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| format!("${{{}:{}}}", i + 1, n))
                    .collect();

                let snippet = format!(
                    "method {}({}) {{\n    $0\n}}",
                    method_def.name,
                    snippet_params.join(", ")
                );

                let signature = format!(
                    "{}({}): {}",
                    method_def.name,
                    param_names.join(", "),
                    return_type_str
                );

                completions.push(CompletionItem {
                    label: method_def.name.clone(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some(format!("trait {} method (default)", trait_name)),
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!(
                            "Override default method `{}` from trait `{}`\n\n```\n{}\n```\n\nThis method has a default implementation.",
                            method_def.name, trait_name, signature
                        ),
                    })),
                    insert_text: Some(snippet),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                });
            }
            _ => {}
        }
    }

    completions
}

/// Get completions for comptime field overrides inside `type X = Y { | }`
fn comptime_field_override_completions(
    struct_fields: &HashMap<String, Vec<(String, String)>>,
    base_type: &str,
) -> Vec<CompletionItem> {
    use tower_lsp_server::ls_types::{Documentation, InsertTextFormat};

    let mut completions = Vec::new();

    if let Some(fields) = struct_fields.get(base_type) {
        for (name, type_str) in fields {
            // Only show comptime fields — they start with "comptime " in the type string
            if !type_str.starts_with("comptime ") {
                continue;
            }
            let snippet = format!("{}: ${{1}}", name);
            completions.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some(type_str.clone()),
                documentation: Some(Documentation::String(format!(
                    "Override comptime field `{}` of type `{}`",
                    name, base_type
                ))),
                insert_text: Some(snippet),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..CompletionItem::default()
            });
        }
    }

    completions
}

/// Get completions for trait names in bound position: `fn foo<T: |>`
fn trait_bound_completions(text: &str) -> Vec<CompletionItem> {
    use tower_lsp_server::ls_types::Documentation;

    let mut completions = Vec::new();

    // Try parsing the program first; if that fails (common when the user is
    // still typing a trait bound like `<T: >`), fall back to text-based trait
    // name extraction so completions still work for incomplete code.
    if let Ok(program) = parse_program(text) {
        for item in &program.items {
            if let shape_ast::ast::Item::Trait(trait_def, _) = item {
                completions.push(CompletionItem {
                    label: trait_def.name.clone(),
                    kind: Some(CompletionItemKind::INTERFACE),
                    detail: Some("trait".to_string()),
                    documentation: Some(Documentation::String(format!(
                        "Trait `{}`",
                        trait_def.name
                    ))),
                    ..CompletionItem::default()
                });
            }
        }
    }

    // Fallback: scan text for `trait <Name> {` declarations
    if completions.is_empty() {
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("trait ") {
                if let Some(name) = rest.split_whitespace().next() {
                    // Avoid duplicates from the parsed path
                    let name = name.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
                    if !name.is_empty() {
                        completions.push(CompletionItem {
                            label: name.to_string(),
                            kind: Some(CompletionItemKind::INTERFACE),
                            detail: Some("trait".to_string()),
                            documentation: Some(Documentation::String(format!("Trait `{}`", name))),
                            ..CompletionItem::default()
                        });
                    }
                }
            }
        }
    }

    completions
}

/// Get all completions (keywords + built-ins + user symbols)
fn all_completions(user_symbols: &[crate::symbols::SymbolInfo]) -> Vec<CompletionItem> {
    let mut completions = Vec::new();

    // Add user-defined symbols first (higher priority)
    completions.extend(symbols_to_completions(user_symbols));

    // Add keyword completions from metadata
    completions.extend(keyword_completions());

    // Add built-in function completions from metadata
    completions.extend(builtin_function_completions());

    completions
}

fn dedupe_completion_items(items: &mut Vec<CompletionItem>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.label.clone()));
}

// extract_struct_fields is imported from crate::type_inference (shared module)

/// Get completions for join strategies after `await join `
fn join_strategy_completions() -> Vec<CompletionItem> {
    vec![
        CompletionItem {
            label: "all".to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("Join strategy".to_string()),
            documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                "Wait for all branches to complete. Returns a tuple of all results.".to_string(),
            )),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "race".to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("Join strategy".to_string()),
            documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                "Return the first branch to complete, cancel the rest.".to_string(),
            )),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "any".to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("Join strategy".to_string()),
            documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                "Return the first branch to succeed (non-error), cancel the rest.".to_string(),
            )),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "settle".to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("Join strategy".to_string()),
            documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                "Wait for all branches, preserving individual success/error results.".to_string(),
            )),
            ..CompletionItem::default()
        },
    ]
}

/// Get completions for inside a join block body — labeled branch snippets
fn join_branch_completions(strategy: &str) -> Vec<CompletionItem> {
    use tower_lsp_server::ls_types::{Documentation, InsertTextFormat};

    let strategy_hint = match strategy {
        "all" => "all branches must complete",
        "race" => "first to complete wins",
        "any" => "first to succeed wins",
        "settle" => "all branches settle (success or error)",
        _ => "concurrent branch",
    };

    vec![
        CompletionItem {
            label: "label: expr".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(format!("Named branch ({})", strategy_hint)),
            documentation: Some(Documentation::String(
                "Add a labeled branch to the join expression.\nLabels enable named access to results.".to_string(),
            )),
            insert_text: Some("${1:name}: ${2:expr}".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "@annotation branch".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some("Annotated branch".to_string()),
            documentation: Some(Documentation::String(
                "Add an annotated branch with per-branch configuration.\nExample: @timeout(5s) fetch_data()".to_string(),
            )),
            insert_text: Some("@${1:annotation} ${2:expr}".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        },
    ]
}

fn interpolation_format_spec_completions(spec_prefix: &str) -> Vec<CompletionItem> {
    use tower_lsp_server::ls_types::{Documentation, InsertTextFormat};

    let mut items = vec![
        CompletionItem {
            label: "fixed(2)".to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("Numeric fixed precision format".to_string()),
            documentation: Some(Documentation::String(
                "Format numeric values with fixed precision.\nExample: `f\"{price:fixed(2)}\"`"
                    .to_string(),
            )),
            insert_text: Some("fixed(${1:2})".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "table(...)".to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("Typed table formatting".to_string()),
            documentation: Some(Documentation::String(
                "Render table values with typed options (no stringly keys).\nExample: `table(max_rows=20, align=right, precision=2, border=on)`".to_string(),
            )),
            insert_text: Some(
                "table(max_rows=${1:20}, align=${2:right}, precision=${3:2}, border=${4:on})"
                    .to_string(),
            ),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        },
    ];

    let trimmed = spec_prefix.trim_start();
    if let Some(table_inner) = trimmed.strip_prefix("table(") {
        items.extend(table_format_argument_completions(table_inner));
    }

    items
}

fn table_format_argument_completions(table_inner: &str) -> Vec<CompletionItem> {
    use tower_lsp_server::ls_types::Documentation;

    let mut items = Vec::new();
    let trailing = table_inner.rsplit(',').next().unwrap_or("").trim_start();

    let push_value = |label: &str, detail: &str, doc: &str| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::ENUM_MEMBER),
        detail: Some(detail.to_string()),
        documentation: Some(Documentation::String(doc.to_string())),
        ..CompletionItem::default()
    };

    if let Some((key, value_prefix)) = trailing.split_once('=') {
        let key = key.trim();
        let value_prefix = value_prefix.trim();

        let mut push_if_matches = |candidate: CompletionItem| {
            if value_prefix.is_empty() || candidate.label.starts_with(value_prefix) {
                items.push(candidate);
            }
        };

        match key {
            "align" => {
                push_if_matches(push_value("left", "Alignment", "Left-aligned cells."));
                push_if_matches(push_value("center", "Alignment", "Center-aligned cells."));
                push_if_matches(push_value("right", "Alignment", "Right-aligned cells."));
            }
            "color" => {
                for color in [
                    "default", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
                ] {
                    push_if_matches(push_value(color, "Color", "Table color hint."));
                }
            }
            "border" => {
                push_if_matches(push_value("on", "Border", "Render table borders."));
                push_if_matches(push_value("off", "Border", "Render borderless table."));
            }
            _ => {}
        }

        return items;
    }

    // Key position completions.
    let key_prefix = trailing.trim();
    let key_item = |label: &str, detail: &str| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::PROPERTY),
        detail: Some(detail.to_string()),
        ..CompletionItem::default()
    };

    for (key, detail) in [
        ("max_rows=", "Maximum number of rendered rows"),
        ("align=", "Global cell alignment (left|center|right)"),
        ("precision=", "Numeric precision for float columns"),
        ("color=", "Optional color hint"),
        ("border=", "Border mode (on|off)"),
    ] {
        if key_prefix.is_empty() || key.starts_with(key_prefix) {
            items.push(key_item(key, detail));
        }
    }

    items
}

fn receiver_type_for_offset(program: &Program, offset: usize) -> Option<String> {
    for item in &program.items {
        match item {
            Item::Impl(impl_block, span) => {
                if !span_contains_offset(*span, offset) {
                    continue;
                }
                if impl_block
                    .methods
                    .iter()
                    .any(|m| method_body_contains_offset(m, offset))
                {
                    return Some(type_name_base_name(&impl_block.target_type));
                }
            }
            Item::Extend(extend_stmt, span) => {
                if !span_contains_offset(*span, offset) {
                    continue;
                }
                if extend_stmt
                    .methods
                    .iter()
                    .any(|m| method_body_contains_offset(m, offset))
                {
                    return Some(type_name_base_name(&extend_stmt.type_name));
                }
            }
            _ => {}
        }
    }
    None
}

fn method_body_contains_offset(method: &MethodDef, offset: usize) -> bool {
    method.body.iter().any(|stmt| {
        let span = match stmt {
            Statement::Return(_, span)
            | Statement::Break(span)
            | Statement::Continue(span)
            | Statement::VariableDecl(_, span)
            | Statement::Assignment(_, span)
            | Statement::Expression(_, span)
            | Statement::For(_, span)
            | Statement::While(_, span)
            | Statement::If(_, span)
            | Statement::Extend(_, span)
            | Statement::RemoveTarget(span)
            | Statement::SetParamType { span, .. }
            | Statement::SetParamValue { span, .. }
            | Statement::SetReturnType { span, .. }
            | Statement::SetReturnExpr { span, .. }
            | Statement::ReplaceBodyExpr { span, .. }
            | Statement::ReplaceBody { span, .. }
            | Statement::ReplaceModuleExpr { span, .. } => *span,
        };
        span_contains_offset(span, offset)
    })
}

fn type_name_base_name(type_name: &TypeName) -> String {
    match type_name {
        TypeName::Simple(name) => name.to_string(),
        TypeName::Generic { name, .. } => name.to_string(),
    }
}

fn span_contains_offset(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn completions_for(code: &str, position: Position) -> Vec<CompletionItem> {
        let (completions, _, _) = get_completions(code, position, &[], &HashMap::new());
        completions
    }

    #[test]
    fn test_keyword_completions() {
        let keywords = keyword_completions();
        assert!(!keywords.is_empty());

        // Check that we have core language keywords (from metadata API)
        let labels: Vec<_> = keywords.iter().map(|k| k.label.as_str()).collect();
        assert!(labels.contains(&"let"));
        assert!(labels.contains(&"const"));
        assert!(labels.contains(&"fn"));
        assert!(
            !labels.contains(&"function"),
            "Legacy alias should not be globally suggested"
        );
        // Note: "pattern" and "strategy" are user-defined annotation names, not language keywords
        // Note: Domain-specific keywords like "backtest" are now in stdlib, not core keywords
    }

    #[test]
    fn test_builtin_functions() {
        let functions = builtin_function_completions();
        assert!(!functions.is_empty());

        // Check for core builtins (generic functions, not domain-specific)
        let labels: Vec<_> = functions.iter().map(|f| f.label.as_str()).collect();
        // Note: Domain-specific functions like sma, ema, rsi, run_simulation are now in stdlib
        // Core builtins include math, utility, and time functions
        assert!(
            labels.contains(&"abs") || labels.contains(&"sqrt") || labels.contains(&"print"),
            "Should include core builtin functions"
        );
    }

    #[test]
    fn test_get_completions() {
        let completions = completions_for(
            "",
            Position {
                line: 0,
                character: 0,
            },
        );

        // Should return all completions (keywords + functions + snippets)
        assert!(completions.len() > 50);
    }

    #[test]
    fn test_dynamic_variable_completion() {
        let code = r#"let myVar = 5;
const MY_CONST = 10;

"#;
        let position = Position {
            line: 3,
            character: 0,
        };
        let completions = completions_for(code, position);

        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"myVar"),
            "Should include user-defined variable"
        );
        assert!(
            labels.contains(&"MY_CONST"),
            "Should include user-defined constant"
        );
    }

    #[test]
    fn test_dynamic_function_completion() {
        let code = r#"function myFunction(x, y) {
    return x + y;
}

"#;
        let position = Position {
            line: 4,
            character: 0,
        };
        let completions = completions_for(code, position);

        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"myFunction"),
            "Should include user-defined function"
        );
    }

    #[test]
    fn test_doc_tag_completion() {
        let code = "/// @pa\nfn add(x: number) -> number { x }\n";
        let position = Position {
            line: 0,
            character: 6,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"param"));
    }

    #[test]
    fn test_doc_param_completion_uses_attached_function_params() {
        let code = "/// Summary.\n/// @param va\nfn add(value: number, scale: number) -> number { value * scale }\n";
        let position = Position {
            line: 1,
            character: 13,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"value"));
        assert!(!labels.contains(&"scale"));
    }

    #[test]
    fn test_interpolation_format_spec_completions_include_fixed_and_table() {
        let code = r#"let s = f"value: {price:f}""#;
        let pos = Position {
            line: 0,
            character: code.find("{price:f").unwrap() as u32 + 8,
        };
        let items = completions_for(code, pos);
        let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"fixed(2)"),
            "expected fixed completion, got {:?}",
            labels
        );
        assert!(
            labels.contains(&"table(...)"),
            "expected table completion, got {:?}",
            labels
        );
    }

    #[test]
    fn test_interpolation_table_align_value_completions() {
        let code = r#"let s = f"{rows:table(align=)}""#;
        let pos = Position {
            line: 0,
            character: code.find("align=").unwrap() as u32 + 6,
        };
        let items = completions_for(code, pos);
        let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"left") && labels.contains(&"right"),
            "expected alignment value completions, got {:?}",
            labels
        );
    }

    #[test]
    fn test_dynamic_pattern_completion() {
        let code = r#"function myPattern(candle) {
    return candle.close > candle.open;
}

let x = 1
"#;
        let position = Position {
            line: 5,
            character: 0,
        };
        let completions = completions_for(code, position);

        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"myPattern"),
            "Should include user-defined pattern. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_property_completion_with_typed_variable() {
        // Row properties are dynamic based on schema - test with an explicitly typed variable
        let code = "type Point { x: number, y: number }\nlet p: Point\np.x\n";
        let position = Position {
            line: 2,
            character: 2,
        };
        let completions = completions_for(code, position);

        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"x"),
            "Should include Point field 'x'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"y"),
            "Should include Point field 'y'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_property_completion_for_self_inside_dollar_interpolation() {
        let code = "type User { name: String }\nimpl Display for User as JsonDisplay {\n  method display() { f$\"\"\"{ \"name\": \"${self.na}\" }\"\"\" }\n}\n";
        let cursor_offset = code
            .find("self.na")
            .expect("expected self property access in interpolation")
            + "self.".len();
        let position = crate::util::offset_to_position(code, cursor_offset);
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"name"),
            "Expected `name` completion for self receiver in interpolation, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_struct_chained_property_completion() {
        // Test struct chained property completion with user-defined types
        let code = "type Summary { total: number, ratio: number }\ntype Result { summary: Summary }\nlet bt: Result\nbt.summary.x\n";
        let position = Position {
            line: 3,
            character: 11,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"total"),
            "Should include Summary field 'total'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"ratio"),
            "Should include Summary field 'ratio'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_user_defined_type_property_completion() {
        // Test property completion for a user-defined struct type
        let code =
            "type Item { name: string, price: number, quantity: number }\nlet item: Item\nitem.x\n";
        let position = Position {
            line: 2,
            character: 5,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"name"),
            "Should include Item field 'name'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"price"),
            "Should include Item field 'price'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"quantity"),
            "Should include Item field 'quantity'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_pattern_reference_completion() {
        let code = r#"function hammer(candle) {
    return candle.close > candle.open;
}

let x = 1
"#;
        // Test that pattern shows up in general completions
        let position = Position {
            line: 5,
            character: 0,
        };
        let completions = completions_for(code, position);

        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        // Should show user-defined pattern in completions
        assert!(
            labels.contains(&"hammer"),
            "Should include user-defined pattern 'hammer'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_metadata_keywords() {
        // Verify metadata API returns keywords
        let keywords = shape_runtime::metadata::LanguageMetadata::keywords();
        assert!(!keywords.is_empty());
        assert!(keywords.iter().any(|k| k.keyword == "let"));
        assert!(keywords.iter().any(|k| k.keyword == "function"));
    }

    #[test]
    fn test_metadata_functions() {
        // Verify unified metadata API returns functions
        let metadata = unified_metadata();
        let functions = metadata.all_functions();
        assert!(!functions.is_empty(), "Should have some functions loaded");
        // Check for either core builtins OR stdlib functions
        // (stdlib may not be loaded in test environment)
        let has_builtins = functions.iter().any(|f| {
            f.name == "abs"
                || f.name == "sqrt"
                || f.name == "print"
                || f.name == "sma"
                || f.name == "rsi"
        });
        assert!(
            has_builtins,
            "Should include either core builtins or stdlib functions"
        );
    }

    #[test]
    fn test_metadata_types() {
        // Verify metadata API returns types
        let types = shape_runtime::metadata::LanguageMetadata::builtin_types();
        assert!(!types.is_empty());
        assert!(types.iter().any(|t| t.name == "Number"));
        assert!(types.iter().any(|t| t.name == "Table"));
    }

    #[test]
    fn test_metadata_row_properties() {
        // Verify unified metadata returns row properties
        // Row now has no hardcoded properties - they're dynamic based on data schema
        let meta = unified_metadata();
        let props = meta
            .get_type_properties("Row")
            .expect("Row type should exist");
        assert_eq!(
            props.len(),
            0,
            "Row should have no hardcoded properties - fields are dynamic"
        );
    }

    #[test]
    fn test_metadata_column_methods() {
        // Verify metadata API returns column methods
        let methods = shape_runtime::metadata::LanguageMetadata::column_methods();
        assert!(!methods.is_empty());
        assert!(methods.iter().any(|m| m.name == "shift"));
        assert!(methods.iter().any(|m| m.name == "filter")); // Generic filtering method
    }

    #[test]
    fn test_result_type_completions() {
        // Test that Result<Instrument> provides Result methods, NOT Instrument properties
        let code = "instr.";
        let position = Position {
            line: 0,
            character: 6,
        };
        // Provide type context that 'instr' is of type Result<Instrument>
        let mut type_context = HashMap::new();
        type_context.insert("instr".to_string(), "Result<Instrument>".to_string());
        let (completions, _, _) = get_completions(code, position, &[], &type_context);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();

        // Should have Result methods
        assert!(
            labels.contains(&"unwrap"),
            "Should include Result method 'unwrap'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"unwrap_or"),
            "Should include Result method 'unwrap_or'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"is_ok"),
            "Should include Result method 'is_ok'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"is_err"),
            "Should include Result method 'is_err'. Got: {:?}",
            labels
        );

        // Should NOT have Instrument properties (those are on the inner type)
        assert!(
            !labels.contains(&"symbol"),
            "Should NOT include Instrument property 'symbol' on Result type. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_option_type_completions() {
        // Test that Option<T> provides Option methods
        let code = "opt.";
        let position = Position {
            line: 0,
            character: 4,
        };
        // Provide type context that 'opt' is of type Option<Number>
        let mut type_context = HashMap::new();
        type_context.insert("opt".to_string(), "Option<Number>".to_string());
        let (completions, _, _) = get_completions(code, position, &[], &type_context);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();

        // Should have Option methods
        assert!(
            labels.contains(&"unwrap"),
            "Should include Option method 'unwrap'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"is_some"),
            "Should include Option method 'is_some'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"is_none"),
            "Should include Option method 'is_none'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_new_keywords_in_completions() {
        let keywords = keyword_completions();
        let labels: Vec<_> = keywords.iter().map(|k| k.label.as_str()).collect();
        // Verify actively supported general-scope keywords appear
        assert!(labels.contains(&"match"), "Should include 'match' keyword");
        assert!(labels.contains(&"try"), "Should include 'try' keyword");
        assert!(
            !labels.contains(&"stream"),
            "Deprecated/placeholder keyword should not be globally suggested"
        );
    }

    #[test]
    fn test_using_impl_selector_context_detection() {
        let code = "let x = value using ";
        let position = Position {
            line: 0,
            character: 20,
        };
        assert!(is_using_impl_selector_context(code, position));
    }

    #[test]
    fn test_using_impl_selector_completions() {
        let code = "trait Display { display(self): string }\n\
                    type User { name: string }\n\
                    impl Display for User as JsonDisplay {\n\
                        method display() { \"json\" }\n\
                    }\n\
                    let u = User { name: \"a\" }\n\
                    print(u using )\n";
        let position = Position {
            line: 6,
            character: 14,
        };
        let (completions, _, _) = get_completions(code, position, &[], &HashMap::new());
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"JsonDisplay"),
            "Expected named impl selector completion. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_keyword_descriptions_have_examples() {
        let keywords = shape_runtime::metadata::LanguageMetadata::keywords();
        // Key keywords should have code examples in their description
        let type_kw = keywords
            .iter()
            .find(|k| k.keyword == "type")
            .expect("type keyword");
        assert!(
            type_kw.description.contains("type Point"),
            "'type' should have struct example"
        );

        let comptime_kw = keywords
            .iter()
            .find(|k| k.keyword == "comptime")
            .expect("comptime keyword");
        assert!(
            comptime_kw.description.contains("comptime symbol"),
            "'comptime' should have example"
        );

        let match_kw = keywords
            .iter()
            .find(|k| k.keyword == "match")
            .expect("match keyword");
        assert!(
            match_kw.description.contains("match color"),
            "'match' should have example"
        );

        let for_kw = keywords
            .iter()
            .find(|k| k.keyword == "for")
            .expect("for keyword");
        assert!(
            for_kw.description.contains("for x in"),
            "'for' should have example"
        );
    }

    #[test]
    fn test_builtin_function_metadata_has_signatures() {
        let metadata = unified_metadata();
        let abs_fn = metadata.get_function("abs").expect("abs should exist");
        assert!(
            abs_fn.signature.contains("abs"),
            "abs signature should contain function name"
        );
        assert!(
            abs_fn.signature.contains("number"),
            "abs signature should mention number type"
        );
        assert!(
            !abs_fn.description.is_empty(),
            "abs should have a description"
        );
        assert_eq!(abs_fn.parameters.len(), 1, "abs should have 1 parameter");

        let print_fn = metadata.get_function("print").expect("print should exist");
        assert!(
            print_fn.signature.contains("print"),
            "print signature should contain name"
        );

        let max_fn = metadata.get_function("max").expect("max should exist");
        assert_eq!(max_fn.parameters.len(), 2, "max should have 2 parameters");
    }

    #[test]
    fn test_snippets_have_correct_format() {
        use tower_lsp_server::ls_types::InsertTextFormat;
        let snippets = snippet_completions();
        for snippet in &snippets {
            assert_eq!(
                snippet.insert_text_format,
                Some(InsertTextFormat::SNIPPET),
                "Snippet '{}' should have SNIPPET insert format",
                snippet.label
            );
            assert!(
                snippet.insert_text.is_some(),
                "Snippet '{}' should have insert text",
                snippet.label
            );
        }
    }

    #[test]
    fn test_user_defined_struct_field_completions() {
        // Regression test: user-defined struct types should show their fields,
        // not the generic "length" fallback.
        // Use parseable code (b.i is valid) with cursor positioned after the dot.
        let code = "type MyType { i: int, name: string }\nlet b = MyType { i: 10, name: \"hello\" }\nb.i\n";
        let position = Position {
            line: 2,
            character: 2, // cursor after "b."
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();

        assert!(
            labels.contains(&"i"),
            "Should include struct field 'i'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"name"),
            "Should include struct field 'name'. Got: {:?}",
            labels
        );
        assert!(
            !labels.contains(&"length"),
            "Should NOT show generic 'length' fallback for typed struct. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_unwrapped_type_has_inner_properties() {
        // Test that a user-defined struct shows its fields (not Result methods)
        let code = "type Device { name: string, status: string }\nlet dev: Device\ndev.x\n";
        let position = Position {
            line: 2,
            character: 4,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();

        // Should have Device fields, NOT Result methods
        assert!(
            labels.contains(&"name"),
            "Device should include 'name'. Got: {:?}",
            labels
        );
        assert!(
            !labels.contains(&"is_ok"),
            "Device should NOT include Result method 'is_ok'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_impl_method_completions() {
        // Use parseable code (t.x is valid) with cursor positioned after "t."
        let code = "trait Q {\n    filter(p): any;\n    select(c): any\n}\nimpl Q for T {\n    method filter(p) { self }\n}\nlet t: T\nt.x\n";
        let position = Position {
            line: 8,
            character: 2, // cursor after "t."
        };
        let mut type_context = HashMap::new();
        type_context.insert("t".to_string(), "T".to_string());
        let (completions, _, _) = get_completions(code, position, &[], &type_context);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"filter"),
            "Should include 'filter' from impl. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"select"),
            "Should include 'select' from trait (via impl). Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_extend_method_completions() {
        // Use parseable code (a.x is valid) with cursor positioned after "a."
        let code =
            "extend Array {\n    method double() {\n        self\n    }\n}\nlet a: Array\na.x\n";
        let position = Position {
            line: 6,
            character: 2, // cursor after "a."
        };
        let mut type_context = HashMap::new();
        type_context.insert("a".to_string(), "Array".to_string());
        let (completions, _, _) = get_completions(code, position, &[], &type_context);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"double"),
            "Should include 'double' from extend block. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_property_completions_chained_struct() {
        // Multi-level chain: o.inner. should show Inner's fields
        let code = "type Outer { inner: Inner }\ntype Inner { val: number }\nlet o = Outer { inner: Inner { val: 1 } }\no.inner.x\n";
        let position = Position {
            line: 3,
            character: 8, // cursor after "o.inner."
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"val"),
            "Should include 'val' from Inner struct. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_string_method_completions() {
        // String-specific methods (toLowerCase, split, etc.) are now registered
        // from Shape stdlib (stdlib-src/core/string_methods.shape) during
        // compilation, not at MethodTable::new() time. The universal methods
        // (toString, type) are always available.
        let code = "let s = \"hi\"\ns.x\n";
        let position = Position {
            line: 1,
            character: 2, // cursor after "s."
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        // Universal methods are always registered
        assert!(
            labels.contains(&"toString"),
            "Should include universal method 'toString'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"type"),
            "Should include universal method 'type'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_number_method_completions() {
        // Number-specific methods (abs, floor, etc.) are now registered from
        // Shape stdlib during compilation. Universal methods are always present.
        let code = "let n = 42\nn.x\n";
        let position = Position {
            line: 1,
            character: 2,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"toString"),
            "Should include universal method 'toString'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_array_method_completions() {
        // Array-specific methods (map, filter, etc.) are now registered from
        // Shape stdlib (stdlib-src/core/vec.shape) during compilation.
        // Universal methods are always present.
        let code = "let a = [1, 2]\na.x\n";
        let position = Position {
            line: 1,
            character: 2,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"toString"),
            "Should include universal method 'toString'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"type"),
            "Should include universal method 'type'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_closure_param_completions_with_struct() {
        // t.filter(|c| c.) should show Candle's fields for c
        let code = "type Candle { open: number, close: number }\nlet t: Table<Candle>\nt.filter(|c| c.x)\n";
        let position = Position {
            line: 2,
            character: 16, // cursor after "c."
        };
        let mut type_context = HashMap::new();
        type_context.insert("t".to_string(), "Table<Candle>".to_string());
        let (completions, _, _) = get_completions(code, position, &[], &type_context);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"open"),
            "Should include 'open' from Candle struct. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"close"),
            "Should include 'close' from Candle struct. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_pipe_target_completions() {
        // `let a = [1, 2]; a |> ` should show array-compatible functions
        let code = "let a = [1, 2]\na |> ";
        let position = Position {
            line: 1,
            character: 5,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        // Should include common pipe-friendly functions
        assert!(
            labels.contains(&"map"),
            "Pipe target should include 'map'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"filter"),
            "Pipe target should include 'filter'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_pipe_chain_type_tracking() {
        // After pipe chain, variable should still have universal methods at minimum.
        // Array-specific methods (from Shape stdlib) are available at full compilation time.
        let code = "let a = [1]\nlet b = a\nb.x\n";
        let position = Position {
            line: 2,
            character: 2,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            !labels.is_empty(),
            "Should have some completions for b. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_impl_block_completions_suggests_unimplemented() {
        let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any;\n    orderBy(col): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}\n";
        let completions =
            impl_block_completions(code, "Queryable", &["filter".to_string()], None, None, None);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"select"),
            "Should suggest unimplemented 'select'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"orderBy"),
            "Should suggest unimplemented 'orderBy'. Got: {:?}",
            labels
        );
        assert!(
            !labels.contains(&"filter"),
            "Should NOT suggest already-implemented 'filter'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_impl_block_completions_empty_when_all_implemented() {
        let code = "trait Simple {\n    foo(): any\n}\nimpl Simple for Bar {\n    method foo() { self }\n}\n";
        let completions =
            impl_block_completions(code, "Simple", &["foo".to_string()], None, None, None);
        assert!(
            completions.is_empty(),
            "Should have no completions when all methods implemented"
        );
    }

    #[test]
    fn test_impl_block_completions_unknown_trait_returns_empty() {
        let code = "let x = 42\n";
        let completions = impl_block_completions(code, "NonExistent", &[], None, None, None);
        assert!(
            completions.is_empty(),
            "Should return empty for unknown trait"
        );
    }

    #[test]
    fn test_impl_block_completions_snippet_format() {
        let code = "trait Filt {\n    filter(pred): any\n}\n";
        let completions = impl_block_completions(code, "Filt", &[], None, None, None);
        assert_eq!(completions.len(), 1);
        let item = &completions[0];
        assert_eq!(item.label, "filter");
        // Should have snippet format
        assert_eq!(
            item.insert_text_format,
            Some(tower_lsp_server::ls_types::InsertTextFormat::SNIPPET)
        );
        // Snippet should contain method keyword and parameter placeholder
        let snippet = item.insert_text.as_ref().unwrap();
        assert!(
            snippet.starts_with("method filter("),
            "Snippet should start with 'method filter('. Got: {}",
            snippet
        );
    }

    #[test]
    fn test_impl_block_completions_resolve_trait_from_modules() {
        let code = "type User { name: String }\nimpl Display for User {\n    \n}\n";
        let position = Position {
            line: 2,
            character: 4,
        };
        let cache = ModuleCache::new();
        let current_file = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("__shape_lsp_impl_completion_test__.shape");

        let (completions, _, _) = get_completions_with_context(
            code,
            position,
            &[],
            &HashMap::new(),
            Some(&cache),
            Some(current_file.as_path()),
            None,
        );
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"display"),
            "Expected Display trait method completion from module resolution, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_comptime_field_override_completions() {
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "Currency".to_string(),
            vec![
                ("symbol".to_string(), "comptime string = \"$\"".to_string()),
                ("decimals".to_string(), "comptime number = 2".to_string()),
                ("amount".to_string(), "number".to_string()),
            ],
        );
        let completions = comptime_field_override_completions(&struct_fields, "Currency");
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels.len(), 2, "Should only show comptime fields");
        assert!(
            labels.contains(&"symbol"),
            "Should include comptime field 'symbol'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"decimals"),
            "Should include comptime field 'decimals'. Got: {:?}",
            labels
        );
        assert!(
            !labels.contains(&"amount"),
            "Should NOT include runtime field 'amount'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_comptime_field_override_completions_integration() {
        // Full integration: typing inside `type EUR = Currency { | }` should suggest comptime fields.
        // The code must parse: use valid `meta_param_override` syntax (ident: expr).
        let code = "type Currency { comptime symbol: string = \"$\", comptime decimals: number = 2, amount: number }\ntype EUR = Currency { symbol: \"E\" }";
        // Cursor after "{ " on line 1 — position is inside the override braces
        let position = Position {
            line: 1,
            character: 22,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"symbol"),
            "Should suggest comptime field 'symbol'. Got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"decimals"),
            "Should suggest comptime field 'decimals'. Got: {:?}",
            labels
        );
        assert!(
            !labels.contains(&"amount"),
            "Should NOT suggest runtime field 'amount'. Got: {:?}",
            labels
        );
    }

    #[test]
    fn test_join_strategy_completions() {
        let completions = join_strategy_completions();
        assert_eq!(
            completions.len(),
            4,
            "Should have 4 join strategy completions"
        );

        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"all"));
        assert!(labels.contains(&"race"));
        assert!(labels.contains(&"any"));
        assert!(labels.contains(&"settle"));

        // All should be KEYWORD kind
        for item in &completions {
            assert_eq!(item.kind, Some(CompletionItemKind::KEYWORD));
        }
    }

    #[test]
    fn test_join_strategy_completions_integration() {
        // After "await join " the context should be JoinStrategy
        let code = "async fn foo() {\n  await join ";
        let position = Position {
            line: 1,
            character: 13,
        };
        let context = crate::context::analyze_context(code, position);
        assert_eq!(
            context,
            crate::context::CompletionContext::JoinStrategy,
            "Should detect JoinStrategy context"
        );
    }

    #[test]
    fn test_async_keywords_in_completions() {
        let keywords = keyword_completions();
        let labels: Vec<_> = keywords.iter().map(|k| k.label.as_str()).collect();
        assert!(labels.contains(&"await"), "Should include 'await' keyword");
        assert!(labels.contains(&"async"), "Should include 'async' keyword");
        assert!(
            !labels.contains(&"join"),
            "join should only appear in `await join` context completions"
        );
    }

    #[test]
    fn test_join_branch_completions() {
        let completions = join_branch_completions("all");
        assert_eq!(completions.len(), 2);
        assert!(
            completions.iter().any(|c| c.label.contains("label")),
            "Should include labeled branch snippet"
        );
        assert!(
            completions.iter().any(|c| c.label.contains("annotation")),
            "Should include annotated branch snippet"
        );
    }

    #[test]
    fn test_join_body_completions_integration() {
        // Inside a join all block, should get branch snippets + general completions
        let code = "async fn foo() {\n  await join all {\n    ";
        let position = Position {
            line: 2,
            character: 4,
        };
        let completions = completions_for(code, position);
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("label")),
            "Should include labeled branch snippet in join body, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_type_aware_completion_typed_assignment() {
        // `let x: number = |` — completions producing numbers should be boosted
        let code = "let n = 42\nlet s = \"hello\"\nlet x: number = ";
        let position = Position {
            line: 2,
            character: 16,
        };
        let mut type_context = HashMap::new();
        type_context.insert("n".to_string(), "number".to_string());
        type_context.insert("s".to_string(), "string".to_string());
        let (completions, _, _) = get_completions(code, position, &[], &type_context);

        // All completions should have sort_text set
        let with_sort: Vec<_> = completions
            .iter()
            .filter(|c| c.sort_text.is_some())
            .collect();
        assert!(!with_sort.is_empty(), "expected sort_text on completions");

        // Variable 'n' (number) should sort before 's' (string)
        let n_sort = completions
            .iter()
            .find(|c| c.label == "n")
            .and_then(|c| c.sort_text.as_deref());
        let s_sort = completions
            .iter()
            .find(|c| c.label == "s")
            .and_then(|c| c.sort_text.as_deref());
        if let (Some(n_s), Some(s_s)) = (n_sort, s_sort) {
            assert!(
                n_s < s_s,
                "number var 'n' should sort before string var 's'. n={}, s={}",
                n_s,
                s_s
            );
        }
    }

    #[test]
    fn test_expected_type_from_typed_assignment() {
        let code = "let x: number = ";
        let position = Position {
            line: 0,
            character: 16,
        };
        let type_context = HashMap::new();
        let expected = expected_type_at_cursor(code, position, &type_context);
        assert_eq!(expected, Some("number".to_string()));
    }

    #[test]
    fn test_expected_type_from_return() {
        let code = "fn foo() -> number {\n  return ";
        let position = Position {
            line: 1,
            character: 9,
        };
        let type_context = HashMap::new();
        let expected = expected_type_at_cursor(code, position, &type_context);
        assert_eq!(expected, Some("number".to_string()));
    }

    #[test]
    fn test_expected_type_from_binary_op() {
        let code = "let x = n + ";
        let position = Position {
            line: 0,
            character: 12,
        };
        let mut type_context = HashMap::new();
        type_context.insert("n".to_string(), "number".to_string());
        let expected = expected_type_at_cursor(code, position, &type_context);
        assert_eq!(expected, Some("number".to_string()));
    }

    #[test]
    fn test_types_compatible_exact() {
        assert_eq!(types_compatible("number", "number"), TypeMatch::Exact);
        assert_eq!(types_compatible("string", "string"), TypeMatch::Exact);
    }

    #[test]
    fn test_types_compatible_numeric() {
        assert_eq!(types_compatible("int", "number"), TypeMatch::Compatible);
        assert_eq!(types_compatible("decimal", "number"), TypeMatch::Compatible);
    }

    #[test]
    fn test_types_compatible_incompatible() {
        assert_eq!(
            types_compatible("string", "number"),
            TypeMatch::Incompatible
        );
        assert_eq!(types_compatible("bool", "number"), TypeMatch::Incompatible);
    }
}
