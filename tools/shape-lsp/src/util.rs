use std::borrow::Cow;

use shape_ast::ast::Span;
use shape_runtime::frontmatter::parse_frontmatter;
use tower_lsp_server::ls_types::{Position, Range};

/// Masks a leading byte prefix with spaces while preserving newlines.
///
/// This is used to keep parser offsets stable when a script contains
/// frontmatter (`--- ... ---`) that should not be parsed as Shape code.
pub(crate) fn mask_leading_prefix_for_parse<'a>(text: &'a str, prefix_len: usize) -> Cow<'a, str> {
    if prefix_len == 0 {
        return Cow::Borrowed(text);
    }

    let mut bytes = text.as_bytes().to_vec();
    let end = prefix_len.min(bytes.len());
    for b in &mut bytes[..end] {
        if *b != b'\n' && *b != b'\r' {
            *b = b' ';
        }
    }

    Cow::Owned(String::from_utf8(bytes).expect("masked source must remain valid UTF-8"))
}

/// Returns a parser-safe view of source by masking frontmatter bytes.
pub(crate) fn parser_source(text: &str) -> Cow<'_, str> {
    let (_frontmatter, rest) = parse_frontmatter(text);
    let prefix_len = text.len().saturating_sub(rest.len());
    mask_leading_prefix_for_parse(text, prefix_len)
}

/// Convert a byte offset in `text` to 0-based (line, column) as `u32`.
pub(crate) fn offset_to_line_col(text: &str, offset: usize) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Convert a byte offset to an LSP `Position`.
pub(crate) fn offset_to_position(text: &str, offset: usize) -> Position {
    let (line, character) = offset_to_line_col(text, offset);
    Position { line, character }
}

/// Convert an LSP `Position` to a byte offset. Returns `None` if the
/// position is past the end of the text.
pub(crate) fn position_to_offset(text: &str, position: Position) -> Option<usize> {
    let mut offset = 0;
    let mut line = 0;
    let mut col = 0;

    for (i, ch) in text.char_indices() {
        if line == position.line as usize && col == position.character as usize {
            return Some(i);
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
        offset = i + ch.len_utf8();
    }

    // Handle position at end of text
    if line == position.line as usize && col == position.character as usize {
        return Some(offset);
    }

    None
}

/// Extract the identifier (word) at an LSP position.
pub(crate) fn get_word_at_position(text: &str, position: Position) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if position.line as usize >= lines.len() {
        return None;
    }

    let line = lines[position.line as usize];
    let char_pos = position.character as usize;

    if char_pos > line.len() {
        return None;
    }

    let mut start = char_pos;
    let mut end = char_pos;

    while start > 0 {
        let ch = line.as_bytes().get(start - 1).copied()?;
        if (ch as char).is_alphanumeric() || ch == b'_' {
            start -= 1;
        } else {
            break;
        }
    }

    while end < line.len() {
        let ch = line.as_bytes().get(end).copied()?;
        if (ch as char).is_alphanumeric() || ch == b'_' {
            end += 1;
        } else {
            break;
        }
    }

    if start == end {
        return None;
    }

    Some(line[start..end].to_string())
}

/// Convert an AST `Span` to an LSP `Range`.
pub(crate) fn span_to_range(text: &str, span: &Span) -> Range {
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn parser_source_masks_frontmatter_block() {
        let source = r#"---
# shape.toml
[[extensions]]
name = "duckdb"
path = "./extensions/libshape_ext_duckdb.so"
---

let conn = duckdb.connect("duckdb://analytics.db")
"#;

        assert!(parse_program(source).is_err());

        let parse_src = parser_source(source);
        assert_eq!(parse_src.len(), source.len());
        assert!(parse_program(parse_src.as_ref()).is_ok());
    }

    #[test]
    fn parser_source_is_passthrough_without_frontmatter() {
        let source = "let x = 1\nprint(x)\n";
        let parse_src = parser_source(source);
        assert_eq!(parse_src.as_ref(), source);
    }
}
