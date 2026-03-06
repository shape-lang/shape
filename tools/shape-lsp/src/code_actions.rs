//! Code actions provider for Shape
//!
//! Provides quick fixes, refactoring actions, and source actions.

use crate::module_cache::ModuleCache;
use crate::util::{get_word_at_position, span_to_range};
use shape_ast::ast::{ImportItems, Item};
use shape_ast::parser::parse_program;
use std::collections::{HashMap, HashSet};
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Diagnostic, Position, Range, TextEdit, Uri,
    WorkspaceEdit,
};

/// Get code actions for a document at a given range
pub fn get_code_actions(
    text: &str,
    uri: &Uri,
    range: Range,
    diagnostics: &[Diagnostic],
    module_cache: Option<&ModuleCache>,
    requested_kinds: Option<&[CodeActionKind]>,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    if is_kind_requested(requested_kinds, CodeActionKind::QUICKFIX.as_str()) {
        // Add quick fixes for diagnostics at/near the requested range.
        for diagnostic in diagnostics {
            if ranges_overlap(range, diagnostic.range) {
                if let Some(fix_actions) = get_quick_fixes(text, uri, diagnostic, module_cache) {
                    actions.extend(fix_actions);
                }
            }
        }

        // Also offer symbol-based auto-import when the cursor is on an unresolved
        // type-like identifier, even if diagnostics are stale/misaligned.
        if let Some(cache) = module_cache {
            actions.extend(get_symbol_auto_import_actions(text, uri, range, cache));
        }
    }

    // Add refactoring actions based on selection
    if is_group_requested(requested_kinds, CodeActionKind::REFACTOR.as_str()) {
        if let Some(refactor_actions) = get_refactor_actions(text, uri, range) {
            actions.extend(refactor_actions);
        }
    }

    // Add source actions (organize imports, etc.)
    if is_group_requested(requested_kinds, CodeActionKind::SOURCE.as_str()) {
        if let Some(source_actions) =
            get_source_actions(text, uri, range, diagnostics, module_cache, requested_kinds)
        {
            actions.extend(source_actions);
        }
    }

    dedupe_actions(actions)
}

