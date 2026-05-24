//! Shared formatted-string interpolation parsing.
//!
//! This module is intentionally syntax-only. It extracts literal and
//! expression segments from `f"..."` strings so all consumers (compiler,
//! type checker, LSP) can run their normal expression pipelines on the
//! extracted `{...}` expressions.

use crate::ast::InterpolationMode;
use crate::content_style::{ContentFormatSpec, parse_content_format_spec};
use crate::{Result, ShapeError};

/// Horizontal alignment for formatted output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatAlignment {
    Left,
    Center,
    Right,
}

impl FormatAlignment {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// Color hint for formatted output.
///
/// Renderers may map these hints to ANSI, HTML, or plain output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatColor {
    Default,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl FormatColor {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "red" => Some(Self::Red),
            "green" => Some(Self::Green),
            "yellow" => Some(Self::Yellow),
            "blue" => Some(Self::Blue),
            "magenta" => Some(Self::Magenta),
            "cyan" => Some(Self::Cyan),
            "white" => Some(Self::White),
            _ => None,
        }
    }
}

/// Typed table rendering configuration for interpolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFormatSpec {
    pub max_rows: Option<usize>,
    pub align: Option<FormatAlignment>,
    pub precision: Option<u8>,
    pub color: Option<FormatColor>,
    pub border: bool,
}

impl Default for TableFormatSpec {
    fn default() -> Self {
        Self {
            max_rows: None,
            align: None,
            precision: None,
            color: None,
            border: true,
        }
    }
}

/// Typed format specification for interpolation expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpolationFormatSpec {
    /// Fixed-point numeric precision (`fixed(2)`).
    Fixed { precision: u8 },
    /// Tabular formatting for `DataTable`-like values (`table(...)`).
    Table(TableFormatSpec),
    /// Content-styling specification (R8 W4 W18.4 — supervisor 2026-05-24
    /// D1 + (a-modified) REVIVE-WITH-SHARED-MODULE). Parsed from f-string
    /// styling syntax like `{x:bold,red}` / `{x:fg(blue),italic}`.
    /// The presence of this variant in any interpolation part flips the
    /// f-string's return type from `string` to `content` per D1's
    /// syntax-determined rule.
    ContentStyle(ContentFormatSpec),
}

/// A parsed segment of an interpolated string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpolationPart {
    /// Literal text.
    Literal(String),
    /// Expression segment with optional format specifier.
    Expression {
        /// Raw Shape expression between `{` and `}`.
        expr: String,
        /// Optional typed format spec after top-level `:`.
        format_spec: Option<InterpolationFormatSpec>,
    },
}

/// Parse a formatted string payload into interpolation parts.
pub fn parse_interpolation(s: &str) -> Result<Vec<InterpolationPart>> {
    parse_interpolation_with_mode(s, InterpolationMode::Braces)
}

/// Parse a formatted string payload into interpolation parts using the given mode.
pub fn parse_interpolation_with_mode(
    s: &str,
    mode: InterpolationMode,
) -> Result<Vec<InterpolationPart>> {
    let mut parts = Vec::new();
    let mut current_text = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        // Backslash-escaped delimiters: `\{` → `{`, `\}` → `}`, `\$` → `$`, `\#` → `#`
        if ch == '\\'
            && matches!(
                chars.peek(),
                Some(&'{') | Some(&'}') | Some(&'$') | Some(&'#')
            )
        {
            current_text.push(chars.next().unwrap());
            continue;
        }

        match mode {
            InterpolationMode::Braces => match ch {
                '{' => {
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        current_text.push('{');
                        continue;
                    }

                    if !current_text.is_empty() {
                        parts.push(InterpolationPart::Literal(current_text.clone()));
                        current_text.clear();
                    }

                    let raw_expr = parse_expression_content(&mut chars)?;
                    let (expr, format_spec) = split_expression_and_format_spec(&raw_expr)?;
                    parts.push(InterpolationPart::Expression { expr, format_spec });
                }
                '}' => {
                    if chars.peek() == Some(&'}') {
                        chars.next();
                        current_text.push('}');
                    } else {
                        return Err(ShapeError::RuntimeError {
                            message:
                                "Unmatched '}' in interpolation string. Use '}}' for a literal '}'"
                                    .to_string(),
                            location: None,
                        });
                    }
                }
                _ => current_text.push(ch),
            },
            InterpolationMode::Dollar | InterpolationMode::Hash => {
                let sigil = mode.sigil().expect("sigil mode must provide sigil");
                if ch == sigil {
                    if chars.peek() == Some(&sigil) {
                        chars.next();
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            current_text.push(sigil);
                            current_text.push('{');
                        } else {
                            current_text.push(sigil);
                        }
                        continue;
                    }

                    if chars.peek() == Some(&'{') {
                        chars.next();
                        if !current_text.is_empty() {
                            parts.push(InterpolationPart::Literal(current_text.clone()));
                            current_text.clear();
                        }
                        let raw_expr = parse_expression_content(&mut chars)?;
                        let (expr, format_spec) = split_expression_and_format_spec(&raw_expr)?;
                        parts.push(InterpolationPart::Expression { expr, format_spec });
                        continue;
                    }
                }

                current_text.push(ch);
            }
        }
    }

    if !current_text.is_empty() {
        parts.push(InterpolationPart::Literal(current_text));
    }

    Ok(parts)
}

