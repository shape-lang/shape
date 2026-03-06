//! Document formatting provider for Shape
//!
//! Provides code formatting with configurable options.
//! Uses a token-level approach that ONLY adjusts whitespace/indentation.
//! Non-whitespace tokens pass through unchanged — code can never be destroyed.

use crate::util::{offset_to_line_col, parser_source, position_to_offset};
use shape_ast::ast::Item;
use tower_lsp_server::ls_types::{FormattingOptions, Position, Range, TextEdit};

/// Formatting configuration
#[derive(Debug, Clone)]
pub struct FormatConfig {
    /// Number of spaces for indentation (or 0 for tabs)
    pub indent_size: u32,
    /// Use tabs instead of spaces
    pub use_tabs: bool,
    /// Maximum line length before wrapping
    pub max_line_length: u32,
    /// Add trailing commas in arrays/objects
    pub trailing_commas: bool,
    /// Space after colons in objects
    pub space_after_colon: bool,
    /// Spaces around binary operators
    pub spaces_around_operators: bool,
    /// Blank lines between top-level items
    pub blank_lines_between_items: u32,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_size: 4,
            use_tabs: false,
            max_line_length: 100,
            trailing_commas: true,
            space_after_colon: true,
            spaces_around_operators: true,
            blank_lines_between_items: 1,
        }
    }
}

impl From<&FormattingOptions> for FormatConfig {
    fn from(opts: &FormattingOptions) -> Self {
        Self {
            indent_size: opts.tab_size,
            use_tabs: !opts.insert_spaces,
            ..Default::default()
        }
    }
}

// ─── Token-level formatter ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Whitespace,    // spaces/tabs (NOT newlines)
    Newline,       // \n or \r\n
    LineComment,   // // ... (not including trailing newline)
    BlockComment,  // /* ... */ (with nesting)
    OpenBrace,     // {
    CloseBrace,    // }
    StringLiteral, // all string varieties, opaque
    Other,         // identifiers, keywords, operators, numbers, parens, brackets
}

struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteRange {
    start: usize,
    end: usize,
}

/// Tokenize source into a flat token stream.
/// Invariant: concatenating all token texts reproduces the original source exactly.
fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let start = i;
        let b = bytes[i];

        // Newline: \r\n or \n
        if b == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            i += 2;
            tokens.push(Token {
                kind: TokenKind::Newline,
                start,
                end: i,
            });
            continue;
        }
        if b == b'\n' {
            i += 1;
            tokens.push(Token {
                kind: TokenKind::Newline,
                start,
                end: i,
            });
            continue;
        }

        // Whitespace (spaces/tabs only)
        if b == b' ' || b == b'\t' {
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Whitespace,
                start,
                end: i,
            });
            continue;
        }

        // Line comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::LineComment,
                start,
                end: i,
            });
            continue;
        }

        // Block comment (with nesting)
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            tokens.push(Token {
                kind: TokenKind::BlockComment,
                start,
                end: i,
            });
            continue;
        }

        // F-string triple: f"""...""", f$"""...""", f#"""..."""
        if b == b'f' {
            let prefix_len = if i + 4 < bytes.len()
                && (bytes[i + 1] == b'$' || bytes[i + 1] == b'#')
                && bytes[i + 2] == b'"'
                && bytes[i + 3] == b'"'
                && bytes[i + 4] == b'"'
            {
                Some(5usize)
            } else if i + 3 < bytes.len()
                && bytes[i + 1] == b'"'
                && bytes[i + 2] == b'"'
                && bytes[i + 3] == b'"'
            {
                Some(4usize)
            } else {
                None
            };
            if let Some(prefix_len) = prefix_len {
                i += prefix_len;
                loop {
                    if i + 2 >= bytes.len() {
                        i = bytes.len();
                        break;
                    }
                    if bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                        i += 3;
                        break;
                    }
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::StringLiteral,
                    start,
                    end: i,
                });
                continue;
            }
        }

        // Triple string: """..."""
        if b == b'"' && i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            i += 3;
            loop {
                if i + 2 >= bytes.len() {
                    i = bytes.len();
                    break;
                }
                if bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                    i += 3;
                    break;
                }
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::StringLiteral,
                start,
                end: i,
            });
            continue;
        }

        // F-string simple: f"...", f$"...", f#"..."
        if b == b'f' {
            let prefix_len = if i + 2 < bytes.len()
                && (bytes[i + 1] == b'$' || bytes[i + 1] == b'#')
                && bytes[i + 2] == b'"'
            {
                Some(3usize)
            } else if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                Some(2usize)
            } else {
                None
            };
            if let Some(prefix_len) = prefix_len {
                i += prefix_len;
                let mut escaped = false;
                while i < bytes.len() {
                    if escaped {
                        escaped = false;
                    } else if bytes[i] == b'\\' {
                        escaped = true;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::StringLiteral,
                    start,
                    end: i,
                });
                continue;
            }
        }

        // Simple string: "..."
        if b == b'"' {
            i += 1;
            let mut escaped = false;
            while i < bytes.len() {
                if escaped {
                    escaped = false;
                } else if bytes[i] == b'\\' {
                    escaped = true;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::StringLiteral,
                start,
                end: i,
            });
            continue;
        }

        // Delimiters
        if b == b'{' {
            i += 1;
            tokens.push(Token {
                kind: TokenKind::OpenBrace,
                start,
                end: i,
            });
            continue;
        }
        if b == b'}' {
            i += 1;
            tokens.push(Token {
                kind: TokenKind::CloseBrace,
                start,
                end: i,
            });
            continue;
        }

        // Other: everything else (identifiers, keywords, operators, parens, brackets, etc.)
        i += 1;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b' '
                || c == b'\t'
                || c == b'\n'
                || c == b'\r'
                || c == b'"'
                || c == b'{'
                || c == b'}'
                || (c == b'/'
                    && i + 1 < bytes.len()
                    && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*'))
            {
                break;
            }
            i += 1;
        }
        tokens.push(Token {
            kind: TokenKind::Other,
            start,
            end: i,
        });
    }

    tokens
}

