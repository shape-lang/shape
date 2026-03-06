//! Shared semantic analysis pipeline for Shape LSP.
//!
//! This module centralizes semantic diagnostics so the language server and
//! the `shape-test` fluent harness use the same logic.

use crate::annotation_discovery::AnnotationDiscovery;
use crate::diagnostics::{
    error_to_diagnostic, validate_annotations, validate_async_join,
    validate_async_structured_concurrency, validate_comptime_builtins_context,
    validate_comptime_overrides, validate_comptime_side_effects, validate_content_strings,
    validate_foreign_function_types, validate_interpolation_format_specs, validate_trait_bounds,
};
use crate::module_cache::ModuleCache;
use crate::scope::ScopeTree;
use crate::util::offset_to_line_col;
use shape_ast::ast::{Expr, ImportItems, Item, Program};
use shape_runtime::visitor::{Visitor, walk_program};
use std::collections::{HashMap, HashSet};
use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, Position, Range};

const MAX_SEMANTIC_DIAGNOSTICS: usize = 200;

/// Run semantic diagnostics for a parsed Shape program.
pub fn analyze_program_semantics(
    program: &Program,
    text: &str,
    file_path: Option<&std::path::Path>,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&std::path::Path>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Discover annotations from the program and imports.
    let mut annotation_discovery = AnnotationDiscovery::new();
    annotation_discovery.discover_from_program(program);
    if let (Some(path), Some(cache)) = (file_path, module_cache) {
        annotation_discovery.discover_from_imports_with_cache(program, path, cache, workspace_root);
    } else {
        annotation_discovery.discover_from_imports(program);
    }

    diagnostics.extend(validate_annotations(program, &annotation_discovery, text));
    diagnostics.extend(validate_async_join(program, text));
    diagnostics.extend(validate_async_structured_concurrency(program, text));
    diagnostics.extend(validate_interpolation_format_specs(program, text));
    diagnostics.extend(validate_comptime_overrides(program, text));
    diagnostics.extend(validate_comptime_side_effects(program, text));
    diagnostics.extend(validate_comptime_builtins_context(program, text));
    diagnostics.extend(validate_trait_bounds(program, text));
    diagnostics.extend(validate_content_strings(program, text));
    diagnostics.extend(validate_foreign_function_types(program, text));

    let mut compiler = shape_vm::BytecodeCompiler::new();
    compiler.set_type_diagnostic_mode(shape_vm::compiler::TypeDiagnosticMode::RecoverAll);
    compiler.set_compile_diagnostic_mode(shape_vm::compiler::CompileDiagnosticMode::RecoverAll);

    if let (Some(path), Some(cache)) = (file_path, module_cache) {
        diagnostics.extend(validate_imports_and_register_items(
            program,
            text,
            path,
            cache,
            workspace_root,
            &mut compiler,
        ));
    }

    if let Err(compile_error) = compiler.compile_with_source(program, text) {
        let mut compile_diagnostics = error_to_diagnostic(&compile_error);
        combine_same_line_undefined_variable_diagnostics(program, text, &mut compile_diagnostics);
        diagnostics.extend(compile_diagnostics);
    }

    dedupe_and_cap_diagnostics(&mut diagnostics);
    diagnostics
}

/// Validate import statements and register imported items in the compiler.
pub fn validate_imports_and_register_items(
    program: &Program,
    text: &str,
    file_path: &std::path::Path,
    module_cache: &ModuleCache,
    workspace_root: Option<&std::path::Path>,
    compiler: &mut shape_vm::BytecodeCompiler,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let importable_modules = module_cache.list_importable_modules_with_context_and_source(
        file_path,
        workspace_root,
        Some(text),
    );
    let mut known_module_names = crate::completion::imports::module_names_with_context_and_source(
        Some(file_path),
        workspace_root,
        Some(text),
    );
    known_module_names.extend(importable_modules.iter().filter_map(|module_path| {
        module_path
            .split('.')
            .next()
            .map(|segment| segment.to_string())
    }));

    for item in &program.items {
        if let Item::Import(import_stmt, import_span) = item {
            match &import_stmt.items {
                ImportItems::Named(_) => {
                    if let Some(module_info) = module_cache
                        .load_module_by_import_with_context_and_source(
                            &import_stmt.from,
                            file_path,
                            workspace_root,
                            Some(text),
                        )
                    {
                        compiler.register_imported_items(&module_info.program.items);
                    } else {
                        diagnostics.push(make_span_diagnostic(
                            text,
                            *import_span,
                            format!(
                                "Cannot resolve module '{}'. Verify the import path and declare dependencies in shape.toml when needed.",
                                import_stmt.from
                            ),
                            DiagnosticSeverity::ERROR,
                        ));
                    }
                }
                ImportItems::Namespace { name, .. } => {
                    if !known_module_names.iter().any(|module| module == name) {
                        diagnostics.push(make_span_diagnostic(
                            text,
                            *import_span,
                            format!(
                                "Cannot resolve module '{}'. Verify the import path and declare dependencies in shape.toml when needed.",
                                name
                            ),
                            DiagnosticSeverity::ERROR,
                        ));
                    }
                }
            }
        }
    }

    diagnostics
}

