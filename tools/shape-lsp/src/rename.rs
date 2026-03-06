//! Rename support for Shape
//!
//! Provides symbol renaming across the document using text-based searching.

use crate::util::{get_word_at_position, offset_to_line_col, position_to_offset};
use shape_ast::ast::Program;
use shape_ast::parser::parse_program;
use std::collections::HashMap;
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
        "interface",
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
    fn test_offset_to_line_col() {
        let text = "line1\nline2\nline3";

        assert_eq!(offset_to_line_col(text, 0), (0, 0));
        assert_eq!(offset_to_line_col(text, 3), (0, 3));
        assert_eq!(offset_to_line_col(text, 6), (1, 0));
        assert_eq!(offset_to_line_col(text, 9), (1, 3));
    }
}