/// Get quick fixes for a diagnostic
fn get_quick_fixes(
    text: &str,
    uri: &Uri,
    diagnostic: &Diagnostic,
    module_cache: Option<&ModuleCache>,
) -> Option<Vec<CodeActionOrCommand>> {
    let mut fixes = Vec::new();
    let message = &diagnostic.message;

    // Fix for undefined variable - suggest declaration
    if message.contains("undefined") || message.contains("not defined") {
        if let Some(var_name) = extract_undefined_name(message) {
            fixes.push(create_quick_fix(
                format!("Declare variable '{}'", var_name),
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: diagnostic.range.start.line,
                            character: 0,
                        },
                        end: Position {
                            line: diagnostic.range.start.line,
                            character: 0,
                        },
                    },
                    new_text: format!("let {} = undefined;\n", var_name),
                }],
                diagnostic.clone(),
            ));
        }
    }

    // Fix for missing semicolon
    if message.contains("expected ';'") || message.contains("missing semicolon") {
        fixes.push(create_quick_fix(
            "Add missing semicolon".to_string(),
            uri.clone(),
            vec![TextEdit {
                range: Range {
                    start: diagnostic.range.end,
                    end: diagnostic.range.end,
                },
                new_text: ";".to_string(),
            }],
            diagnostic.clone(),
        ));
    }

    // Fix for missing closing brace
    if message.contains("expected '}'") || message.contains("unclosed") {
        fixes.push(create_quick_fix(
            "Add missing closing brace".to_string(),
            uri.clone(),
            vec![TextEdit {
                range: Range {
                    start: diagnostic.range.end,
                    end: diagnostic.range.end,
                },
                new_text: "\n}".to_string(),
            }],
            diagnostic.clone(),
        ));
    }

    // Fix for var to let conversion suggestion
    if message.contains("prefer 'let'") || message.contains("use 'let' instead of 'var'") {
        let line = get_line(text, diagnostic.range.start.line as usize);
        if let Some(line_text) = line {
            if let Some(var_pos) = line_text.find("var ") {
                fixes.push(create_quick_fix(
                    "Change 'var' to 'let'".to_string(),
                    uri.clone(),
                    vec![TextEdit {
                        range: Range {
                            start: Position {
                                line: diagnostic.range.start.line,
                                character: var_pos as u32,
                            },
                            end: Position {
                                line: diagnostic.range.start.line,
                                character: (var_pos + 3) as u32,
                            },
                        },
                        new_text: "let".to_string(),
                    }],
                    diagnostic.clone(),
                ));
            }
        }
    }

    // Auto-import for unknown enum/type
    if message.contains("Unknown enum type") || message.contains("Unknown variant") {
        if let Some(cache) = module_cache {
            if let Some(name) = extract_quoted_name(message) {
                let symbols = if let Some(current_file) = uri.to_file_path() {
                    cache.find_exported_symbol_with_context(&name, current_file.as_ref(), None)
                } else {
                    cache.find_exported_symbol(&name)
                };
                for (import_path, _export) in symbols {
                    fixes.push(create_quick_fix(
                        format!("Import '{}' from {}", name, import_path),
                        uri.clone(),
                        vec![TextEdit {
                            range: Range {
                                start: Position {
                                    line: 0,
                                    character: 0,
                                },
                                end: Position {
                                    line: 0,
                                    character: 0,
                                },
                            },
                            new_text: format!("from {} use {{ {} }}\n", import_path, name),
                        }],
                        diagnostic.clone(),
                    ));
                }
            }
        }
    }

    if message.contains("match expression requires at least one arm") {
        if let Some((insert_pos, indent)) = find_match_arm_insert_position(text, diagnostic.range) {
            let arm_indent = format!("{indent}  ");
            fixes.push(create_quick_fix(
                "Add wildcard match arm".to_string(),
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: insert_pos,
                        end: insert_pos,
                    },
                    new_text: format!("{arm_indent}_ => {{\n{arm_indent}}},\n"),
                }],
                diagnostic.clone(),
            ));
        }
    }

    if let Some((enum_name, missing_variants)) = parse_non_exhaustive_match(message) {
        if let Some((insert_pos, indent)) = find_match_arm_insert_position(text, diagnostic.range) {
            let arm_indent = format!("{indent}  ");
            let mut new_text = String::new();
            for variant in missing_variants {
                new_text.push_str(&format!(
                    "{arm_indent}{enum_name}::{variant} => {{\n{arm_indent}}},\n"
                ));
            }
            fixes.push(create_quick_fix(
                format!("Add missing match arms for {}", enum_name),
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: insert_pos,
                        end: insert_pos,
                    },
                    new_text,
                }],
                diagnostic.clone(),
            ));
        }
    }

    // Fix for missing required trait method — suggest adding the method stub
    if message.contains("Missing required method") {
        if let Some(method_name) = extract_quoted_name(message) {
            // Find the closing brace of the impl block on the diagnostic line
            let impl_end_line = diagnostic.range.end.line;
            // Insert just before the closing brace
            fixes.push(create_quick_fix(
                format!("Implement method '{}'", method_name),
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: impl_end_line,
                            character: 0,
                        },
                        end: Position {
                            line: impl_end_line,
                            character: 0,
                        },
                    },
                    new_text: format!(
                        "    method {}() {{\n        // TODO: implement\n    }}\n",
                        method_name
                    ),
                }],
                diagnostic.clone(),
            ));
        }
    }

    // Fix for unused variable - add underscore prefix
    if message.contains("unused") {
        if let Some(var_name) = extract_unused_name(message) {
            if !var_name.starts_with('_') {
                let line = get_line(text, diagnostic.range.start.line as usize);
                if let Some(line_text) = line {
                    if let Some(name_pos) = line_text.find(&var_name) {
                        fixes.push(create_quick_fix(
                            format!("Prefix with underscore: _{}", var_name),
                            uri.clone(),
                            vec![TextEdit {
                                range: Range {
                                    start: Position {
                                        line: diagnostic.range.start.line,
                                        character: name_pos as u32,
                                    },
                                    end: Position {
                                        line: diagnostic.range.start.line,
                                        character: (name_pos + var_name.len()) as u32,
                                    },
                                },
                                new_text: format!("_{}", var_name),
                            }],
                            diagnostic.clone(),
                        ));
                    }
                }
            }
        }
    }

    if fixes.is_empty() { None } else { Some(fixes) }
}

