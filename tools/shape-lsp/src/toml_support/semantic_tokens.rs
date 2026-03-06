//! Semantic token support for script frontmatter (`--- ... ---`).
//!
//! Frontmatter is TOML-like metadata at the top of `.shape` files. This module
//! provides lightweight lexical tokens for section headers, keys, strings,
//! numbers, booleans, comments, and delimiter lines.

use shape_runtime::frontmatter::parse_frontmatter_validated;

/// Absolute semantic token entry for frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontmatterSemanticToken {
    pub line: u32,
    pub start_char: u32,
    pub length: u32,
    pub token_type: u32,
    pub modifiers: u32,
}

const TOKEN_NAMESPACE: u32 = 0;
const TOKEN_PROPERTY: u32 = 7;
const TOKEN_KEYWORD: u32 = 8;
const TOKEN_STRING: u32 = 9;
const TOKEN_NUMBER: u32 = 10;
const TOKEN_OPERATOR: u32 = 11;
const TOKEN_COMMENT: u32 = 12;

/// Collect semantic tokens for the frontmatter block of a source file.
pub fn collect_frontmatter_semantic_tokens(source: &str) -> Vec<FrontmatterSemanticToken> {
    let (_config, _diagnostics, rest) = parse_frontmatter_validated(source);
    let prefix_len = source.len().saturating_sub(rest.len());
    if prefix_len == 0 {
        return Vec::new();
    }

    let frontmatter = &source[..prefix_len];
    let mut tokens = Vec::new();

    for (line_idx, raw_line) in frontmatter.split('\n').enumerate() {
        let line = raw_line.trim_end_matches('\r');
        let line_no = line_idx as u32;
        tokenize_frontmatter_line(line, line_no, &mut tokens);
    }

    tokens
}

fn tokenize_frontmatter_line(line: &str, line_no: u32, out: &mut Vec<FrontmatterSemanticToken>) {
    if line.is_empty() {
        return;
    }

    let comment_start = find_comment_start(line);
    let content_end = comment_start.unwrap_or(line.len());
    let content = &line[..content_end];

    if let Some(comment_byte) = comment_start {
        let start = byte_to_char(line, comment_byte);
        let len = line.chars().count() as u32 - start;
        if len > 0 {
            push_token(out, line_no, start, len, TOKEN_COMMENT);
        }
    }

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }

    if trimmed == "---" {
        if let Some(byte) = content.find("---") {
            push_token(out, line_no, byte_to_char(content, byte), 3, TOKEN_KEYWORD);
        }
        return;
    }

    if is_section_header(trimmed) {
        if let Some((inner_start_byte, inner_end_byte)) = section_inner_range(content, trimmed) {
            for (seg_start, seg_end) in dotted_segments(content, inner_start_byte, inner_end_byte) {
                let start = byte_to_char(content, seg_start);
                let len = (content[seg_start..seg_end]).chars().count() as u32;
                if len > 0 {
                    push_token(out, line_no, start, len, TOKEN_NAMESPACE);
                }
            }
        }
        return;
    }

    if let Some(eq_byte) = content.find('=') {
        tokenize_key_value_line(content, line_no, eq_byte, out);
    }
}

fn tokenize_key_value_line(
    line: &str,
    line_no: u32,
    eq_byte: usize,
    out: &mut Vec<FrontmatterSemanticToken>,
) {
    let key_part = &line[..eq_byte];
    let value_part = &line[eq_byte + 1..];

    let key_trimmed = key_part.trim();
    if !key_trimmed.is_empty()
        && let Some(key_rel_byte) = key_part.find(key_trimmed)
    {
        let key_start_byte = key_rel_byte;
        let key_end_byte = key_start_byte + key_trimmed.len();
        push_token(
            out,
            line_no,
            byte_to_char(line, key_start_byte),
            (line[key_start_byte..key_end_byte]).chars().count() as u32,
            TOKEN_PROPERTY,
        );
    }

    push_token(out, line_no, byte_to_char(line, eq_byte), 1, TOKEN_OPERATOR);

    let value_trimmed = value_part.trim_start();
    if value_trimmed.is_empty() {
        return;
    }
    let value_offset = value_part.len() - value_trimmed.len();
    let value_start_byte = eq_byte + 1 + value_offset;
    tokenize_value_expression(line, line_no, value_start_byte, value_trimmed, out);
}

fn tokenize_value_expression(
    line: &str,
    line_no: u32,
    value_start_byte: usize,
    value: &str,
    out: &mut Vec<FrontmatterSemanticToken>,
) {
    if let Some((str_start, str_end)) = quoted_string_range(value) {
        let start = value_start_byte + str_start;
        let end = value_start_byte + str_end;
        push_token(
            out,
            line_no,
            byte_to_char(line, start),
            (line[start..end]).chars().count() as u32,
            TOKEN_STRING,
        );
        return;
    }

    // Tokenize simple scalar arrays like ["a", "b"].
    if value.starts_with('[') && value.ends_with(']') {
        let mut idx = 1usize;
        let bytes = value.as_bytes();
        while idx + 1 < value.len() {
            while idx + 1 < value.len() && bytes[idx].is_ascii_whitespace() {
                idx += 1;
            }
            if idx + 1 >= value.len() {
                break;
            }
            if bytes[idx] == b',' {
                idx += 1;
                continue;
            }
            let token_start = idx;
            let token_type = if bytes[idx] == b'"' || bytes[idx] == b'\'' {
                let quote = bytes[idx];
                idx += 1;
                while idx + 1 < value.len() {
                    if bytes[idx] == quote && bytes[idx.saturating_sub(1)] != b'\\' {
                        idx += 1;
                        break;
                    }
                    idx += 1;
                }
                TOKEN_STRING
            } else {
                while idx + 1 < value.len()
                    && bytes[idx] != b','
                    && !bytes[idx].is_ascii_whitespace()
                {
                    idx += 1;
                }
                classify_bare_value(&value[token_start..idx])
            };
            let abs_start = value_start_byte + token_start;
            push_token(
                out,
                line_no,
                byte_to_char(line, abs_start),
                (value[token_start..idx]).chars().count() as u32,
                token_type,
            );
        }
        return;
    }

    let token_type = classify_bare_value(value);
    push_token(
        out,
        line_no,
        byte_to_char(line, value_start_byte),
        value.chars().count() as u32,
        token_type,
    );
}