/// Check whether a string contains at least one interpolation segment.
pub fn has_interpolation(s: &str) -> bool {
    has_interpolation_with_mode(s, InterpolationMode::Braces)
}

/// Check whether a string contains at least one interpolation segment for the mode.
pub fn has_interpolation_with_mode(s: &str, mode: InterpolationMode) -> bool {
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        // Skip backslash-escaped braces
        if ch == '\\' && matches!(chars.peek(), Some(&'{') | Some(&'}')) {
            chars.next();
            continue;
        }
        match mode {
            InterpolationMode::Braces => {
                if ch == '{' {
                    if chars.peek() != Some(&'{') {
                        return true;
                    }
                    chars.next();
                }
            }
            InterpolationMode::Dollar | InterpolationMode::Hash => {
                let sigil = mode.sigil().expect("sigil mode must provide sigil");
                if ch == sigil && chars.peek() == Some(&'{') {
                    return true;
                }
            }
        }
    }
    false
}

/// Split interpolation content `expr[:spec]` at the top-level format separator.
///
/// This preserves `::` (enum/type separators) and ignores separators inside
/// nested delimiters/strings.
pub fn split_expression_and_format_spec(
    raw: &str,
) -> Result<(String, Option<InterpolationFormatSpec>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "Empty expression in interpolation".to_string(),
            location: None,
        });
    }

    let split_at = find_top_level_format_colon(trimmed);

    if let Some(idx) = split_at {
        let expr = trimmed[..idx].trim();
        let spec = trimmed[idx + 1..].trim();
        if expr.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "Missing expression before format spec in interpolation".to_string(),
                location: None,
            });
        }
        if spec.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "Missing format spec after ':' in interpolation".to_string(),
                location: None,
            });
        }
        Ok((expr.to_string(), Some(parse_format_spec(spec)?)))
    } else {
        Ok((trimmed.to_string(), None))
    }
}