/// Get refactoring actions for a selection
fn get_refactor_actions(text: &str, uri: &Uri, range: Range) -> Option<Vec<CodeActionOrCommand>> {
    let mut actions = Vec::new();

    // Get the selected text
    let selected = get_text_in_range(text, range);
    if selected.is_empty() {
        return None;
    }

    // Extract to variable
    if is_expression(&selected) {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Extract to variable".to_string(),
            kind: Some(CodeActionKind::REFACTOR_EXTRACT),
            diagnostics: None,
            edit: Some(WorkspaceEdit {
                changes: Some({
                    let mut changes = HashMap::new();
                    changes.insert(
                        uri.clone(),
                        vec![
                            TextEdit {
                                range: Range {
                                    start: Position {
                                        line: range.start.line,
                                        character: 0,
                                    },
                                    end: Position {
                                        line: range.start.line,
                                        character: 0,
                                    },
                                },
                                new_text: format!("let extracted = {};\n", selected),
                            },
                            TextEdit {
                                range,
                                new_text: "extracted".to_string(),
                            },
                        ],
                    );
                    changes
                }),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Extract to function (for multi-line selections or complex expressions)
    if selected.contains('\n') || selected.len() > 50 {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Extract to function".to_string(),
            kind: Some(CodeActionKind::REFACTOR_EXTRACT),
            diagnostics: None,
            edit: Some(WorkspaceEdit {
                changes: Some({
                    let mut changes = HashMap::new();
                    changes.insert(
                        uri.clone(),
                        vec![
                            TextEdit {
                                range: Range {
                                    start: Position {
                                        line: 0,
                                        character: 0,
                                    },
                                    end: Position {
                                        line: 0,
                                        character: 0,
                                    },
                                },
                                new_text: format!(
                                    "fn extractedFunction() {{\n    {}\n}}\n\n",
                                    selected.replace('\n', "\n    ")
                                ),
                            },
                            TextEdit {
                                range,
                                new_text: "extractedFunction()".to_string(),
                            },
                        ],
                    );
                    changes
                }),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Convert string concatenation to template string
    if selected.contains(" + ") && selected.contains('"') {
        // This is a simplification - real implementation would need proper parsing
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Convert to template string".to_string(),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: None,
            edit: None, // Would need proper implementation
            command: None,
            is_preferred: None,
            disabled: Some(tower_lsp_server::ls_types::CodeActionDisabled {
                reason: "Complex conversion - manual edit recommended".to_string(),
            }),
            data: None,
        }));
    }

    if actions.is_empty() {
        None
    } else {
        Some(actions)
    }
}

/// Get source actions for the document
fn get_source_actions(
    text: &str,
    uri: &Uri,
    range: Range,
    diagnostics: &[Diagnostic],
    module_cache: Option<&ModuleCache>,
    requested_kinds: Option<&[CodeActionKind]>,
) -> Option<Vec<CodeActionOrCommand>> {
    let mut actions = Vec::new();

    let import_ranges = import_statement_ranges(text);
    let on_import_stmt = import_ranges.iter().any(|r| ranges_overlap(*r, range));
    let organize_requested = is_kind_explicitly_requested(
        requested_kinds,
        CodeActionKind::SOURCE_ORGANIZE_IMPORTS.as_str(),
    );

    // Show organize-imports only when explicitly requested or when cursor is
    // currently inside import declarations.
    if !import_ranges.is_empty() && (organize_requested || on_import_stmt) {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Organize imports".to_string(),
            kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
            diagnostics: None,
            edit: None, // Would need proper implementation
            command: None,
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Add "Fix all" only when requested explicitly or when there are fixable
    // diagnostics on the current range.
    let fix_all_requested =
        is_kind_explicitly_requested(requested_kinds, CodeActionKind::SOURCE_FIX_ALL.as_str());
    let has_fixable_here = diagnostics
        .iter()
        .filter(|d| ranges_overlap(d.range, range))
        .any(|d| get_quick_fixes(text, uri, d, module_cache).is_some());
    if fix_all_requested || has_fixable_here {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Fix all auto-fixable problems".to_string(),
            kind: Some(CodeActionKind::SOURCE_FIX_ALL),
            diagnostics: None,
            edit: None, // Would need to collect all quick fixes
            command: None,
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    if actions.is_empty() {
        None
    } else {
        Some(actions)
    }
}

/// Create a quick fix code action
fn create_quick_fix(
    title: String,
    uri: Uri,
    edits: Vec<TextEdit>,
    diagnostic: Diagnostic,
) -> CodeActionOrCommand {
    let mut changes = HashMap::new();
    changes.insert(uri, edits);

    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    })
}

/// Extract a single-quoted name from an error message
fn extract_quoted_name(message: &str) -> Option<String> {
    let start = message.find('\'')?;
    let end = message[start + 1..].find('\'')?;
    Some(message[start + 1..start + 1 + end].to_string())
}

/// Extract the undefined variable name from an error message
fn extract_undefined_name(message: &str) -> Option<String> {
    // Pattern: "undefined variable 'name'" or "'name' is not defined"
    if let Some(start) = message.find('\'') {
        if let Some(end) = message[start + 1..].find('\'') {
            return Some(message[start + 1..start + 1 + end].to_string());
        }
    }
    None
}

/// Extract the unused variable name from an error message
fn extract_unused_name(message: &str) -> Option<String> {
    // Pattern: "unused variable 'name'" or "'name' is unused"
    if let Some(start) = message.find('\'') {
        if let Some(end) = message[start + 1..].find('\'') {
            return Some(message[start + 1..start + 1 + end].to_string());
        }
    }
    None
}

fn parse_non_exhaustive_match(message: &str) -> Option<(String, Vec<String>)> {
    const PREFIX: &str = "Non-exhaustive match on '";
    const MARKER: &str = "': missing variants ";
    let after_prefix = message.strip_prefix(PREFIX)?;
    let marker_pos = after_prefix.find(MARKER)?;
    let enum_name = after_prefix[..marker_pos].trim().to_string();
    if enum_name.is_empty() {
        return None;
    }
    let variants_part = &after_prefix[marker_pos + MARKER.len()..];
    let variants = variants_part
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    if variants.is_empty() {
        None
    } else {
        Some((enum_name, variants))
    }
}

fn find_match_arm_insert_position(text: &str, range: Range) -> Option<(Position, String)> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let start_line = range.start.line as usize;
    let mut line_index = start_line.min(lines.len().saturating_sub(1));
    while line_index < lines.len() {
        let line = lines[line_index];
        let trimmed = line.trim_start();
        if trimmed.starts_with('}') {
            let indent_len = line.len().saturating_sub(trimmed.len());
            let indent = " ".repeat(indent_len);
            return Some((
                Position {
                    line: line_index as u32,
                    character: 0,
                },
                indent,
            ));
        }
        line_index += 1;
    }
    None
}

/// Check if two ranges overlap
fn ranges_overlap(a: Range, b: Range) -> bool {
    !(a.end.line < b.start.line
        || (a.end.line == b.start.line && a.end.character < b.start.character)
        || b.end.line < a.start.line
        || (b.end.line == a.start.line && b.end.character < a.start.character))
}

/// Get a line from text by line number
fn get_line(text: &str, line: usize) -> Option<&str> {
    text.lines().nth(line)
}

/// Get text within a range
fn get_text_in_range(text: &str, range: Range) -> String {
    let lines: Vec<&str> = text.lines().collect();

    if range.start.line == range.end.line {
        // Single line selection
        if let Some(line) = lines.get(range.start.line as usize) {
            let start = range.start.character as usize;
            let end = range.end.character as usize;
            if start < line.len() && end <= line.len() {
                return line[start..end].to_string();
            }
        }
    } else {
        // Multi-line selection
        let mut result = String::new();

        for (i, line) in lines.iter().enumerate() {
            let line_num = i as u32;

            if line_num < range.start.line {
                continue;
            }
            if line_num > range.end.line {
                break;
            }

            if line_num == range.start.line {
                let start = range.start.character as usize;
                if start < line.len() {
                    result.push_str(&line[start..]);
                }
            } else if line_num == range.end.line {
                let end = range.end.character as usize;
                if end <= line.len() {
                    result.push_str(&line[..end]);
                }
            } else {
                result.push_str(line);
            }

            if line_num != range.end.line {
                result.push('\n');
            }
        }

        return result;
    }

    String::new()
}

/// Check if a string looks like an expression
fn is_expression(text: &str) -> bool {
    let trimmed = text.trim();

    // Empty or whitespace-only is not an expression
    if trimmed.is_empty() {
        return false;
    }

    // Statements are not expressions (simplified check)
    if trimmed.starts_with("let ")
        || trimmed.starts_with("var ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("function ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("return ")
    {
        return false;
    }

    // Try to parse as expression
    let test_code = format!("let _test = {};", trimmed);
    parse_program(&test_code).is_ok()
}

/// True when `requested_kinds` allows the exact `target` kind.
/// If a broad parent kind is requested (e.g. `source`), sub-kinds are allowed.
fn is_kind_requested(requested_kinds: Option<&[CodeActionKind]>, target: &str) -> bool {
    match requested_kinds {
        None => true,
        Some(kinds) if kinds.is_empty() => true,
        Some(kinds) => kinds.iter().any(|k| {
            let requested = k.as_str();
            requested == target || target.starts_with(&format!("{requested}."))
        }),
    }
}

/// True when `requested_kinds` allows a kind group (`quickfix`, `source`, `refactor`).
fn is_group_requested(requested_kinds: Option<&[CodeActionKind]>, group: &str) -> bool {
    match requested_kinds {
        None => true,
        Some(kinds) if kinds.is_empty() => true,
        Some(kinds) => kinds.iter().any(|k| {
            let requested = k.as_str();
            requested == group || requested.starts_with(&format!("{group}."))
        }),
    }
}

/// True only when a request explicitly provided `only` kinds including `target`
/// (or a parent kind such as `source` for `source.organizeImports`).
fn is_kind_explicitly_requested(requested_kinds: Option<&[CodeActionKind]>, target: &str) -> bool {
    let Some(kinds) = requested_kinds else {
        return false;
    };
    if kinds.is_empty() {
        return false;
    }
    kinds.iter().any(|k| {
        let requested = k.as_str();
        requested == target || target.starts_with(&format!("{requested}."))
    })
}

/// Deduplicate actions by `(kind, title)`.
fn dedupe_actions(actions: Vec<CodeActionOrCommand>) -> Vec<CodeActionOrCommand> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for action in actions {
        let key = match &action {
            CodeActionOrCommand::CodeAction(ca) => format!(
                "{}::{}",
                ca.kind.as_ref().map(|k| k.as_str()).unwrap_or(""),
                ca.title
            ),
            CodeActionOrCommand::Command(cmd) => format!("command::{}", cmd.title),
        };

        if seen.insert(key) {
            deduped.push(action);
        }
    }

    deduped
}

/// Collect LSP ranges for parsed import statements.
fn import_statement_ranges(text: &str) -> Vec<Range> {
    if let Ok(program) = parse_program(text) {
        let mut ranges = Vec::new();
        for item in &program.items {
            if let Item::Import(_, span) = item {
                ranges.push(span_to_range(text, span));
            }
        }
        return ranges;
    }

    // Parse fallback: use line-based detection for unfinished import lines.
    text.lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let trimmed = raw.trim_start();
            if trimmed.starts_with("from ") || trimmed.starts_with("use ") {
                Some(Range {
                    start: Position {
                        line: line as u32,
                        character: 0,
                    },
                    end: Position {
                        line: line as u32,
                        character: raw.len() as u32,
                    },
                })
            } else {
                None
            }
        })
        .collect()
}