fn classify_bare_value(value: &str) -> u32 {
    let v = value.trim();
    if matches!(v, "true" | "false") {
        return TOKEN_KEYWORD;
    }
    if is_number_literal(v) {
        return TOKEN_NUMBER;
    }
    TOKEN_STRING
}

fn quoted_string_range(value: &str) -> Option<(usize, usize)> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut idx = 1usize;
    while idx < value.len() {
        if bytes[idx] == quote && bytes[idx.saturating_sub(1)] != b'\\' {
            return Some((0, idx + 1));
        }
        idx += 1;
    }
    None
}

fn is_section_header(trimmed: &str) -> bool {
    (trimmed.starts_with("[[") && trimmed.ends_with("]]"))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

fn section_inner_range(line: &str, trimmed: &str) -> Option<(usize, usize)> {
    let start = line.find(trimmed)?;
    if trimmed.starts_with("[[") {
        Some((start + 2, start + trimmed.len() - 2))
    } else {
        Some((start + 1, start + trimmed.len() - 1))
    }
}

fn dotted_segments(line: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut seg_start = start;
    let bytes = line.as_bytes();
    let mut idx = start;
    while idx <= end {
        let at_end = idx == end;
        let at_dot = !at_end && bytes[idx] == b'.';
        if at_end || at_dot {
            if seg_start < idx {
                out.push((seg_start, idx));
            }
            seg_start = idx + 1;
        }
        idx += 1;
    }
    out
}

fn is_number_literal(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let mut has_digit = false;
    for (idx, ch) in value.chars().enumerate() {
        if ch.is_ascii_digit() {
            has_digit = true;
            continue;
        }
        if ch == '-' && idx == 0 {
            continue;
        }
        if ch == '.' || ch == '_' {
            continue;
        }
        return false;
    }
    has_digit
}

fn find_comment_start(line: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_escape = false;

    for (idx, ch) in line.char_indices() {
        match ch {
            '\'' if !in_double && !prev_escape => in_single = !in_single,
            '"' if !in_single && !prev_escape => in_double = !in_double,
            '#' if !in_single && !in_double => return Some(idx),
            _ => {}
        }
        prev_escape = ch == '\\' && !prev_escape;
    }
    None
}

fn byte_to_char(text: &str, byte_idx: usize) -> u32 {
    text[..byte_idx.min(text.len())].chars().count() as u32
}

fn push_token(
    out: &mut Vec<FrontmatterSemanticToken>,
    line: u32,
    start_char: u32,
    length: u32,
    token_type: u32,
) {
    if length == 0 {
        return;
    }
    out.push(FrontmatterSemanticToken {
        line,
        start_char,
        length,
        token_type,
        modifiers: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_lexeme(source: &str, token: &FrontmatterSemanticToken) -> String {
        let line = source.lines().nth(token.line as usize).unwrap_or_default();
        let mut start_byte = 0usize;
        let mut end_byte = line.len();
        let mut chars_seen = 0u32;
        for (idx, ch) in line.char_indices() {
            if chars_seen == token.start_char {
                start_byte = idx;
                break;
            }
            chars_seen += 1;
            if chars_seen >= token.start_char {
                start_byte = idx + ch.len_utf8();
                break;
            }
        }
        chars_seen = 0;
        for (idx, ch) in line.char_indices() {
            if chars_seen == token.start_char + token.length {
                end_byte = idx;
                break;
            }
            chars_seen += 1;
            if chars_seen > token.start_char + token.length {
                end_byte = idx;
                break;
            }
            end_byte = idx + ch.len_utf8();
        }
        line[start_byte.min(line.len())..end_byte.min(line.len())].to_string()
    }

    #[test]
    fn test_collect_frontmatter_semantic_tokens_basic() {
        let source = r#"---
[[extensions]]
name = "python"
path = "/tmp/libshape_ext_python.so" # comment
---
let x = 1
"#;
        let tokens = collect_frontmatter_semantic_tokens(source);
        assert!(!tokens.is_empty(), "expected frontmatter tokens");

        let lexemes: Vec<(u32, String, u32)> = tokens
            .iter()
            .map(|t| (t.line, token_lexeme(source, t), t.token_type))
            .collect();

        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 0 && lex == "---" && *ty == TOKEN_KEYWORD)
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 1 && lex == "extensions" && *ty == TOKEN_NAMESPACE)
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 2 && lex == "name" && *ty == TOKEN_PROPERTY)
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 2 && lex == "\"python\"" && *ty == TOKEN_STRING)
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 3 && lex == "path" && *ty == TOKEN_PROPERTY)
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 3 && lex == "# comment" && *ty == TOKEN_COMMENT)
        );

        assert!(
            tokens.iter().all(|t| t.line <= 4),
            "frontmatter tokenization should not include body lines: {:?}",
            tokens
        );
    }
}