/// Walk the token stream and emit TextEdits that fix indentation.
/// Only leading whitespace on each line is modified; all other content is untouched.
fn reindent(
    tokens: &[Token],
    source: &str,
    config: &FormatConfig,
    protected_lines: &std::collections::HashSet<u32>,
    protected_ranges: &[ByteRange],
) -> Vec<TextEdit> {
    let mut edits = Vec::new();
    let mut brace_depth: usize = 0;
    let mut line: u32 = 0;
    let mut i = 0;
    let mut after_newline = true; // start of file counts as start of line

    while i < tokens.len() {
        let tok = &tokens[i];

        if after_newline {
            let ws_token = if tok.kind == TokenKind::Whitespace {
                Some(tok)
            } else {
                None
            };

            // Preserve foreign-language body lines exactly as authored.
            if protected_lines.contains(&line) {
                after_newline = false;
                if ws_token.is_some() {
                    i += 1;
                }
                continue;
            }

            let first_non_ws_idx = if ws_token.is_some() { i + 1 } else { i };

            // Peek ahead to find first substantive token on this line
            let mut line_is_blank = true;
            let mut first_code_kind = None;
            if first_non_ws_idx < tokens.len() {
                let pk = &tokens[first_non_ws_idx];
                if pk.kind != TokenKind::Newline {
                    line_is_blank = false;
                    first_code_kind = Some(pk.kind);
                }
            }

            if line_is_blank {
                // Blank line — strip any trailing whitespace
                if let Some(ws) = ws_token {
                    let ws_len = (ws.end - ws.start) as u32;
                    if ws_len > 0 {
                        edits.push(TextEdit {
                            range: Range {
                                start: Position { line, character: 0 },
                                end: Position {
                                    line,
                                    character: ws_len,
                                },
                            },
                            new_text: String::new(),
                        });
                    }
                }
            } else {
                // Compute target indentation
                let effective_depth = if first_code_kind == Some(TokenKind::CloseBrace) {
                    brace_depth.saturating_sub(1)
                } else {
                    brace_depth
                };
                let target_indent = make_indent(effective_depth, config);
                let current_indent = if let Some(ws) = ws_token {
                    &source[ws.start..ws.end]
                } else {
                    ""
                };

                if current_indent != target_indent {
                    let current_len = current_indent.len() as u32;
                    edits.push(TextEdit {
                        range: Range {
                            start: Position { line, character: 0 },
                            end: Position {
                                line,
                                character: current_len,
                            },
                        },
                        new_text: target_indent,
                    });
                }
            }

            after_newline = false;
            if ws_token.is_some() {
                i += 1;
                continue;
            }
        }

        // Track brace depth and line numbers
        match tok.kind {
            TokenKind::OpenBrace => {
                if !is_offset_in_ranges(tok.start, protected_ranges) {
                    brace_depth += 1;
                }
            }
            TokenKind::CloseBrace => {
                if !is_offset_in_ranges(tok.start, protected_ranges) {
                    brace_depth = brace_depth.saturating_sub(1);
                }
            }
            TokenKind::Newline => {
                after_newline = true;
                line += 1;
            }
            TokenKind::StringLiteral | TokenKind::BlockComment => {
                // Multi-line tokens: count internal newlines for line tracking
                let text = &source[tok.start..tok.end];
                line += text.matches('\n').count() as u32;
            }
            _ => {}
        }

        i += 1;
    }

    edits
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Format an entire document
pub fn format_document(text: &str, options: &FormattingOptions) -> Vec<TextEdit> {
    let config = FormatConfig::from(options);
    let tokens = tokenize(text);
    let protected_ranges = collect_foreign_body_ranges(text);
    let protected_lines = collect_protected_lines(text, &protected_ranges);
    reindent(&tokens, text, &config, &protected_lines, &protected_ranges)
}

/// Format a range within a document
pub fn format_range(text: &str, range: Range, options: &FormattingOptions) -> Vec<TextEdit> {
    let config = FormatConfig::from(options);
    let tokens = tokenize(text);
    let protected_ranges = collect_foreign_body_ranges(text);
    let protected_lines = collect_protected_lines(text, &protected_ranges);
    reindent(&tokens, text, &config, &protected_lines, &protected_ranges)
        .into_iter()
        .filter(|edit| {
            edit.range.start.line >= range.start.line && edit.range.end.line <= range.end.line
        })
        .collect()
}

/// Format a document while typing (lightweight indentation-only path).
///
/// This avoids full AST formatting on each keystroke, which keeps on-type
/// formatting responsive for larger files and partially-typed code.
pub fn format_on_type(
    text: &str,
    position: Position,
    ch: &str,
    options: &FormattingOptions,
) -> Vec<TextEdit> {
    if ch != "\n" && ch != "}" {
        return vec![];
    }

    let protected_ranges = collect_foreign_body_ranges(text);
    if let Some(offset) = position_to_offset(text, position)
        && is_offset_in_ranges(offset, &protected_ranges)
    {
        return vec![];
    }

    let line_index = position.line as usize;
    let lines: Vec<&str> = text.split('\n').collect();
    if line_index >= lines.len() {
        return vec![];
    }

    let line = lines[line_index];
    let config = FormatConfig::from(options);

    if let Some(quote_column) = triple_string_quote_column_before_line(text, line_index) {
        if ch == "}" {
            return vec![];
        }

        let target_indent = make_indent_to_column(quote_column, &config);
        return line_indent_edit(line_index, line, target_indent);
    }

    if ch == "}" && !line.trim_start().starts_with('}') {
        return vec![];
    }

    let depth_before_line = brace_depth_before_line(text, line_index, &protected_ranges);
    let leading_closers = line.trim_start().chars().take_while(|c| *c == '}').count();
    let target_depth = depth_before_line.saturating_sub(leading_closers);
    let target_indent = make_indent(target_depth, &config);

    line_indent_edit(line_index, line, target_indent)
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn line_indent_edit(line_index: usize, line: &str, target_indent: String) -> Vec<TextEdit> {
    let current_indent_len = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let current_indent: String = line.chars().take(current_indent_len).collect();

    if current_indent == target_indent {
        return vec![];
    }

    vec![TextEdit {
        range: Range {
            start: Position {
                line: line_index as u32,
                character: 0,
            },
            end: Position {
                line: line_index as u32,
                character: current_indent_len as u32,
            },
        },
        new_text: target_indent,
    }]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScannerStringMode {
    Simple,
    Triple { quote_column: usize },
}

/// If the given line is inside a triple-quoted string, return the column of
/// the opening quote (`"`). Used by on-type formatting to align body/closing
/// lines with the delimiter column.
fn triple_string_quote_column_before_line(text: &str, target_line: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut line = 0usize;
    let mut col = 0usize;
    let mut in_line_comment = false;
    let mut block_comment_depth = 0usize;
    let mut string_mode: Option<ScannerStringMode> = None;
    let mut escaped = false;

    while i < bytes.len() {
        if line >= target_line {
            break;
        }

        let b = bytes[i];

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            i += 1;
            continue;
        }

        if block_comment_depth > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_comment_depth += 1;
                i += 2;
                col += 2;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_comment_depth -= 1;
                i += 2;
                col += 2;
                continue;
            }
            if b == b'\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            i += 1;
            continue;
        }

        if let Some(mode) = string_mode {
            match mode {
                ScannerStringMode::Simple => {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        string_mode = None;
                    }

                    if b == b'\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                    i += 1;
                }
                ScannerStringMode::Triple { .. } => {
                    if i + 2 < bytes.len()
                        && bytes[i] == b'"'
                        && bytes[i + 1] == b'"'
                        && bytes[i + 2] == b'"'
                    {
                        string_mode = None;
                        i += 3;
                        col += 3;
                    } else if b == b'\n' {
                        line += 1;
                        col = 0;
                        i += 1;
                    } else {
                        col += 1;
                        i += 1;
                    }
                }
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            in_line_comment = true;
            i += 2;
            col += 2;
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            block_comment_depth = 1;
            i += 2;
            col += 2;
            continue;
        }

        if i + 3 < bytes.len()
            && bytes[i] == b'f'
            && bytes[i + 1] == b'"'
            && bytes[i + 2] == b'"'
            && bytes[i + 3] == b'"'
        {
            string_mode = Some(ScannerStringMode::Triple {
                quote_column: col + 1,
            });
            i += 4;
            col += 4;
            continue;
        }

        if i + 2 < bytes.len() && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            string_mode = Some(ScannerStringMode::Triple { quote_column: col });
            i += 3;
            col += 3;
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'f' && bytes[i + 1] == b'"' {
            string_mode = Some(ScannerStringMode::Simple);
            escaped = false;
            i += 2;
            col += 2;
            continue;
        }

        if b == b'"' {
            string_mode = Some(ScannerStringMode::Simple);
            escaped = false;
            i += 1;
            col += 1;
            continue;
        }

        if b == b'\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }

        i += 1;
    }

    if line == target_line {
        if let Some(ScannerStringMode::Triple { quote_column }) = string_mode {
            return Some(quote_column);
        }
    }

    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StringMode {
    Simple,
    Triple,
}

/// Compute brace nesting depth before a given 0-based line.
/// Braces inside comments and strings are ignored.
fn brace_depth_before_line(
    text: &str,
    target_line: usize,
    protected_ranges: &[ByteRange],
) -> usize {
    if target_line == 0 {
        return 0;
    }

    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut line = 0usize;
    let mut depth = 0usize;
    let mut in_line_comment = false;
    let mut block_comment_depth = 0usize;
    let mut string_mode: Option<StringMode> = None;
    let mut escaped = false;

    while i < bytes.len() {
        if line >= target_line {
            break;
        }

        if is_offset_in_ranges(i, protected_ranges) {
            if bytes[i] == b'\n' {
                line += 1;
            }
            i += 1;
            continue;
        }

        let b = bytes[i];

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
                line += 1;
            }
            i += 1;
            continue;
        }

        if block_comment_depth > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_comment_depth += 1;
                i += 2;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_comment_depth -= 1;
                i += 2;
                continue;
            }
            if b == b'\n' {
                line += 1;
            }
            i += 1;
            continue;
        }

        if let Some(mode) = string_mode {
            match mode {
                StringMode::Simple => {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        string_mode = None;
                    } else if b == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                StringMode::Triple => {
                    if i + 2 < bytes.len()
                        && bytes[i] == b'"'
                        && bytes[i + 1] == b'"'
                        && bytes[i + 2] == b'"'
                    {
                        string_mode = None;
                        i += 3;
                    } else {
                        if b == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            in_line_comment = true;
            i += 2;
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            block_comment_depth = 1;
            i += 2;
            continue;
        }

        if i + 3 < bytes.len()
            && bytes[i] == b'f'
            && bytes[i + 1] == b'"'
            && bytes[i + 2] == b'"'
            && bytes[i + 3] == b'"'
        {
            string_mode = Some(StringMode::Triple);
            i += 4;
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'f' && bytes[i + 1] == b'"' {
            string_mode = Some(StringMode::Simple);
            escaped = false;
            i += 2;
            continue;
        }

        if i + 2 < bytes.len() && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            string_mode = Some(StringMode::Triple);
            i += 3;
            continue;
        }

        if b == b'"' {
            string_mode = Some(StringMode::Simple);
            escaped = false;
            i += 1;
            continue;
        }

        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth = depth.saturating_sub(1);
        } else if b == b'\n' {
            line += 1;
        }

        i += 1;
    }

    depth
}