/// Return local names currently imported into scope.
fn collect_imported_local_names(text: &str) -> HashSet<String> {
    let Ok(program) = parse_program(text) else {
        return HashSet::new();
    };

    let mut imported = HashSet::new();
    for item in &program.items {
        let Item::Import(import_stmt, _) = item else {
            continue;
        };
        match &import_stmt.items {
            ImportItems::Named(specs) => {
                for spec in specs {
                    imported.insert(spec.alias.clone().unwrap_or_else(|| spec.name.clone()));
                }
            }
            ImportItems::Namespace { name, alias } => {
                imported.insert(alias.clone().unwrap_or_else(|| name.clone()));
            }
        }
    }

    imported
}

fn import_insert_position(text: &str) -> Position {
    let import_ranges = import_statement_ranges(text);
    if let Some(last_line) = import_ranges.iter().map(|r| r.end.line).max() {
        Position {
            line: last_line + 1,
            character: 0,
        }
    } else {
        Position {
            line: 0,
            character: 0,
        }
    }
}

fn get_symbol_auto_import_actions(
    text: &str,
    uri: &Uri,
    range: Range,
    cache: &ModuleCache,
) -> Vec<CodeActionOrCommand> {
    let Some(symbol) = symbol_at_or_in_range(text, range) else {
        return Vec::new();
    };
    if !is_import_candidate_symbol(&symbol) {
        return Vec::new();
    }

    let imported_names = collect_imported_local_names(text);
    if imported_names.contains(&symbol) {
        return Vec::new();
    }

    let matches = if let Some(current_file) = uri.to_file_path() {
        cache.find_exported_symbol_with_context(&symbol, current_file.as_ref(), None)
    } else {
        cache.find_exported_symbol(&symbol)
    };
    if matches.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let insert_at = import_insert_position(text);
    for (import_path, _export) in matches {
        out.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Import '{}' from {}", symbol, import_path),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: None,
            edit: Some(WorkspaceEdit {
                changes: Some({
                    let mut changes = HashMap::new();
                    changes.insert(
                        uri.clone(),
                        vec![TextEdit {
                            range: Range {
                                start: insert_at,
                                end: insert_at,
                            },
                            new_text: format!("from {} use {{ {} }}\n", import_path, symbol),
                        }],
                    );
                    changes
                }),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        }));
    }

    out
}

