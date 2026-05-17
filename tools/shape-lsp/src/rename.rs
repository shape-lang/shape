//! Rename support for Shape
//!
//! Provides symbol renaming across the document using text-based searching.

use crate::document::DocumentManager;
use crate::module_cache::ModuleCache;
use crate::util::{get_word_at_position, offset_to_line_col, position_to_offset};
use shape_ast::ast::{ImportItems, Item, Program, Statement};
use shape_ast::parser::parse_program;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp_server::ls_types::{
    Position, PrepareRenameResponse, Range, TextEdit, Uri, WorkspaceEdit,
};

/// Prepare for rename - check if the symbol at the position can be renamed
pub fn prepare_rename(text: &str, position: Position) -> Option<PrepareRenameResponse> {
    // Get the word at position
    let word = get_word_at_position(text, position)?;
    let range = get_word_range(text, position)?;

    // Check if it's a renameable symbol
    if is_keyword(&word) {
        return None;
    }

    // Check if it's a built-in function
    if is_builtin_function(&word) {
        return None;
    }

    Some(PrepareRenameResponse::Range(range))
}

/// Perform the rename operation.
///
/// When `cached_program` is provided, it is used as fallback when the
/// current source fails to parse.
pub fn rename(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
) -> Option<WorkspaceEdit> {
    // Validate new name
    if !is_valid_identifier(new_name) {
        return None;
    }

    // Get the current name
    let old_name = get_word_at_position(text, position)?;

    // Check if it's renameable
    if is_keyword(&old_name) || is_builtin_function(&old_name) {
        return None;
    }

    // Parse to extract symbols and verify this is a valid symbol
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => match cached_program {
            Some(p) => p.clone(),
            None => return None,
        },
    };

    // Convert cursor position to byte offset for scope-aware lookup
    let offset = position_to_offset(text, position)?;
    let tree = crate::scope::ScopeTree::build(&program, text);

    // Use scope-aware resolution to find all references to this binding
    let edits = if let Some(spans) = tree.references_of(offset) {
        spans
            .into_iter()
            .map(|(start, end)| {
                let (start_line, start_col) = offset_to_line_col(text, start);
                let (end_line, end_col) = offset_to_line_col(text, end);
                TextEdit {
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
                    new_text: new_name.to_string(),
                }
            })
            .collect()
    } else {
        // Fallback to text-based search
        find_symbol_occurrences(text, &old_name, new_name)
    };

    if edits.is_empty() {
        return None;
    }

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

/// W2.6 — Cross-file rename.
///
/// Performs the same scope-aware in-file rename as [`rename`], then for
/// module-scope symbols extends edits to other open documents + workspace
/// `.shape` files. Mirrors the cross-file find-references algorithm in
/// `definition.rs::get_references_cross_file`: only top-level
/// (module-scope-visible) symbols cascade; locally-shadowing inner bindings
/// are excluded by `ScopeTree` semantics in each file.
///
/// Returns `None` if the symbol is not renameable or no edits were
/// produced.
#[allow(clippy::too_many_arguments)]
pub fn rename_cross_file(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
    documents: Option<&DocumentManager>,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&Path>,
) -> Option<WorkspaceEdit> {
    // Same-file edits via the existing scope-aware path.
    let mut workspace_edit = rename(text, uri, position, new_name, cached_program)?;

    // Determine the symbol name for cross-file scan.
    let Some(old_name) = get_word_at_position(text, position) else {
        return Some(workspace_edit);
    };

    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => match cached_program {
            Some(p) => p.clone(),
            None => return Some(workspace_edit),
        },
    };

    if !is_module_scope_symbol_in_rename(&program, &old_name) {
        // Local-scope binding — same-file rename is sufficient.
        return Some(workspace_edit);
    }

    let changes_map = workspace_edit.changes.get_or_insert_with(HashMap::new);

    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    if let Some(current_path) = uri.to_file_path() {
        visited.insert(current_path.into_owned());
    }

    if let Some(docs) = documents {
        for other_uri in docs.all_uris() {
            if &other_uri == uri {
                continue;
            }
            let Some(other_path_cow) = other_uri.to_file_path() else {
                continue;
            };
            let other_path = other_path_cow.into_owned();
            if !visited.insert(other_path.clone()) {
                continue;
            }
            let Some(other_doc) = docs.get(&other_uri) else {
                continue;
            };
            let other_text = other_doc.text();
            let edits =
                collect_module_scope_edits_in_file(&other_text, &old_name, new_name);
            if !edits.is_empty() {
                changes_map
                    .entry(other_uri)
                    .or_insert_with(Vec::new)
                    .extend(edits);
            }
        }
    }

    if let (Some(cache), Some(root)) = (module_cache, workspace_root) {
        let _ = cache;
        for path in cache.enumerate_workspace_shape_files(root) {
            if !visited.insert(path.clone()) {
                continue;
            }
            let Some(other_uri) = Uri::from_file_path(&path) else {
                continue;
            };
            let Ok(other_text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let edits =
                collect_module_scope_edits_in_file(&other_text, &old_name, new_name);
            if !edits.is_empty() {
                changes_map
                    .entry(other_uri)
                    .or_insert_with(Vec::new)
                    .extend(edits);
            }
        }
    }

    Some(workspace_edit)
}