fn collect_foreign_body_ranges(source: &str) -> Vec<ByteRange> {
    let mut ranges = Vec::new();
    // Frontmatter (`--- ... ---`) is not Shape syntax; mask it so spans stay
    // byte-aligned while resilient parsing can still discover foreign blocks.
    let parse_source = parser_source(source);
    let partial = shape_ast::parse_program_resilient(parse_source.as_ref());
    for item in partial.items {
        if let Item::ForeignFunction(def, _) = item {
            let start = def.body_span.start.min(source.len());
            let end = def.body_span.end.min(source.len());
            if end > start {
                ranges.push(ByteRange { start, end });
            }
        }
    }
    ranges.sort_by_key(|range| range.start);
    ranges
}

fn collect_protected_lines(source: &str, ranges: &[ByteRange]) -> std::collections::HashSet<u32> {
    let mut lines = std::collections::HashSet::new();
    for range in ranges {
        if range.end <= range.start {
            continue;
        }
        let start_line = offset_to_line_col(source, range.start).0;
        let end_line = offset_to_line_col(source, range.end.saturating_sub(1)).0;
        for line in start_line..=end_line {
            lines.insert(line);
        }
    }
    lines
}

fn is_offset_in_ranges(offset: usize, ranges: &[ByteRange]) -> bool {
    ranges
        .iter()
        .any(|range| offset >= range.start && offset < range.end)
}