fn make_span_diagnostic(
    text: &str,
    span: shape_ast::ast::Span,
    message: String,
    severity: DiagnosticSeverity,
) -> Diagnostic {
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Diagnostic {
        range: Range {
            start: Position {
                line: start_line,
                character: start_col,
            },
            end: Position {
                line: end_line,
                character: end_col,
            },
        },
        severity: Some(severity),
        message,
        source: Some("shape".to_string()),
        ..Default::default()
    }
}

fn combine_same_line_undefined_variable_diagnostics(
    program: &Program,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut undefined_diag_indices_by_line: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if is_undefined_variable_message(&diagnostic.message) {
            undefined_diag_indices_by_line
                .entry(diagnostic.range.start.line)
                .or_default()
                .push(index);
        }
    }

    if undefined_diag_indices_by_line.is_empty() {
        return;
    }

    let undefined_names_by_line = collect_undefined_identifier_names_by_line(program, text);
    if undefined_names_by_line.is_empty() {
        return;
    }

    let mut indices_to_drop: HashSet<usize> = HashSet::new();

    for (line, diag_indices) in undefined_diag_indices_by_line {
        let Some(undefined_names) = undefined_names_by_line.get(&line) else {
            continue;
        };

        if undefined_names.len() <= 1 {
            continue;
        }

        let first_index = diag_indices[0];
        diagnostics[first_index].message = format!(
            "Undefined variables: {}",
            undefined_names
                .iter()
                .map(|name| format!("'{}'", name))
                .collect::<Vec<_>>()
                .join(", ")
        );

        for index in diag_indices.into_iter().skip(1) {
            indices_to_drop.insert(index);
        }
    }

    if indices_to_drop.is_empty() {
        return;
    }

    let mut filtered = Vec::with_capacity(diagnostics.len().saturating_sub(indices_to_drop.len()));
    for (index, diagnostic) in diagnostics.drain(..).enumerate() {
        if !indices_to_drop.contains(&index) {
            filtered.push(diagnostic);
        }
    }
    *diagnostics = filtered;
}

fn is_undefined_variable_message(message: &str) -> bool {
    message.starts_with("Undefined variable: '") || message.starts_with("Undefined variable: ")
}

#[derive(Default)]
struct IdentifierCollector {
    identifiers: Vec<(String, shape_ast::ast::Span)>,
}

impl Visitor for IdentifierCollector {
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        if let Expr::Identifier(name, span) = expr
            && !span.is_dummy()
        {
            self.identifiers.push((name.clone(), *span));
        }
        true
    }
}

fn collect_undefined_identifier_names_by_line(
    program: &Program,
    text: &str,
) -> HashMap<u32, Vec<String>> {
    let scope_tree = ScopeTree::build(program, text);
    let mut collector = IdentifierCollector::default();
    walk_program(&mut collector, program);

    let mut by_line_with_offsets: HashMap<u32, Vec<(usize, String)>> = HashMap::new();
    for (name, span) in collector.identifiers {
        if scope_tree.binding_at(span.start).is_some() {
            continue;
        }
        let (line, _) = offset_to_line_col(text, span.start);
        by_line_with_offsets
            .entry(line)
            .or_default()
            .push((span.start, name));
    }

    let mut by_line: HashMap<u32, Vec<String>> = HashMap::new();
    for (line, mut names_with_offsets) in by_line_with_offsets {
        names_with_offsets.sort_by_key(|(offset, _)| *offset);
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for (_, name) in names_with_offsets {
            if seen.insert(name.clone()) {
                names.push(name);
            }
        }
        if !names.is_empty() {
            by_line.insert(line, names);
        }
    }

    by_line
}