/// Find the top-level format-separator `:` in an interpolation expression.
///
/// Returns the byte index of the separator if present.
pub fn find_top_level_format_colon(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in raw.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ':' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                let prev_is_colon = idx > 0 && bytes[idx - 1] == b':';
                let next_is_colon = idx + 1 < bytes.len() && bytes[idx + 1] == b':';
                if !prev_is_colon && !next_is_colon {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_format_spec(raw_spec: &str) -> Result<InterpolationFormatSpec> {
    let spec = raw_spec.trim();

    // Legacy shorthand kept as a parser alias, but normalized into typed format.
    if let Some(precision) = parse_legacy_fixed_precision(spec)? {
        return Ok(InterpolationFormatSpec::Fixed { precision });
    }

    if let Some(inner) = parse_call_like_spec(spec, "fixed")? {
        let precision = parse_u8_value(inner.trim(), "fixed precision")?;
        return Ok(InterpolationFormatSpec::Fixed { precision });
    }

    if let Some(inner) = parse_call_like_spec(spec, "table")? {
        return Ok(InterpolationFormatSpec::Table(parse_table_format_spec(
            inner,
        )?));
    }

    // R8 W4 W18.4 (supervisor 2026-05-24 D1): try content-styling syntax
    // (`bold` / `red` / `fg(blue)` / `bold,red` / etc.). The content-style
    // parser is permissive about composition (multiple comma-separated
    // entries) and surfaces a descriptive error if no entry matches. Any
    // successful parse here triggers the syntax-determined `string` →
    // `content` flip at the f-string return-type inference site.
    let content_spec = parse_content_format_spec(spec).map_err(|err| ShapeError::RuntimeError {
        message: format!(
            "Unsupported interpolation format spec '{}'. Supported: fixed(N), table(...), content-styling (bold, italic, underline, dim, fg(color), bg(color), border(style), align(side), chart(type)). Inner error: {}",
            spec, err
        ),
        location: None,
    })?;
    Ok(InterpolationFormatSpec::ContentStyle(content_spec))
}

fn parse_legacy_fixed_precision(spec: &str) -> Result<Option<u8>> {
    if let Some(rest) = spec.strip_prefix('.') {
        let digits = rest.strip_suffix('f').unwrap_or(rest);
        if digits.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "Legacy fixed format requires digits after '.'".to_string(),
                location: None,
            });
        }
        if digits.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Some(parse_u8_value(digits, "fixed precision")?));
        }
    }
    Ok(None)
}

fn parse_call_like_spec<'a>(spec: &'a str, name: &str) -> Result<Option<&'a str>> {
    if !spec.starts_with(name) {
        return Ok(None);
    }

    let rest = &spec[name.len()..];
    if !rest.starts_with('(') || !rest.ends_with(')') {
        return Err(ShapeError::RuntimeError {
            message: format!("Format spec '{}' must use call syntax: {}(...)", spec, name),
            location: None,
        });
    }

    Ok(Some(&rest[1..rest.len() - 1]))
}

fn parse_table_format_spec(inner: &str) -> Result<TableFormatSpec> {
    let mut spec = TableFormatSpec::default();
    let trimmed = inner.trim();

    if trimmed.is_empty() {
        return Ok(spec);
    }

    for entry in split_top_level_commas(trimmed)? {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }

        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Invalid table format argument '{}'. Expected key=value pairs.",
                    entry
                ),
                location: None,
            })?;
        let key = key.trim();
        let value = value.trim();

        match key {
            "max_rows" => {
                spec.max_rows = Some(parse_usize_value(value, "max_rows")?);
            }
            "align" => {
                spec.align = Some(FormatAlignment::parse(value).ok_or_else(|| {
                    ShapeError::RuntimeError {
                        message: format!(
                            "Invalid align value '{}'. Expected: left, center, right.",
                            value
                        ),
                        location: None,
                    }
                })?);
            }
            "precision" => {
                spec.precision = Some(parse_u8_value(value, "precision")?);
            }
            "color" => {
                spec.color = Some(FormatColor::parse(value).ok_or_else(|| {
                    ShapeError::RuntimeError {
                        message: format!(
                            "Invalid color value '{}'. Expected: default, red, green, yellow, blue, magenta, cyan, white.",
                            value
                        ),
                        location: None,
                    }
                })?);
            }
            "border" => {
                spec.border = parse_on_off(value)?;
            }
            other => {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "Unknown table format key '{}'. Supported: max_rows, align, precision, color, border.",
                        other
                    ),
                    location: None,
                });
            }
        }
    }

    Ok(spec)
}

fn split_top_level_commas(s: &str) -> Result<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in s.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                parts.push(&s[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }

    if in_string.is_some() || paren_depth != 0 || brace_depth != 0 || bracket_depth != 0 {
        return Err(ShapeError::RuntimeError {
            message: "Unclosed delimiter in table format spec".to_string(),
            location: None,
        });
    }

    parts.push(&s[start..]);
    Ok(parts)
}

fn parse_u8_value(value: &str, label: &str) -> Result<u8> {
    value.parse::<u8>().map_err(|_| ShapeError::RuntimeError {
        message: format!(
            "Invalid {} '{}'. Expected an integer in range 0..=255.",
            label, value
        ),
        location: None,
    })
}

