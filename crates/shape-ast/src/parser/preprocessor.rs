/// Automatic semicolon insertion preprocessor.
///
/// Resolves newline ambiguities where `[...]` or `(...)` on a new line could
/// be parsed as index access or function call (Pest's greedy postfix matching).
///
/// Inserts `;` at the end of lines where:
/// 1. The line ends with a statement-ending token (identifier char, `)`, `]`, `}`, `"`)
/// 2. The next non-empty line starts with `[` or `(`
///
/// This mirrors Go's automatic semicolon insertion strategy.
pub fn preprocess_semicolons(source: &str) -> String {
    let lines: Vec<&str> = source.split('\n').collect();
    if lines.len() <= 1 {
        return source.to_string();
    }

    let mut result = String::with_capacity(source.len() + 64);
    let mut in_block_comment = false;
    let mut in_triple_string = false;

    for i in 0..lines.len() {
        let line = lines[i];

        // Determine the effective last character on this line, skipping comments/strings
        let last_char = effective_last_char(line, &mut in_block_comment, &mut in_triple_string);

        let needs_semicolon = if let Some(ch) = last_char {
            is_statement_ender(ch) && next_nonblank_starts_with_bracket_or_paren(&lines, i + 1)
        } else {
            false
        };

        result.push_str(line);
        if needs_semicolon {
            result.push(';');
        }
        if i < lines.len() - 1 {
            result.push('\n');
        }
    }

    result
}

/// Returns the last significant (non-whitespace, non-comment) character on a line,
/// while tracking block comment and triple-string state across lines.
fn effective_last_char(
    line: &str,
    in_block_comment: &mut bool,
    in_triple_string: &mut bool,
) -> Option<char> {
    let mut last_significant: Option<char> = None;
    let mut in_simple_string = false;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let ch = bytes[i] as char;

        // Inside triple-quoted string — scan for closing """
        if *in_triple_string {
            if ch == '"' && i + 2 < len && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                *in_triple_string = false;
                last_significant = Some('"');
                i += 3;
            } else {
                i += 1;
            }
            continue;
        }

        // Inside block comment — scan for closing */
        if *in_block_comment {
            if ch == '*' && i + 1 < len && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // Inside simple (single-line) string
        if in_simple_string {
            if ch == '\\' {
                i += 2; // skip escaped char
            } else if ch == '"' {
                in_simple_string = false;
                last_significant = Some('"');
                i += 1;
            } else {
                i += 1;
            }
            continue;
        }

        // Not in any special context
        match ch {
            '"' => {
                // Check for triple-quote opening
                if i + 2 < len && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                    *in_triple_string = true;
                    i += 3;
                } else {
                    in_simple_string = true;
                    i += 1;
                }
            }
            '/' => {
                if i + 1 < len && bytes[i + 1] == b'/' {
                    // Line comment — rest of line is comment
                    break;
                } else if i + 1 < len && bytes[i + 1] == b'*' {
                    *in_block_comment = true;
                    i += 2;
                } else {
                    last_significant = Some(ch);
                    i += 1;
                }
            }
            _ => {
                if !ch.is_ascii_whitespace() {
                    last_significant = Some(ch);
                }
                i += 1;
            }
        }
    }

    last_significant
}

/// Whether a character at end-of-line indicates a complete statement.
fn is_statement_ender(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == ')' || ch == ']' || ch == '}' || ch == '"'
}