/// Mirrors `definition::is_module_scope_symbol` — kept local to avoid a
/// pub-cross-module dependency between sibling LSP modules.
fn is_module_scope_symbol_in_rename(program: &Program, name: &str) -> bool {
    for item in &program.items {
        match item {
            Item::Function(func, _) if func.name == name => return true,
            Item::ForeignFunction(func, _) if func.name == name => return true,
            Item::Trait(t, _) if t.name == name => return true,
            Item::Enum(e, _) if e.name == name => return true,
            Item::TypeAlias(ta, _) if ta.name == name => return true,
            Item::StructType(s, _) if s.name == name => return true,
            Item::VariableDecl(decl, _) => {
                for (n, _) in crate::symbols::get_pattern_names(&decl.pattern) {
                    if n == name {
                        return true;
                    }
                }
            }
            Item::Statement(Statement::VariableDecl(decl, _), _) => {
                for (n, _) in crate::symbols::get_pattern_names(&decl.pattern) {
                    if n == name {
                        return true;
                    }
                }
            }
            Item::Import(import_stmt, _) => match &import_stmt.items {
                ImportItems::Named(specs) => {
                    for spec in specs {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        if local == name {
                            return true;
                        }
                    }
                }
                ImportItems::Namespace { name: ns_name, alias } => {
                    let local = alias.as_ref().unwrap_or(ns_name);
                    if local == name {
                        return true;
                    }
                }
            },
            _ => {}
        }
    }
    false
}

/// Collect TextEdits for module-scope occurrences of `old_name` in
/// `text`, replacing each with `new_name`. Uses ScopeTree to skip
/// locally-shadowing inner bindings.
fn collect_module_scope_edits_in_file(
    text: &str,
    old_name: &str,
    new_name: &str,
) -> Vec<TextEdit> {
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                return Vec::new();
            }
            partial.into_program()
        }
    };

    if !is_module_scope_symbol_in_rename(&program, old_name) {
        return Vec::new();
    }

    let tree = crate::scope::ScopeTree::build(&program, text);
    let Some(root) = tree.scopes.first() else {
        return Vec::new();
    };

    let mut edits = Vec::new();
    for binding in &root.bindings {
        if binding.name != old_name {
            continue;
        }
        let mut push = |span: (usize, usize)| {
            let (sl, sc) = offset_to_line_col(text, span.0);
            let (el, ec) = offset_to_line_col(text, span.1);
            edits.push(TextEdit {
                range: Range {
                    start: Position {
                        line: sl,
                        character: sc,
                    },
                    end: Position {
                        line: el,
                        character: ec,
                    },
                },
                new_text: new_name.to_string(),
            });
        };
        push(binding.def_span);
        for span in &binding.references {
            push(*span);
        }
    }
    edits
}

/// Find all occurrences of a symbol and create edits to rename them
fn find_symbol_occurrences(text: &str, old_name: &str, new_name: &str) -> Vec<TextEdit> {
    let mut edits = Vec::new();

    // Find all word-boundary matches
    let name_len = old_name.len();

    for (i, _) in text.match_indices(old_name) {
        // Check word boundaries
        let before_ok = i == 0 || !is_identifier_char(text.chars().nth(i - 1).unwrap_or(' '));
        let after_ok = i + name_len >= text.len()
            || !is_identifier_char(text.chars().nth(i + name_len).unwrap_or(' '));

        if before_ok && after_ok {
            let (start_line, start_col) = offset_to_line_col(text, i);
            let (end_line, end_col) = offset_to_line_col(text, i + name_len);

            edits.push(TextEdit {
                range: Range {
                    start: Position {
                        line: start_line as u32,
                        character: start_col as u32,
                    },
                    end: Position {
                        line: end_line as u32,
                        character: end_col as u32,
                    },
                },
                new_text: new_name.to_string(),
            });
        }
    }

    edits
}

/// Get the range of the word at a position
fn get_word_range(text: &str, position: Position) -> Option<Range> {
    let offset = position_to_offset(text, position)?;
    let bytes = text.as_bytes();

    // Find word start
    let mut start = offset;
    while start > 0 && is_identifier_char(bytes[start - 1] as char) {
        start -= 1;
    }

    // Find word end
    let mut end = offset;
    while end < bytes.len() && is_identifier_char(bytes[end] as char) {
        end += 1;
    }

    if start == end {
        return None;
    }

    let (start_line, start_col) = offset_to_line_col(text, start);
    let (end_line, end_col) = offset_to_line_col(text, end);

    Some(Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    })
}

/// Check if a character is valid in an identifier
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Check if a string is a valid identifier
fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap();

    // First character must be letter or underscore
    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    // Rest must be alphanumeric or underscore
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Check if a string is a Shape keyword
fn is_keyword(name: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "let",
        "var",
        "const",
        "function",
        "pattern",
        "if",
        "else",
        "while",
        "for",
        "return",
        "break",
        "continue",
        "in",
        "and",
        "or",
        "not",
        "true",
        "false",
        "None",
        "Some",
        "pub",
        "from",
        "type",
        "trait",
        "enum",
        "extend",
        "find",
        "scan",
        "analyze",
        "backtest",
        "alert",
        "on",
        "test",
        "stream",
    ];
    KEYWORDS.contains(&name)
}

