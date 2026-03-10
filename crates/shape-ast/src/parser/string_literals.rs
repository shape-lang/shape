//! String literal decoding helpers.
//!
//! Supports:
//! - simple strings: `"text"`
//! - triple strings: `"""multiline"""`

use crate::ast::InterpolationMode;
use crate::error::{Result, ShapeError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStringLiteral {
    pub value: String,
    pub interpolation_mode: Option<InterpolationMode>,
    /// `true` when the source used a `c` prefix (content string).
    pub is_content: bool,
}

/// Decode a parsed string literal (including surrounding quotes) into its runtime content.
pub fn parse_string_literal(raw: &str) -> Result<String> {
    Ok(parse_string_literal_with_kind(raw)?.value)
}

/// Decode a parsed string literal and report whether it used the `f` or `c` prefix.
pub fn parse_string_literal_with_kind(raw: &str) -> Result<ParsedStringLiteral> {
    let (interpolation_mode, is_content, unprefixed) = strip_interpolation_prefix(raw);
    let is_interpolated = interpolation_mode.is_some();
    let value = if is_triple_quoted(unprefixed) {
        parse_triple_quoted(unprefixed)
    } else if is_simple_quoted(unprefixed) {
        parse_simple_quoted(&unprefixed[1..unprefixed.len() - 1], is_interpolated)?
    } else {
        unprefixed.to_string()
    };
    Ok(ParsedStringLiteral {
        value,
        interpolation_mode,
        is_content,
    })
}

/// Strip `f`/`f$`/`f#`/`c`/`c$`/`c#` prefix and return (mode, is_content, rest).
fn strip_interpolation_prefix(raw: &str) -> (Option<InterpolationMode>, bool, &str) {
    // Try f-string prefixes first (higher priority)
    if raw.starts_with("f$") && raw.get(2..).is_some_and(|rest| rest.starts_with('"')) {
        (Some(InterpolationMode::Dollar), false, &raw[2..])
    } else if raw.starts_with("f#") && raw.get(2..).is_some_and(|rest| rest.starts_with('"')) {
        (Some(InterpolationMode::Hash), false, &raw[2..])
    } else if raw.starts_with('f') && raw.get(1..).is_some_and(|rest| rest.starts_with('"')) {
        (Some(InterpolationMode::Braces), false, &raw[1..])
    }
    // Then c-string prefixes
    else if raw.starts_with("c$") && raw.get(2..).is_some_and(|rest| rest.starts_with('"')) {
        (Some(InterpolationMode::Dollar), true, &raw[2..])
    } else if raw.starts_with("c#") && raw.get(2..).is_some_and(|rest| rest.starts_with('"')) {
        (Some(InterpolationMode::Hash), true, &raw[2..])
    } else if raw.starts_with('c') && raw.get(1..).is_some_and(|rest| rest.starts_with('"')) {
        (Some(InterpolationMode::Braces), true, &raw[1..])
    } else {
        (None, false, raw)
    }
}

fn is_simple_quoted(raw: &str) -> bool {
    raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"')
}

fn is_triple_quoted(raw: &str) -> bool {
    raw.len() >= 6 && raw.starts_with("\"\"\"") && raw.ends_with("\"\"\"")
}

fn parse_triple_quoted(raw: &str) -> String {
    // Normalize line endings first so trimming rules are deterministic.
    let normalized = raw[3..raw.len() - 3].replace("\r\n", "\n");
    let mut lines: Vec<&str> = normalized.split('\n').collect();

    // Ignore delimiter-adjacent blank lines when they only contain whitespace.
    if lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let common_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| leading_indent(line))
        .min()
        .unwrap_or(0);

    lines
        .into_iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                line.chars().skip(common_indent).collect()
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Decode escape sequences in a simple quoted string.
///
/// When `preserve_brace_escapes` is true (for f-strings / c-strings), `\{` and
/// `\}` are kept as-is so the downstream interpolation parser can treat them as
/// literal brace escapes rather than interpolation delimiters.
fn parse_simple_quoted(inner: &str, preserve_brace_escapes: bool) -> Result<String> {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let Some(escaped) = chars.next() else {
            out.push('\\');
            break;
        };

        match escaped {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '{' | '}' | '$' | '#' if preserve_brace_escapes => {
                // Keep `\{`, `\}`, `\$`, `\#` verbatim for the interpolation parser
                out.push('\\');
                out.push(escaped);
            }
            '{' => out.push('{'),
            '}' => out.push('}'),
            '$' => out.push('$'),
            '#' => out.push('#'),
            other => {
                return Err(ShapeError::ParseError {
                    message: format!(
                        "unknown escape sequence '\\{}', expected one of: \\n, \\t, \\r, \\\\, \\\", \\', \\0, \\{{, \\}}, \\$, \\#",
                        other
                    ),
                    location: None,
                });
            }
        }
    }

    Ok(out)
}