fn parse_usize_value(value: &str, label: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .map_err(|_| ShapeError::RuntimeError {
            message: format!(
                "Invalid {} '{}'. Expected a non-negative integer.",
                label, value
            ),
            location: None,
        })
}

fn parse_on_off(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(ShapeError::RuntimeError {
            message: format!("Invalid border value '{}'. Expected on or off.", value),
            location: None,
        }),
    }
}

fn parse_expression_content(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<String> {
    let mut expr = String::new();
    let mut brace_depth = 1usize;

    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                brace_depth += 1;
                expr.push(ch);
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                if brace_depth == 0 {
                    return if expr.trim().is_empty() {
                        Err(ShapeError::RuntimeError {
                            message: "Empty expression in interpolation".to_string(),
                            location: None,
                        })
                    } else {
                        Ok(expr)
                    };
                }
                expr.push(ch);
            }
            '"' => {
                expr.push(ch);
                while let Some(c) = chars.next() {
                    expr.push(c);
                    if c == '"' {
                        break;
                    }
                    if c == '\\' {
                        if let Some(escaped) = chars.next() {
                            expr.push(escaped);
                        }
                    }
                }
            }
            '\'' => {
                expr.push(ch);
                while let Some(c) = chars.next() {
                    expr.push(c);
                    if c == '\'' {
                        break;
                    }
                    if c == '\\' {
                        if let Some(escaped) = chars.next() {
                            expr.push(escaped);
                        }
                    }
                }
            }
            _ => expr.push(ch),
        }
    }

    Err(ShapeError::RuntimeError {
        message: "Unclosed interpolation (missing })".to_string(),
        location: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::InterpolationMode;

    #[test]
    fn parse_basic_interpolation() {
        let parts = parse_interpolation("value: {x}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "value: "));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "x"
        ));
    }

    #[test]
    fn parse_format_spec() {
        let parts = parse_interpolation("px={price:fixed(2)}").unwrap();
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: Some(spec)
            } if expr == "price" && *spec == InterpolationFormatSpec::Fixed { precision: 2 }
        ));
    }

    #[test]
    fn parse_legacy_fixed_precision_alias() {
        let parts = parse_interpolation("px={price:.2f}").unwrap();
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: Some(spec)
            } if expr == "price" && *spec == InterpolationFormatSpec::Fixed { precision: 2 }
        ));
    }

    #[test]
    fn parse_table_format_spec() {
        let parts = parse_interpolation(
            "rows={dt:table(max_rows=5, align=right, precision=2, color=green, border=off)}",
        )
        .unwrap();

        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: Some(InterpolationFormatSpec::Table(TableFormatSpec {
                    max_rows: Some(5),
                    align: Some(FormatAlignment::Right),
                    precision: Some(2),
                    color: Some(FormatColor::Green),
                    border: false
                }))
            } if expr == "dt"
        ));
    }

    #[test]
    fn parse_table_format_unknown_key_errors() {
        let err = parse_interpolation("rows={dt:table(foo=1)}").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Unknown table format key"),
            "unexpected error: {}",
            msg
        );
    }

    #[test]
    fn parse_double_colon_is_not_format_spec() {
        let parts = parse_interpolation("{Type::Variant}").unwrap();
        assert!(matches!(
            &parts[0],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "Type::Variant"
        ));
    }

    #[test]
    fn escaped_braces_do_not_count_as_interpolation() {
        assert!(!has_interpolation("Use {{x}} for literal"));
        assert!(has_interpolation("Use {x} for value"));
    }

    #[test]
    fn parse_dollar_interpolation() {
        let parts = parse_interpolation_with_mode(
            "json={\"name\": ${user.name}}",
            InterpolationMode::Dollar,
        )
        .unwrap();
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "json={\"name\": "));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "user.name"
        ));
        assert!(matches!(&parts[2], InterpolationPart::Literal(s) if s == "}"));
    }

    #[test]
    fn parse_hash_interpolation() {
        let parts = parse_interpolation_with_mode("echo #{cmd}", InterpolationMode::Hash).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "echo "));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression {
                expr,
                format_spec: None
            } if expr == "cmd"
        ));
    }

    #[test]
    fn escaped_sigil_opener_is_literal_in_sigil_modes() {
        let parts =
            parse_interpolation_with_mode("literal $${x}", InterpolationMode::Dollar).unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Literal(s) if s == "literal ${x}"
        ));
    }

    #[test]
    fn braces_are_plain_text_in_sigil_mode() {
        assert!(!has_interpolation_with_mode(
            "{\"a\": 1}",
            InterpolationMode::Dollar
        ));
        assert!(has_interpolation_with_mode(
            "${x}",
            InterpolationMode::Dollar
        ));
    }

    // --- LOW-2: backslash-escaped braces in interpolation ---

    #[test]
    fn backslash_escaped_braces_produce_literal_text() {
        // `\{` and `\}` should produce literal `{` and `}`, not interpolation.
        let parts = parse_interpolation("hello \\{world\\}").unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Literal(s) if s == "hello {world}"
        ));
    }

    #[test]
    fn backslash_escaped_braces_not_counted_as_interpolation() {
        assert!(!has_interpolation("hello \\{world\\}"));
        assert!(has_interpolation("hello {world}"));
    }

    #[test]
    fn backslash_escaped_braces_mixed_with_real_interpolation() {
        // `\{literal\} and {expr}` → Literal("{literal} and "), Expression("expr")
        let parts = parse_interpolation("\\{literal\\} and {expr}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Literal(s) if s == "{literal} and "
        ));
        assert!(matches!(
            &parts[1],
            InterpolationPart::Expression { expr, .. } if expr == "expr"
        ));
    }

    // --- R8 W4 W18.4: content-styling f-string syntax ---

    #[test]
    fn parse_content_style_bold() {
        use crate::content_style::ContentFormatSpec;
        let parts = parse_interpolation("{x:bold}").unwrap();
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InterpolationPart::Expression {
                expr,
                format_spec: Some(InterpolationFormatSpec::ContentStyle(spec)),
            } => {
                assert_eq!(expr, "x");
                let expected = ContentFormatSpec {
                    bold: true,
                    ..Default::default()
                };
                assert_eq!(spec, &expected);
            }
            other => panic!("expected ContentStyle bold, got {:?}", other),
        }
    }

    #[test]
    fn parse_content_style_bold_red_canonical() {
        use crate::content_style::{ColorSpec, NamedContentColor};
        // Canonical W18.4 example from supervisor: `{x:bold,red}` parses to
        // ContentStyle with bold=true and fg=Named(Red).
        let parts = parse_interpolation("hello {name:bold,red}").unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InterpolationPart::Literal(s) if s == "hello "));
        match &parts[1] {
            InterpolationPart::Expression {
                expr,
                format_spec: Some(InterpolationFormatSpec::ContentStyle(spec)),
            } => {
                assert_eq!(expr, "name");
                assert!(spec.bold);
                assert_eq!(spec.fg, Some(ColorSpec::Named(NamedContentColor::Red)));
            }
            other => panic!("expected ContentStyle, got {:?}", other),
        }
    }

    #[test]
    fn parse_content_style_fg_call() {
        use crate::content_style::{ColorSpec, NamedContentColor};
        let parts = parse_interpolation("{x:fg(green)}").unwrap();
        match &parts[0] {
            InterpolationPart::Expression {
                format_spec: Some(InterpolationFormatSpec::ContentStyle(spec)),
                ..
            } => {
                assert_eq!(spec.fg, Some(ColorSpec::Named(NamedContentColor::Green)));
            }
            other => panic!("expected ContentStyle, got {:?}", other),
        }
    }

    #[test]
    fn fixed_and_table_still_parse_separately_from_content_style() {
        // Regression: fixed(N) must not be reinterpreted as content-style.
        let parts = parse_interpolation("{x:fixed(2)}").unwrap();
        match &parts[0] {
            InterpolationPart::Expression {
                format_spec: Some(InterpolationFormatSpec::Fixed { precision }),
                ..
            } => {
                assert_eq!(*precision, 2);
            }
            other => panic!("expected Fixed, got {:?}", other),
        }

        let parts = parse_interpolation("{x:table()}").unwrap();
        assert!(matches!(
            &parts[0],
            InterpolationPart::Expression {
                format_spec: Some(InterpolationFormatSpec::Table(_)),
                ..
            }
        ));
    }
}