/// Check if a name is a language-level built-in function
///
/// These are functions provided by the VM runtime, not from stdlib.
/// Stdlib functions are discovered dynamically via annotation/import discovery.
fn is_builtin_function(name: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "abs", "sqrt", "pow", "log", "exp", "sin", "cos", "tan", "min", "max", "avg", "sum", "std",
        "variance", "len", "first", "last", "at", "slice", "map", "filter", "reduce", "sort",
        "reverse", "unique", "flatten", "zip", "range", "print",
    ];
    BUILTINS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("baz123"));
        assert!(is_valid_identifier("_"));

        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123abc"));
        assert!(!is_valid_identifier("foo-bar"));
    }

    #[test]
    fn test_is_keyword() {
        assert!(is_keyword("let"));
        assert!(is_keyword("function"));
        assert!(is_keyword("if"));

        assert!(!is_keyword("foo"));
        assert!(!is_keyword("myVar"));
    }

    #[test]
    fn test_is_builtin_function() {
        // stdlib functions (e.g. sma/ema) are not language-level builtins.
        assert!(is_builtin_function("print"));
        assert!(is_builtin_function("abs"));

        assert!(!is_builtin_function("myFunc"));
        assert!(!is_builtin_function("sma")); // Now in stdlib/finance, not builtin
    }

    #[test]
    fn test_get_word_at_position() {
        let text = "let foo = bar + baz;";

        let word = get_word_at_position(
            text,
            Position {
                line: 0,
                character: 5,
            },
        );
        assert_eq!(word, Some("foo".to_string()));

        let word = get_word_at_position(
            text,
            Position {
                line: 0,
                character: 11,
            },
        );
        assert_eq!(word, Some("bar".to_string()));
    }

    #[test]
    fn test_rename_cross_file_module_scope_fn() {
        use crate::document::DocumentManager;
        let docs = DocumentManager::new();
        let main_text = "fn shared() { return 1 }\nlet a = shared()".to_string();
        let other_text = "fn shared() { return 2 }\nlet b = shared() + shared()".to_string();
        let main_uri = Uri::from_file_path("/main.shape").unwrap();
        let other_uri = Uri::from_file_path("/other.shape").unwrap();
        docs.open(main_uri.clone(), 1, main_text.clone());
        docs.open(other_uri.clone(), 1, other_text);

        let pos = Position {
            line: 0,
            character: 3,
        };
        let edit = rename_cross_file(
            &main_text,
            &main_uri,
            pos,
            "renamed",
            None,
            Some(&docs),
            None,
            None,
        )
        .expect("rename should produce a WorkspaceEdit");

        let changes = edit.changes.expect("changes map");
        assert!(
            changes.contains_key(&main_uri),
            "main uri must be in changes"
        );
        assert!(
            changes.contains_key(&other_uri),
            "other uri must be in changes for cross-file rename"
        );
        let other_edits = &changes[&other_uri];
        // other.shape: def + 2 refs = 3 edits
        assert!(
            other_edits.len() >= 3,
            "expected ≥3 edits in /other.shape (def + 2 refs), got {}",
            other_edits.len()
        );
        for te in other_edits {
            assert_eq!(te.new_text, "renamed");
        }
    }

    #[test]
    fn test_rename_cross_file_local_binding_no_crossover() {
        use crate::document::DocumentManager;
        let docs = DocumentManager::new();
        let main_text =
            "fn outer() {\n  let local = 1\n  return local + local\n}".to_string();
        let other_text =
            "fn other() {\n  let local = 5\n  return local\n}".to_string();
        let main_uri = Uri::from_file_path("/main.shape").unwrap();
        let other_uri = Uri::from_file_path("/other.shape").unwrap();
        docs.open(main_uri.clone(), 1, main_text.clone());
        docs.open(other_uri.clone(), 1, other_text);

        let offset = main_text.find("local").unwrap();
        let (line, col) = offset_to_line_col(&main_text, offset);
        let edit = rename_cross_file(
            &main_text,
            &main_uri,
            Position {
                line,
                character: col,
            },
            "new_local",
            None,
            Some(&docs),
            None,
            None,
        )
        .expect("rename should produce edits for local scope");

        let changes = edit.changes.expect("changes");
        assert!(changes.contains_key(&main_uri));
        assert!(
            !changes.contains_key(&other_uri),
            "local-scope `local` rename must NOT touch /other.shape"
        );
    }

    #[test]
    fn test_offset_to_line_col() {
        let text = "line1\nline2\nline3";

        assert_eq!(offset_to_line_col(text, 0), (0, 0));
        assert_eq!(offset_to_line_col(text, 3), (0, 3));
        assert_eq!(offset_to_line_col(text, 6), (1, 0));
        assert_eq!(offset_to_line_col(text, 9), (1, 3));
    }
}