/// Create indentation string
fn make_indent(level: usize, config: &FormatConfig) -> String {
    if config.use_tabs {
        "\t".repeat(level)
    } else {
        " ".repeat(level * config.indent_size as usize)
    }
}

fn make_indent_to_column(column: usize, config: &FormatConfig) -> String {
    if config.use_tabs {
        let tab_width = config.indent_size.max(1) as usize;
        let tabs = column / tab_width;
        let spaces = column % tab_width;
        format!("{}{}", "\t".repeat(tabs), " ".repeat(spaces))
    } else {
        " ".repeat(column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply indentation edits to source text (for test assertions).
    fn apply_edits(source: &str, edits: &[TextEdit]) -> String {
        let mut lines: Vec<String> = source.split('\n').map(String::from).collect();
        let mut sorted: Vec<&TextEdit> = edits.iter().collect();
        sorted.sort_by(|a, b| b.range.start.line.cmp(&a.range.start.line));
        for edit in sorted {
            let idx = edit.range.start.line as usize;
            if idx < lines.len() {
                let start = edit.range.start.character as usize;
                let end = edit.range.end.character as usize;
                let line = &lines[idx];
                lines[idx] = format!("{}{}{}", &line[..start], edit.new_text, &line[end..]);
            }
        }
        lines.join("\n")
    }

    #[test]
    fn test_tokenize_roundtrip() {
        let sources = [
            "fn test() {\n    let x = 1\n}\n",
            "let s = \"hello world\"\n",
            "let s = f\"value: {x}\"\n",
            "let s = \"\"\"\ntriple\n\"\"\"\n",
            "let s = f\"\"\"\ntriple {x}\n\"\"\"\n",
            "// line comment\nlet x = 1\n",
            "/* block */ let x = 1\n",
            "/* nested /* inner */ outer */ let x = 1\n",
            "let (d: {x: int}, e: {y: int}) = c\n",
            "",
            "a",
        ];
        for source in &sources {
            let tokens = tokenize(source);
            let reconstructed: String = tokens.iter().map(|t| &source[t.start..t.end]).collect();
            assert_eq!(
                &reconstructed, source,
                "tokenizer roundtrip failed for: {:?}",
                source
            );
        }
    }

    #[test]
    fn test_format_document_preserves_function_keyword() {
        let source = "function add(a, b) { return a + b; }\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        // Token-level formatter preserves `function` keyword as-is (no rename to `fn`)
        // Single-line function — all braces on same line — no indentation changes
        assert_eq!(edits.len(), 0);
    }

    #[test]
    fn test_format_document_preserves_enum_declaration() {
        let source = "enum Signal {\nBuy,\nSell = \"sell\",\nLimit { price: number, size: number },\nMarket(number, number)\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        let result = apply_edits(source, &edits);
        // Content preserved exactly — only indentation adjusted, no trailing comma added
        assert_eq!(
            result,
            "enum Signal {\n    Buy,\n    Sell = \"sell\",\n    Limit { price: number, size: number },\n    Market(number, number)\n}\n"
        );
    }

    #[test]
    fn test_format_document_preserves_triple_style_for_single_line_formatted_string() {
        let source = "function test() {\n  let test3 = f\"\"\"\nbla {33+1}\n\"\"\"\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        // Only the indentation on line 1 changes (2 spaces → 4 spaces).
        // String content on lines 2-3 is inside the StringLiteral token and untouched.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(
            edits[0].range.end,
            Position {
                line: 1,
                character: 2
            }
        );
        assert_eq!(edits[0].new_text, "    ");
    }

    #[test]
    fn test_format_document_preserves_sigil_formatted_strings() {
        let source = "function test() {\n  let a = f$\"{\\\"name\\\": ${user.name}}\";\n  let b = f#\"run #{cmd}\";\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        let result = apply_edits(source, &edits);
        assert!(result.contains("let a = f$\"{\\\"name\\\": ${user.name}}\";"));
        assert!(result.contains("let b = f#\"run #{cmd}\";"));
    }

    #[test]
    fn test_format_document_preserves_blank_lines_in_function_body() {
        let source = "function test() {\nlet a = 1;\n\n\nlet b = 2;\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        let result = apply_edits(source, &edits);
        assert!(
            result.contains("let a = 1;\n\n\n    let b = 2;"),
            "formatter should preserve the original number of blank lines, got:\n{}",
            result
        );
    }

    #[test]
    fn test_format_document_preserves_blank_lines_between_top_level_items() {
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        // Single blank line between items
        let source1 = "let a = 1\n\nlet b = 2\n";
        let edits1 = format_document(source1, &options);
        let result1 = apply_edits(source1, &edits1);
        assert!(
            result1.contains("let a = 1\n\nlet b = 2"),
            "single blank line should be preserved, got:\n{:?}",
            result1
        );

        // Two blank lines between items
        let source2 = "let a = 1\n\n\nlet b = 2\n";
        let edits2 = format_document(source2, &options);
        let result2 = apply_edits(source2, &edits2);
        assert!(
            result2.contains("let a = 1\n\n\nlet b = 2"),
            "two blank lines should be preserved, got:\n{:?}",
            result2
        );

        // Blank lines between functions
        let source3 = "fn foo() {\n    42\n}\n\nfn bar() {\n    43\n}\n";
        let edits3 = format_document(source3, &options);
        let result3 = apply_edits(source3, &edits3);
        assert!(
            result3.contains("}\n\nfn bar"),
            "blank line between functions should be preserved, got:\n{:?}",
            result3
        );
    }

    #[test]
    fn test_format_document_preserves_destructuring() {
        let source = "fn test() {\n    let (d: {x: int}, e: {y: int, z: int}) = c\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };
        let edits = format_document(source, &options);
        assert_eq!(
            edits.len(),
            0,
            "properly indented destructuring should produce no edits"
        );
    }

    #[test]
    fn test_format_document_preserves_as_expression() {
        let source = "fn test() {\n    let (f: TypeA, g: TypeB) = c as (TypeA + TypeB)\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };
        let edits = format_document(source, &options);
        assert_eq!(edits.len(), 0);
    }

    #[test]
    fn test_format_document_preserves_inline_comments() {
        let source = "fn test() {\n    let x = 1 // important\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };
        let edits = format_document(source, &options);
        assert_eq!(
            edits.len(),
            0,
            "inline comments should stay on the same line"
        );
    }

    #[test]
    fn test_format_document_formats_invalid_syntax() {
        let source = "fn test() {\nlet x = @#$\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };
        let edits = format_document(source, &options);
        // Even invalid syntax gets indented (no parse required)
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "    ");
    }

    #[test]
    fn test_format_document_fstring_nested_braces() {
        let source = "fn test() {\nlet x = f\"{obj.method({x: 1})}\"\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };
        let edits = format_document(source, &options);
        // F-string braces are inside the string token and don't affect depth
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "    ");
    }

    #[test]
    fn test_format_document_multiple_closing_braces() {
        let source = "fn outer() {\nfn inner() {\nlet x = 1\n}\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };
        let edits = format_document(source, &options);
        let result = apply_edits(source, &edits);
        assert_eq!(
            result,
            "fn outer() {\n    fn inner() {\n        let x = 1\n    }\n}\n"
        );
    }

    #[test]
    fn test_format_document_preserves_foreign_block_indentation() {
        let source = "fn python percentile(values: Array<number>, pct: number) -> number {\nsorted_v = sorted(values)\nif len(sorted_v) > 0:\n  return sorted_v[0]\nreturn 0.0\n}\nfn shape_fn() {\nlet x = 1\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        let result = apply_edits(source, &edits);
        assert_eq!(
            result,
            "fn python percentile(values: Array<number>, pct: number) -> number {\nsorted_v = sorted(values)\nif len(sorted_v) > 0:\n  return sorted_v[0]\nreturn 0.0\n}\nfn shape_fn() {\n    let x = 1\n}\n"
        );
    }

    #[test]
    fn test_format_document_preserves_foreign_block_indentation_with_frontmatter() {
        let source = r#"---
[[extensions]]
name = "python"
path = "/tmp/libshape_ext_python.so"
---
fn python percentile(values: Array<number>, pct: number) -> number {
  sorted_v = sorted(values)
  if len(sorted_v) > 0:
    return sorted_v[-1]
  return 0.0
}
fn shape_fn() {
let x = 1
}
"#;
        let options = FormattingOptions {
            tab_size: 2,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_document(source, &options);
        let result = apply_edits(source, &edits);
        assert_eq!(
            result,
            r#"---
[[extensions]]
name = "python"
path = "/tmp/libshape_ext_python.so"
---
fn python percentile(values: Array<number>, pct: number) -> number {
  sorted_v = sorted(values)
  if len(sorted_v) > 0:
    return sorted_v[-1]
  return 0.0
}
fn shape_fn() {
  let x = 1
}
"#
        );
    }

    #[test]
    fn test_format_on_type_newline_indents_inside_block() {
        let text = "fn test() {\n\n}";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_on_type(
            text,
            Position {
                line: 1,
                character: 0,
            },
            "\n",
            &options,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].range,
            Range {
                start: Position {
                    line: 1,
                    character: 0
                },
                end: Position {
                    line: 1,
                    character: 0
                }
            }
        );
        assert_eq!(edits[0].new_text, "    ");
    }

    #[test]
    fn test_format_on_type_skips_foreign_block() {
        let text = "fn python p() -> number {\nif True:\n\n    return 1\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_on_type(
            text,
            Position {
                line: 2,
                character: 0,
            },
            "\n",
            &options,
        );

        assert!(edits.is_empty());
    }

    #[test]
    fn test_format_on_type_closing_brace_dedents() {
        let text = "fn test() {\n    let x = 1;\n    }\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_on_type(
            text,
            Position {
                line: 2,
                character: 5,
            },
            "}",
            &options,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].range,
            Range {
                start: Position {
                    line: 2,
                    character: 0
                },
                end: Position {
                    line: 2,
                    character: 4
                }
            }
        );
        assert_eq!(edits[0].new_text, "");
    }

    #[test]
    fn test_format_on_type_ignores_braces_inside_strings() {
        let text = "fn test() {\n    let s = f\"{x}\";\n\n}";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_on_type(
            text,
            Position {
                line: 2,
                character: 0,
            },
            "\n",
            &options,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "    ");
    }

    #[test]
    fn test_format_on_type_newline_aligns_inside_formatted_triple_string() {
        let text = "fn test() {\n    let test = f\"\"\"\n\n                \"\"\"\n}\n";
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        };

        let edits = format_on_type(
            text,
            Position {
                line: 2,
                character: 0,
            },
            "\n",
            &options,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "                ");
    }
}