/// Check if any of the next non-blank lines (starting at `from`) begins with `[` or `(`.
fn next_nonblank_starts_with_bracket_or_paren(lines: &[&str], from: usize) -> bool {
    for i in from..lines.len() {
        let trimmed = lines[i].trim();
        if !trimmed.is_empty() {
            return trimmed.starts_with('[') || trimmed.starts_with('(');
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_after_identifier_before_bracket() {
        let input = "let x = foo\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, "let x = foo;\n[1, 2]");
    }

    #[test]
    fn test_insert_after_paren_before_bracket() {
        let input = "let m = HashMap().set(\"x\", None)\n[m.has(\"x\"), m.get(\"x\")]";
        let output = preprocess_semicolons(input);
        assert!(
            output.contains(");\n["),
            "should insert ; after closing paren"
        );
    }

    #[test]
    fn test_no_insert_when_line_ends_with_comma() {
        let input = "foo(a,\n[1, 2])";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "comma means continuation");
    }

    #[test]
    fn test_no_insert_when_line_ends_with_dot() {
        let input = "foo.\n[0]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "dot means method chain continuation");
    }

    #[test]
    fn test_no_insert_when_next_line_not_bracket() {
        let input = "let x = 5\nlet y = 10";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "no bracket on next line");
    }

    #[test]
    fn test_skips_blank_lines() {
        let input = "let x = foo\n\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, "let x = foo;\n\n[1, 2]");
    }

    #[test]
    fn test_line_comment_stripped() {
        let input = "let x = foo // comment\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, "let x = foo // comment;\n[1, 2]");
    }

    #[test]
    fn test_block_comment_tracked() {
        // Line ends inside block comment — no insertion
        let input = "let x = foo /* start\nend */ [1, 2]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "inside block comment");
    }

    #[test]
    fn test_string_not_confused_with_comment() {
        let input = "let x = \"//not a comment\"\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert!(
            output.contains("\";\n["),
            "string ending with quote is a statement ender"
        );
    }

    #[test]
    fn test_closing_bracket_before_bracket() {
        let input = "let a = [10, 20, 30]\n[a.first(), a.last()]";
        let output = preprocess_semicolons(input);
        assert!(output.contains("];\n["), "closing ] is a statement ender");
    }

    #[test]
    fn test_closing_brace_before_bracket() {
        let input = "let f = { x: 1 }\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert!(output.contains("};\n["), "closing }} is a statement ender");
    }

    #[test]
    fn test_no_insert_after_operator() {
        let input = "let x = a +\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "+ means expression continues");
    }

    #[test]
    fn test_single_line_unchanged() {
        let input = "let x = [1, 2, 3]";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(preprocess_semicolons(""), "");
    }

    #[test]
    fn test_no_insert_inside_triple_string() {
        // Content inside """ """ should not trigger insertion
        let input = "let s = \"\"\"\nfoo\n[bar]\n\"\"\"\n[1, 2]";
        let output = preprocess_semicolons(input);
        // The semicolon should be inserted after the closing """ line, not inside
        assert!(
            output.contains("\"\"\";\n[1, 2]"),
            "semicolon after triple string close, got: {}",
            output
        );
        // No semicolon on the lines inside the triple string
        assert!(
            !output.contains("foo;\n"),
            "no insertion inside triple string"
        );
    }

    #[test]
    fn test_insert_before_paren_on_new_line() {
        let input = "let b = Pt { x: 10.0, y: 20.0 }\n(a + b).x";
        let output = preprocess_semicolons(input);
        assert_eq!(output, "let b = Pt { x: 10.0, y: 20.0 };\n(a + b).x");
    }

    #[test]
    fn test_insert_before_paren_after_identifier() {
        let input = "let dy = self.y2 - self.y1\n(dx * dx + dy * dy)";
        let output = preprocess_semicolons(input);
        assert_eq!(output, "let dy = self.y2 - self.y1;\n(dx * dx + dy * dy)");
    }

    #[test]
    fn test_no_insert_before_paren_after_operator() {
        let input = "let x = a +\n(b + c)";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "+ means expression continues");
    }

    #[test]
    fn test_no_insert_before_paren_after_comma() {
        let input = "foo(a,\n(b + c))";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "comma means continuation");
    }

    #[test]
    fn test_no_insert_before_paren_after_equals() {
        let input = "let x =\n(1 + 2)";
        let output = preprocess_semicolons(input);
        assert_eq!(output, input, "= means assignment continues");
    }

    #[test]
    fn test_real_hashmap_pattern() {
        let input = r#"let m = HashMap().set("x", None)
[m.has("x"), m.get("x") == None]"#;
        let expected = r#"let m = HashMap().set("x", None);
[m.has("x"), m.get("x") == None]"#;
        assert_eq!(preprocess_semicolons(input), expected);
    }

    // --- Multiline triple-string tests ---

    #[test]
    fn test_triple_string_multiline_with_bracket_inside() {
        // A triple string spanning multiple lines with [ inside should not
        // cause semicolon insertion anywhere inside the string.
        let input = r#"let s = """
    this has
    [brackets inside]
    the string
"""
let x = 5"#;
        let output = preprocess_semicolons(input);
        // No semicolons should be inserted inside the triple string
        assert!(
            !output.contains("has;\n"),
            "no insertion inside triple string"
        );
        assert!(
            !output.contains("inside];\n"),
            "no insertion inside triple string before brackets"
        );
        assert_eq!(output, input, "no changes needed here");
    }

    #[test]
    fn test_triple_string_ending_then_array_on_next_line() {
        // Triple string on one line followed by array literal
        let input = "let s = \"\"\"hello\"\"\"\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert!(
            output.contains("\"\"\";\n[1, 2]"),
            "semicolon after triple string close before [, got: {}",
            output
        );
    }

    #[test]
    fn test_triple_string_multiline_close_then_array() {
        // Multi-line triple string where the closing """ is on its own line,
        // followed by an array literal.
        let input = "let s = \"\"\"\n    content\n    \"\"\"\n[1, 2]";
        let output = preprocess_semicolons(input);
        assert!(
            output.contains("\"\"\";\n[1, 2]"),
            "semicolon after closing triple-quote line, got: {}",
            output
        );
    }

    #[test]
    fn test_triple_string_with_indented_bracket_lines() {
        // Simulates the user's exact multiline string example followed by an array
        let input = "let a_str = \"\"\"\n            this is\n            a multiline\n            string.\n              -it should indent\n            \"\"\"\n[a_str.length]";
        let output = preprocess_semicolons(input);
        // Semicolon only after the closing """
        assert!(
            output.contains("\"\"\";\n[a_str"),
            "semicolon after triple string, got: {}",
            output
        );
        // No semicolons inside the string content
        assert!(!output.contains("is;\n"), "no insertion inside string");
        assert!(!output.contains("indent;\n"), "no insertion inside string");
    }

    #[test]
    fn test_formatted_triple_string_tracked() {
        // f""" ... """ should also be tracked (the f prefix is before the quotes)
        let input = "let s = f\"\"\"\n    value: {x}\n    [y]\n    \"\"\"\n[1, 2]";
        let output = preprocess_semicolons(input);
        // No insertion inside the formatted triple string
        assert!(
            !output.contains("{x};\n"),
            "no insertion inside f-triple string"
        );
        // Semicolon after the closing """
        assert!(
            output.contains("\"\"\";\n[1, 2]"),
            "semicolon after f-triple string close, got: {}",
            output
        );
    }

    #[test]
    fn test_multiple_triple_strings_in_sequence() {
        let input = "let a = \"\"\"\n  [inside a]\n  \"\"\"\nlet b = \"\"\"\n  [inside b]\n  \"\"\"\n[1, 2]";
        let output = preprocess_semicolons(input);
        // No insertion inside either triple string
        assert!(
            !output.contains("a];\n"),
            "no insertion inside first string"
        );
        assert!(
            !output.contains("b];\n"),
            "no insertion inside second string"
        );
        // Semicolon before the final array
        assert!(
            output.contains("\"\"\";\n[1, 2]"),
            "semicolon before final array, got: {}",
            output
        );
    }
}