fn leading_indent(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

#[cfg(test)]
mod tests {
    use super::{parse_string_literal, parse_string_literal_with_kind};
    use crate::ast::InterpolationMode;

    #[test]
    fn simple_string_is_unwrapped() {
        assert_eq!(parse_string_literal("\"hello\"").unwrap(), "hello");
    }

    #[test]
    fn triple_string_trims_delimiter_blank_lines_and_dedent() {
        let raw = "\"\"\"\n        this\n        is\n        a\n        multiline\n        \"\"\"";
        assert_eq!(parse_string_literal(raw).unwrap(), "this\nis\na\nmultiline");
    }

    #[test]
    fn triple_string_preserves_relative_indentation() {
        let raw =
            "\"\"\"\n            root\n              nested\n            end\n            \"\"\"";
        assert_eq!(parse_string_literal(raw).unwrap(), "root\n  nested\nend");
    }

    #[test]
    fn triple_string_keeps_inline_form() {
        let raw = "\"\"\"a\n  b\"\"\"";
        assert_eq!(parse_string_literal(raw).unwrap(), "a\n  b");
    }

    #[test]
    fn formatted_simple_string_sets_formatted_flag() {
        let parsed = parse_string_literal_with_kind("f\"value: {x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        assert_eq!(parsed.value, "value: {x}");
    }

    #[test]
    fn formatted_triple_string_sets_formatted_flag() {
        let parsed = parse_string_literal_with_kind("f\"\"\"\n  x\n\"\"\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        assert_eq!(parsed.value, "x");
    }

    #[test]
    fn formatted_triple_string_preserves_relative_indentation() {
        let parsed = parse_string_literal_with_kind(
            "f\"\"\"\n            value:\n              {33+1}\n            \"\"\"",
        )
        .unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        assert_eq!(parsed.value, "value:\n  {33+1}");
    }

    #[test]
    fn formatted_dollar_prefix_sets_mode() {
        let parsed = parse_string_literal_with_kind("f$\"value: ${x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Dollar));
        assert_eq!(parsed.value, "value: ${x}");
    }

    #[test]
    fn formatted_hash_prefix_sets_mode() {
        let parsed = parse_string_literal_with_kind("f#\"value: #{x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Hash));
        assert_eq!(parsed.value, "value: #{x}");
    }

    #[test]
    fn simple_string_decodes_common_escapes() {
        let parsed = parse_string_literal_with_kind("\"a\\n\\t\\\"b\\\\c\"").unwrap();
        assert_eq!(parsed.interpolation_mode, None);
        assert_eq!(parsed.value, "a\n\t\"b\\c");
    }

    // --- User-specified multiline triple-string behavior ---

    #[test]
    fn triple_string_multiline_with_relative_indent() {
        let raw = "\"\"\"\n            this is\n            a multiline\n            string.\n              -it should indent\n              -but remove the block spaces\n            \"\"\"";
        assert_eq!(
            parse_string_literal(raw).unwrap(),
            "this is\na multiline\nstring.\n  -it should indent\n  -but remove the block spaces"
        );
    }

    #[test]
    fn triple_string_inline_with_inner_quotes() {
        let raw = "\"\"\"a string with quotes\"\"\"";
        assert_eq!(parse_string_literal(raw).unwrap(), "a string with quotes");
    }

    #[test]
    fn triple_string_inline_with_single_inner_quote() {
        let raw = "\"\"\"she said \"hello\" today\"\"\"";
        assert_eq!(
            parse_string_literal(raw).unwrap(),
            "she said \"hello\" today"
        );
    }

    #[test]
    fn triple_string_no_leading_trailing_newline() {
        let raw = "\"\"\"\n  hello world\n  \"\"\"";
        let result = parse_string_literal(raw).unwrap();
        assert!(
            !result.starts_with('\n'),
            "should not start with newline, got: {:?}",
            result
        );
        assert!(
            !result.ends_with('\n'),
            "should not end with newline, got: {:?}",
            result
        );
        assert_eq!(result, "hello world");
    }

    #[test]
    fn triple_string_empty_lines_preserved_in_middle() {
        let raw = "\"\"\"\n    first\n\n    last\n    \"\"\"";
        assert_eq!(parse_string_literal(raw).unwrap(), "first\n\nlast");
    }

    #[test]
    fn triple_string_does_not_process_escape_sequences() {
        let raw = "\"\"\"\n    line with \\n in it\n    \"\"\"";
        let result = parse_string_literal(raw).unwrap();
        assert_eq!(result, "line with \\n in it");
    }

    #[test]
    fn simple_string_escape_newline() {
        assert_eq!(
            parse_string_literal("\"hello\\nworld\"").unwrap(),
            "hello\nworld"
        );
    }

    #[test]
    fn simple_string_escape_tab() {
        assert_eq!(
            parse_string_literal("\"col1\\tcol2\"").unwrap(),
            "col1\tcol2"
        );
    }

    #[test]
    fn simple_string_escape_backslash() {
        assert_eq!(
            parse_string_literal("\"path\\\\file\"").unwrap(),
            "path\\file"
        );
    }

    #[test]
    fn simple_string_escape_quote() {
        assert_eq!(
            parse_string_literal("\"say \\\"hi\\\"\"").unwrap(),
            "say \"hi\""
        );
    }

    #[test]
    fn simple_string_unknown_escape_is_error() {
        // BUG-12: Unknown escape sequences must produce an error
        let result = parse_string_literal("\"hello\\q\"");
        assert!(result.is_err(), "expected error for unknown escape \\q");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unknown escape sequence"),
            "error should mention 'unknown escape sequence', got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("\\q"),
            "error should mention the bad escape \\q, got: {}",
            err_msg
        );
    }

    #[test]
    fn simple_string_unknown_escape_x_is_error() {
        // \x is not a supported escape sequence (no hex escape support yet)
        let result = parse_string_literal("\"\\x41\"");
        assert!(result.is_err(), "expected error for unsupported \\x escape");
    }

    #[test]
    fn simple_string_escape_null() {
        // \0 should produce a null byte
        assert_eq!(parse_string_literal("\"a\\0b\"").unwrap(), "a\0b");
    }

    // --- Content string (c-prefix) tests ---

    #[test]
    fn content_simple_string_sets_content_flag() {
        let parsed = parse_string_literal_with_kind("c\"hello {x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        assert!(parsed.is_content);
        assert_eq!(parsed.value, "hello {x}");
    }

    #[test]
    fn content_dollar_prefix_sets_mode_and_content() {
        let parsed = parse_string_literal_with_kind("c$\"value: ${x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Dollar));
        assert!(parsed.is_content);
        assert_eq!(parsed.value, "value: ${x}");
    }

    #[test]
    fn content_hash_prefix_sets_mode_and_content() {
        let parsed = parse_string_literal_with_kind("c#\"value: #{x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Hash));
        assert!(parsed.is_content);
        assert_eq!(parsed.value, "value: #{x}");
    }

    #[test]
    fn content_triple_string_sets_content_flag() {
        let parsed = parse_string_literal_with_kind("c\"\"\"\n  row: {x}\n\"\"\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        assert!(parsed.is_content);
        assert_eq!(parsed.value, "row: {x}");
    }

    #[test]
    fn formatted_string_is_not_content() {
        let parsed = parse_string_literal_with_kind("f\"value: {x}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        assert!(!parsed.is_content);
    }

    #[test]
    fn plain_string_is_not_content() {
        let parsed = parse_string_literal_with_kind("\"plain\"").unwrap();
        assert_eq!(parsed.interpolation_mode, None);
        assert!(!parsed.is_content);
    }

    // --- LOW-2: f-string backslash-escaped braces ---

    #[test]
    fn fstring_backslash_brace_preserves_literal_brace() {
        // f"hello \{world\}" should produce value with preserved \{ and \}
        // so the interpolation parser sees them as literal braces, not interpolation.
        let parsed = parse_string_literal_with_kind("f\"hello \\{world\\}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, Some(InterpolationMode::Braces));
        // The value should contain `\{` and `\}` so the interpolation parser
        // can distinguish them from real interpolation delimiters.
        assert_eq!(parsed.value, "hello \\{world\\}");
    }

    #[test]
    fn plain_string_backslash_brace_decodes_to_literal() {
        // In a plain (non-interpolated) string, \{ should still decode to {
        let parsed = parse_string_literal_with_kind("\"hello \\{world\\}\"").unwrap();
        assert_eq!(parsed.interpolation_mode, None);
        assert_eq!(parsed.value, "hello {world}");
    }
}