fn symbol_at_or_in_range(text: &str, range: Range) -> Option<String> {
    let selected = get_text_in_range(text, range);
    let selected = selected.trim();
    if !selected.is_empty() && is_identifier(selected) {
        return Some(selected.to_string());
    }
    get_word_at_position(text, range.start)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_import_candidate_symbol(name: &str) -> bool {
    is_identifier(name) && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_undefined_name() {
        assert_eq!(
            extract_undefined_name("undefined variable 'foo'"),
            Some("foo".to_string())
        );
        assert_eq!(
            extract_undefined_name("'bar' is not defined"),
            Some("bar".to_string())
        );
        assert_eq!(extract_undefined_name("some other message"), None);
    }

    #[test]
    fn test_ranges_overlap() {
        let r1 = Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 10,
            },
        };
        let r2 = Range {
            start: Position {
                line: 1,
                character: 5,
            },
            end: Position {
                line: 1,
                character: 15,
            },
        };
        let r3 = Range {
            start: Position {
                line: 2,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 10,
            },
        };

        assert!(ranges_overlap(r1, r2));
        assert!(!ranges_overlap(r1, r3));
    }

    #[test]
    fn test_get_text_in_range() {
        let text = "let x = 42;\nlet y = 10;";

        let range = Range {
            start: Position {
                line: 0,
                character: 4,
            },
            end: Position {
                line: 0,
                character: 5,
            },
        };
        assert_eq!(get_text_in_range(text, range), "x");

        let range = Range {
            start: Position {
                line: 0,
                character: 8,
            },
            end: Position {
                line: 0,
                character: 10,
            },
        };
        assert_eq!(get_text_in_range(text, range), "42");
    }

    #[test]
    fn test_is_expression() {
        assert!(is_expression("42"));
        assert!(is_expression("x + y"));
        assert!(is_expression("foo()"));
        assert!(!is_expression("let x = 42"));
        assert!(!is_expression("function foo() {}"));
    }

    #[test]
    fn test_extract_quoted_name_from_compiler_errors() {
        // Matches actual compiler error message format from checking.rs
        assert_eq!(
            extract_quoted_name(
                "Unknown enum type 'Snapshot'. Make sure it is imported or defined."
            ),
            Some("Snapshot".to_string())
        );
        assert_eq!(
            extract_quoted_name("Unknown variant 'BadVariant' for enum 'Color'"),
            Some("BadVariant".to_string())
        );
        assert_eq!(extract_quoted_name("no quotes here"), None);
    }

    #[test]
    fn test_missing_trait_method_quick_fix() {
        let text = "trait Q {\n    filter(p): any;\n    select(c): any\n}\nimpl Q for T {\n    method filter(p) { self }\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let diagnostic = Diagnostic {
            range: Range {
                start: Position {
                    line: 4,
                    character: 0,
                },
                end: Position {
                    line: 6,
                    character: 1,
                },
            },
            severity: Some(tower_lsp_server::ls_types::DiagnosticSeverity::ERROR),
            code: Some(tower_lsp_server::ls_types::NumberOrString::String(
                "E0401".to_string(),
            )),
            message: "Missing required method 'select' in impl Q for T.".to_string(),
            ..Default::default()
        };
        let actions = get_code_actions(text, &uri, diagnostic.range, &[diagnostic], None, None);
        assert!(
            actions.iter().any(|a| {
                if let CodeActionOrCommand::CodeAction(action) = a {
                    action.title.contains("Implement method 'select'")
                } else {
                    false
                }
            }),
            "Should have quick fix to implement missing method. Got: {:?}",
            actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(action) => action.title.clone(),
                    CodeActionOrCommand::Command(cmd) => cmd.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_auto_import_generates_valid_syntax() {
        // Verify the generated import text matches Shape grammar
        let name = "Snapshot";
        let import_path = "std::core::snapshot";
        let import_text = format!("from {} use {{ {} }}\n", import_path, name);
        assert_eq!(import_text, "from std::core::snapshot use { Snapshot }\n");
        // Verify it parses as valid Shape
        let full_code = format!("{}let x = 1\n", import_text);
        assert!(
            shape_ast::parser::parse_program(&full_code).is_ok(),
            "Generated import should be valid Shape syntax: {}",
            full_code
        );
    }

    #[test]
    fn test_source_actions_organize_imports_only_on_import_lines() {
        let text = "from std::core::math use { abs }\nlet x = abs(1)\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();

        let import_range = Range {
            start: Position {
                line: 0,
                character: 5,
            },
            end: Position {
                line: 0,
                character: 5,
            },
        };
        let non_import_range = Range {
            start: Position {
                line: 1,
                character: 4,
            },
            end: Position {
                line: 1,
                character: 4,
            },
        };

        let on_import = get_code_actions(text, &uri, import_range, &[], None, None);
        let away_from_import = get_code_actions(text, &uri, non_import_range, &[], None, None);

        let has_organize = |actions: &[CodeActionOrCommand]| {
            actions.iter().any(|a| {
                matches!(
                    a,
                    CodeActionOrCommand::CodeAction(CodeAction {
                        kind: Some(kind),
                        ..
                    }) if kind == &CodeActionKind::SOURCE_ORGANIZE_IMPORTS
                )
            })
        };

        assert!(has_organize(&on_import));
        assert!(!has_organize(&away_from_import));
    }

    #[test]
    fn test_symbol_auto_import_action_from_cursor() {
        let text = "match snapshot() {\n  Snapshot::Resumed => { }\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let range = Range {
            start: Position {
                line: 1,
                character: 3,
            },
            end: Position {
                line: 1,
                character: 11,
            },
        };

        let cache = ModuleCache::new();
        let actions = get_code_actions(text, &uri, range, &[], Some(&cache), None);
        assert!(
            actions.iter().any(|a| {
                matches!(
                    a,
                    CodeActionOrCommand::CodeAction(CodeAction { title, .. })
                        if title.contains("Import 'Snapshot' from std::core::snapshot")
                )
            }),
            "Expected auto-import action for Snapshot. Got: {:?}",
            actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(action) => action.title.clone(),
                    CodeActionOrCommand::Command(cmd) => cmd.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_match_quick_fix_adds_wildcard_arm() {
        let text = "fn afunc(c) {\n  match c {\n\n  }\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let diagnostic = Diagnostic {
            range: Range {
                start: Position {
                    line: 1,
                    character: 2,
                },
                end: Position {
                    line: 3,
                    character: 3,
                },
            },
            severity: None,
            code: None,
            code_description: None,
            source: Some("shape".to_string()),
            message: "match expression requires at least one arm".to_string(),
            related_information: None,
            tags: None,
            data: None,
        };
        let range = Range {
            start: Position {
                line: 2,
                character: 2,
            },
            end: Position {
                line: 2,
                character: 2,
            },
        };

        let actions = get_code_actions(text, &uri, range, &[diagnostic], None, None);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                CodeActionOrCommand::CodeAction(CodeAction { title, .. })
                    if title == "Add wildcard match arm"
            )),
            "Expected wildcard match-arm quick fix. Got: {:?}",
            actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(action) => action.title.clone(),
                    CodeActionOrCommand::Command(cmd) => cmd.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_non_exhaustive_match_quick_fix_adds_missing_arms() {
        let text = "match snapshot() {\n  Snapshot::Resumed => { }\n}\n";
        let uri = Uri::from_file_path("/tmp/test.shape").unwrap();
        let diagnostic = Diagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 2,
                    character: 1,
                },
            },
            severity: None,
            code: None,
            code_description: None,
            source: Some("shape".to_string()),
            message: "Non-exhaustive match on 'Snapshot': missing variants Hash".to_string(),
            related_information: None,
            tags: None,
            data: None,
        };
        let range = Range {
            start: Position {
                line: 1,
                character: 5,
            },
            end: Position {
                line: 1,
                character: 5,
            },
        };

        let actions = get_code_actions(text, &uri, range, &[diagnostic], None, None);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                CodeActionOrCommand::CodeAction(CodeAction { title, .. })
                    if title == "Add missing match arms for Snapshot"
            )),
            "Expected missing-arms quick fix. Got: {:?}",
            actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(action) => action.title.clone(),
                    CodeActionOrCommand::Command(cmd) => cmd.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }
}