fn dedupe_and_cap_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    diagnostics.retain(|diagnostic| seen.insert(diagnostic_dedupe_key(diagnostic)));
    if diagnostics.len() > MAX_SEMANTIC_DIAGNOSTICS {
        diagnostics.truncate(MAX_SEMANTIC_DIAGNOSTICS);
    }
}

fn diagnostic_dedupe_key(diagnostic: &Diagnostic) -> String {
    format!(
        "{}:{}:{}",
        diagnostic.range.start.line,
        diagnostic.range.start.character,
        normalize_diagnostic_message(&diagnostic.message)
    )
}

fn normalize_diagnostic_message(message: &str) -> String {
    if let Some(canonical) = canonicalize_undefined_variable_message(message) {
        return canonical;
    }
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn canonicalize_undefined_variable_message(message: &str) -> Option<String> {
    const PREFIX: &str = "Undefined variable:";
    if !message.starts_with(PREFIX) {
        return None;
    }
    let rest = message[PREFIX.len()..].trim();
    let trimmed = rest.trim_start_matches('\'');
    let name: String = trimmed
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    if name.is_empty() {
        Some("undefined variable".to_string())
    } else {
        Some(format!("undefined variable:{}", name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn semantic_analysis_keeps_named_decomposition_bindings_defined() {
        let source = r#"let a = { x: 1}
let b = { z: 3}
//print(a.y) //compiler error: no y (even though a has y in the shape via optimistic hoisting, see next line)
a.y = 2
print(a.y) //works!
let c = a+b //resulting type is {x: int, y: int, z: int}
//destructuring works, e.g.
let (d:{x}, e: {y, z})  = c
//destructuring to named structs works also but need the as keyword:
type TypeA {x: int, y: int}
type TypeB {z: int}
let (f:TypeA, g: TypeB) = c as (TypeA+TypeB)
print(f, g)
"#;

        let program = parse_program(source).expect("program should parse");
        let symbols = crate::symbols::extract_symbols(&program);
        assert!(
            symbols.iter().any(|s| s.name == "f"),
            "parser/symbol extraction should include decomposition binding f: {:?}",
            symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
        );
        assert!(
            symbols.iter().any(|s| s.name == "g"),
            "parser/symbol extraction should include decomposition binding g: {:?}",
            symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
        );
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("script.shape");
        std::fs::write(&file_path, source).expect("write source");
        let module_cache = ModuleCache::new();

        let diagnostics = analyze_program_semantics(
            &program,
            source,
            Some(&file_path),
            Some(&module_cache),
            None,
        );

        assert!(
            diagnostics
                .iter()
                .all(|diag| !diag.message.contains("Undefined variable: 'f'")),
            "unexpected diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            diagnostics
                .iter()
                .all(|diag| !diag.message.contains("Undefined variable: 'g'")),
            "unexpected diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_analysis_combines_undefined_variables_on_same_line() {
        let source = "print(h, i)\n";
        let program = parse_program(source).expect("program should parse");

        let diagnostics = analyze_program_semantics(&program, source, None, None, None);

        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("Undefined variables: 'h', 'i'")),
            "expected combined undefined variable diagnostic, got {:?}",
            messages
        );
        assert!(
            messages
                .iter()
                .all(|message| !message.contains("Undefined variable: 'h'")),
            "did not expect singular undefined diagnostic for h, got {:?}",
            messages
        );
    }

    #[test]
    fn semantic_analysis_reports_undefined_variables_on_multiple_lines() {
        let source = "print(h)\nprint(i)\n";
        let program = parse_program(source).expect("program should parse");

        let diagnostics = analyze_program_semantics(&program, source, None, None, None);

        assert!(
            diagnostics.iter().any(|diag| {
                diag.range.start.line == 0 && is_undefined_variable_message(&diag.message)
            }),
            "expected undefined variable diagnostic on line 0, got {:?}",
            diagnostics
                .iter()
                .map(|d| (d.range.start.line, d.message.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(
            diagnostics.iter().any(|diag| {
                diag.range.start.line == 1 && is_undefined_variable_message(&diag.message)
            }),
            "expected undefined variable diagnostic on line 1, got {:?}",
            diagnostics
                .iter()
                .map(|d| (d.range.start.line, d.message.as_str()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_analysis_combines_same_line_and_keeps_next_line_diagnostic() {
        let source = "print(h, i)\nprint(j)\n";
        let program = parse_program(source).expect("program should parse");

        let diagnostics = analyze_program_semantics(&program, source, None, None, None);
        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();

        assert!(
            messages
                .iter()
                .any(|message| message.contains("Undefined variables: 'h', 'i'")),
            "expected combined diagnostic for line 0, got {:?}",
            messages
        );
        assert!(
            diagnostics.iter().any(|diag| {
                diag.range.start.line == 1 && is_undefined_variable_message(&diag.message)
            }),
            "expected undefined diagnostic on line 1, got {:?}",
            diagnostics
                .iter()
                .map(|d| (d.range.start.line, d.message.as_str()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_analysis_frontmatter_foreign_function_percentile_call_has_no_type_mismatch() {
        let source = r#"---
[[extensions]]
name = "python"
path = "/tmp/libshape_ext_python.so"
---
fn python percentile(values: Array<number>, pct: number) -> number {
  sorted_v = sorted(values)
  k = (len(sorted_v) - 1) * (pct / 100.0)
  f = int(k)
  c = f + 1
  if c >= len(sorted_v):
    return sorted_v[-1]
  return sorted_v[f] + (k - f) * (sorted_v[c] - sorted_v[f])
}

print(percentile([1.0, 2.0, 3.0], 50.0))
"#;

        let parse_source = crate::util::parser_source(source);
        let program = parse_program(parse_source.as_ref()).expect("program should parse");
        let foreign_fn = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::ForeignFunction(def, _) if def.name == "percentile" => Some(def),
                _ => None,
            })
            .expect("percentile foreign function should be present");
        let first_param = foreign_fn
            .params
            .first()
            .and_then(|p| p.type_annotation.as_ref())
            .expect("first param annotation");
        assert_eq!(
            first_param.to_type_string(),
            "Array<number>",
            "unexpected foreign parameter annotation AST: {:?}",
            first_param
        );
        let diagnostics = analyze_program_semantics(&program, source, None, None, None);

        let mismatch_messages: Vec<&str> = diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .filter(|m| m.contains("Could not solve type constraints"))
            .collect();
        assert!(
            mismatch_messages.is_empty(),
            "unexpected type constraint diagnostics: {:?}",
            mismatch_messages
        );
    }

    #[test]
    fn semantic_analysis_foreign_function_accepts_struct_array_argument() {
        let source = r#"type Measurement {
  timestamp: string,
  value: number,
  sensor_id: string,
}

fn python outlier_ratio(readings: Array<Measurement>, z_threshold: number) -> number {
  values = [r['value'] for r in readings]
  mean = sum(values) / len(values)
  std = (sum((v - mean) ** 2 for v in values) / len(values)) ** 0.5
  outliers = [v for v in values if abs(v - mean) > z_threshold * std]
  return len(outliers) / len(values)
}

let readings: Array<Measurement> = [
  { timestamp: "2026-02-22T10:00:00Z", value: 10.0, sensor_id: "A" },
  { timestamp: "2026-02-22T10:01:00Z", value: 10.5, sensor_id: "A" },
  { timestamp: "2026-02-22T10:02:00Z", value: 9.8, sensor_id: "A" },
  { timestamp: "2026-02-22T10:03:00Z", value: 10.2, sensor_id: "A" },
  { timestamp: "2026-02-22T10:04:00Z", value: 35.0, sensor_id: "A" }
]

print(outlier_ratio(readings, 1.5))
"#;

        let program = parse_program(source).expect("program should parse");
        let diagnostics = analyze_program_semantics(&program, source, None, None, None);

        let mismatch_messages: Vec<&str> = diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .filter(|m| m.contains("Could not solve type constraints"))
            .collect();
        assert!(
            mismatch_messages.is_empty(),
            "unexpected type constraint diagnostics: {:?}",
            mismatch_messages
        );
    }
}
