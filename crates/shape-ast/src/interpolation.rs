//! Shared formatted-string interpolation parsing.
//!
//! This module is intentionally syntax-only. It extracts literal and
//! expression segments from `f"..."` strings so all consumers (compiler,
//! type checker, LSP) can run their normal expression pipelines on the
//! extracted `{...}` expressions.

use crate::ast::InterpolationMode;
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
    /// Content-string styling specification.
    ContentStyle(ContentFormatSpec),
}

/// Chart type hint for content format spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartTypeSpec {
    Line,
    Bar,
    Scatter,
    Area,
    Histogram,
}

/// Content-string format specification for rich terminal/HTML output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentFormatSpec {
    pub fg: Option<ColorSpec>,
    pub bg: Option<ColorSpec>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub fixed_precision: Option<u8>,
    pub border: Option<BorderStyleSpec>,
    pub max_rows: Option<usize>,
    pub align: Option<AlignSpec>,
    /// Chart type hint: render the value as a chart instead of text.
    pub chart_type: Option<ChartTypeSpec>,
    /// Column name to use as x-axis data.
    pub x_column: Option<String>,
    /// Column names to use as y-axis series.
    pub y_columns: Vec<String>,
}

impl Default for ContentFormatSpec {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            fixed_precision: None,
            border: None,
            max_rows: None,
            align: None,
            chart_type: None,
            x_column: None,
            y_columns: vec![],
        }
    }
}

/// Color specification for content strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorSpec {
    Named(NamedContentColor),
    Rgb(u8, u8, u8),
}

/// Named colors for content strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedContentColor {
    Red,
    Green,
    Blue,
    Yellow,
    Magenta,
    Cyan,
    White,
    Default,
}

/// Border style for content-string table rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyleSpec {
    Rounded,
    Sharp,
    Heavy,
    Double,
    Minimal,
    None,
}

/// Alignment for content-string rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignSpec {
    Left,
    Center,
    Right,
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
        if ch == '\\' && matches!(chars.peek(), Some(&'{') | Some(&'}') | Some(&'$') | Some(&'#')) {
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

/// Parse a content string payload into interpolation parts.
///
/// Unlike `parse_interpolation_with_mode`, this uses `split_expression_and_content_format_spec`
/// to parse content-specific format specs (e.g., `fg(red), bold`) instead of the regular
/// fixed/table format specs.
pub fn parse_content_interpolation_with_mode(
    s: &str,
    mode: InterpolationMode,
) -> Result<Vec<InterpolationPart>> {
    let mut parts = Vec::new();
    let mut current_text = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        // Backslash-escaped delimiters: `\{` → `{`, `\}` → `}`, `\$` → `$`, `\#` → `#`
        if ch == '\\' && matches!(chars.peek(), Some(&'{') | Some(&'}') | Some(&'$') | Some(&'#')) {
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
                    let (expr, format_spec) = split_expression_and_content_format_spec(&raw_expr)?;
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
                        let (expr, format_spec) =
                            split_expression_and_content_format_spec(&raw_expr)?;
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

    Err(ShapeError::RuntimeError {
        message: format!(
            "Unsupported interpolation format spec '{}'. Supported: fixed(N), table(...).",
            spec
        ),
        location: None,
    })
}

/// Parse a content-string format spec like `"fg(red), bold, fixed(2)"`.
pub fn parse_content_format_spec(raw_spec: &str) -> Result<ContentFormatSpec> {
    let mut spec = ContentFormatSpec::default();
    let trimmed = raw_spec.trim();
    if trimmed.is_empty() {
        return Ok(spec);
    }

    for entry in split_top_level_commas(trimmed)? {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }

        // Boolean flags (no parens)
        match entry {
            "bold" => {
                spec.bold = true;
                continue;
            }
            "italic" => {
                spec.italic = true;
                continue;
            }
            "underline" => {
                spec.underline = true;
                continue;
            }
            "dim" => {
                spec.dim = true;
                continue;
            }
            _ => {}
        }

        // Call-like specs: fg(...), bg(...), fixed(...), border(...), max_rows(...), align(...)
        if let Some(idx) = entry.find('(') {
            if !entry.ends_with(')') {
                return Err(ShapeError::RuntimeError {
                    message: format!("Unclosed parenthesis in content format spec '{}'", entry),
                    location: None,
                });
            }
            let key = entry[..idx].trim();
            let inner = entry[idx + 1..entry.len() - 1].trim();
            match key {
                "fg" => {
                    spec.fg = Some(parse_color_spec(inner)?);
                }
                "bg" => {
                    spec.bg = Some(parse_color_spec(inner)?);
                }
                "fixed" => {
                    spec.fixed_precision = Some(parse_u8_value(inner, "fixed precision")?);
                }
                "border" => {
                    spec.border = Some(parse_border_style_spec(inner)?);
                }
                "max_rows" => {
                    spec.max_rows = Some(parse_usize_value(inner, "max_rows")?);
                }
                "align" => {
                    spec.align = Some(parse_align_spec(inner)?);
                }
                "chart" => {
                    spec.chart_type = Some(parse_chart_type_spec(inner)?);
                }
                "x" => {
                    spec.x_column = Some(inner.to_string());
                }
                "y" => {
                    // y(col) or y(col1, col2, ...)
                    spec.y_columns = inner
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                other => {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "Unknown content format key '{}'. Supported: fg, bg, bold, italic, underline, dim, fixed, border, max_rows, align, chart, x, y.",
                            other
                        ),
                        location: None,
                    });
                }
            }
            continue;
        }

        return Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown content format entry '{}'. Expected a flag (bold, italic, ...) or key(value).",
                entry
            ),
            location: None,
        });
    }

    Ok(spec)
}

fn parse_color_spec(s: &str) -> Result<ColorSpec> {
    let s = s.trim();
    // Try RGB: rgb(r, g, b)
    if s.starts_with("rgb(") && s.ends_with(')') {
        let inner = &s[4..s.len() - 1];
        let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
        if parts.len() != 3 {
            return Err(ShapeError::RuntimeError {
                message: format!("rgb() expects 3 values, got {}", parts.len()),
                location: None,
            });
        }
        let r = parse_u8_value(parts[0], "red")?;
        let g = parse_u8_value(parts[1], "green")?;
        let b = parse_u8_value(parts[2], "blue")?;
        return Ok(ColorSpec::Rgb(r, g, b));
    }
    // Named color
    match s {
        "red" => Ok(ColorSpec::Named(NamedContentColor::Red)),
        "green" => Ok(ColorSpec::Named(NamedContentColor::Green)),
        "blue" => Ok(ColorSpec::Named(NamedContentColor::Blue)),
        "yellow" => Ok(ColorSpec::Named(NamedContentColor::Yellow)),
        "magenta" => Ok(ColorSpec::Named(NamedContentColor::Magenta)),
        "cyan" => Ok(ColorSpec::Named(NamedContentColor::Cyan)),
        "white" => Ok(ColorSpec::Named(NamedContentColor::White)),
        "default" => Ok(ColorSpec::Named(NamedContentColor::Default)),
        _ => Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown color '{}'. Expected: red, green, blue, yellow, magenta, cyan, white, default, or rgb(r,g,b).",
                s
            ),
            location: None,
        }),
    }
}

fn parse_border_style_spec(s: &str) -> Result<BorderStyleSpec> {
    match s.trim() {
        "rounded" => Ok(BorderStyleSpec::Rounded),
        "sharp" => Ok(BorderStyleSpec::Sharp),
        "heavy" => Ok(BorderStyleSpec::Heavy),
        "double" => Ok(BorderStyleSpec::Double),
        "minimal" => Ok(BorderStyleSpec::Minimal),
        "none" => Ok(BorderStyleSpec::None),
        _ => Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown border style '{}'. Expected: rounded, sharp, heavy, double, minimal, none.",
                s
            ),
            location: None,
        }),
    }
}

fn parse_chart_type_spec(s: &str) -> Result<ChartTypeSpec> {
    match s.trim().to_lowercase().as_str() {
        "line" => Ok(ChartTypeSpec::Line),
        "bar" => Ok(ChartTypeSpec::Bar),
        "scatter" => Ok(ChartTypeSpec::Scatter),
        "area" => Ok(ChartTypeSpec::Area),
        "histogram" => Ok(ChartTypeSpec::Histogram),
        _ => Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown chart type '{}'. Expected: line, bar, scatter, area, histogram.",
                s
            ),
            location: None,
        }),
    }
}

fn parse_align_spec(s: &str) -> Result<AlignSpec> {
    match s.trim() {
        "left" => Ok(AlignSpec::Left),
        "center" => Ok(AlignSpec::Center),
        "right" => Ok(AlignSpec::Right),
        _ => Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown align value '{}'. Expected: left, center, right.",
                s
            ),
            location: None,
        }),
    }
}

/// Split interpolation content for content strings.
///
/// Content-string format specs use `parse_content_format_spec` instead of
/// the regular `parse_format_spec`.
pub fn split_expression_and_content_format_spec(
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
        Ok((
            expr.to_string(),
            Some(InterpolationFormatSpec::ContentStyle(
                parse_content_format_spec(spec)?,
            )),
        ))
    } else {
        Ok((trimmed.to_string(), None))
    }
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

    // ====== Content format spec tests ======

    #[test]
    fn parse_content_format_spec_bold() {
        let spec = parse_content_format_spec("bold").unwrap();
        assert!(spec.bold);
        assert!(!spec.italic);
    }

    #[test]
    fn parse_content_format_spec_multiple_flags() {
        let spec = parse_content_format_spec("bold, italic, underline").unwrap();
        assert!(spec.bold);
        assert!(spec.italic);
        assert!(spec.underline);
        assert!(!spec.dim);
    }

    #[test]
    fn parse_content_format_spec_fg_named() {
        let spec = parse_content_format_spec("fg(red)").unwrap();
        assert_eq!(spec.fg, Some(ColorSpec::Named(NamedContentColor::Red)));
    }

    #[test]
    fn parse_content_format_spec_fg_rgb() {
        let spec = parse_content_format_spec("fg(rgb(255, 128, 0))").unwrap();
        assert_eq!(spec.fg, Some(ColorSpec::Rgb(255, 128, 0)));
    }

    #[test]
    fn parse_content_format_spec_full() {
        let spec = parse_content_format_spec(
            "fg(green), bg(blue), bold, fixed(2), border(rounded), align(center)",
        )
        .unwrap();
        assert_eq!(spec.fg, Some(ColorSpec::Named(NamedContentColor::Green)));
        assert_eq!(spec.bg, Some(ColorSpec::Named(NamedContentColor::Blue)));
        assert!(spec.bold);
        assert_eq!(spec.fixed_precision, Some(2));
        assert_eq!(spec.border, Some(BorderStyleSpec::Rounded));
        assert_eq!(spec.align, Some(AlignSpec::Center));
    }

    #[test]
    fn parse_content_format_spec_unknown_key_errors() {
        let err = parse_content_format_spec("foo(bar)").unwrap_err();
        assert!(err.to_string().contains("Unknown content format key"));
    }

    #[test]
    fn split_content_format_spec_basic() {
        let (expr, spec) = split_expression_and_content_format_spec("price:fg(red), bold").unwrap();
        assert_eq!(expr, "price");
        assert!(matches!(
            spec,
            Some(InterpolationFormatSpec::ContentStyle(_))
        ));
        if let Some(InterpolationFormatSpec::ContentStyle(cs)) = spec {
            assert_eq!(cs.fg, Some(ColorSpec::Named(NamedContentColor::Red)));
            assert!(cs.bold);
        }
    }

    #[test]
    fn parse_content_format_spec_chart_type() {
        let spec = parse_content_format_spec("chart(bar)").unwrap();
        assert_eq!(spec.chart_type, Some(ChartTypeSpec::Bar));
    }

    #[test]
    fn parse_content_format_spec_chart_with_axes() {
        let spec = parse_content_format_spec("chart(line), x(month), y(revenue, profit)").unwrap();
        assert_eq!(spec.chart_type, Some(ChartTypeSpec::Line));
        assert_eq!(spec.x_column, Some("month".to_string()));
        assert_eq!(spec.y_columns, vec!["revenue", "profit"]);
    }

    #[test]
    fn parse_content_format_spec_chart_single_y() {
        let spec = parse_content_format_spec("chart(scatter), x(date), y(price)").unwrap();
        assert_eq!(spec.chart_type, Some(ChartTypeSpec::Scatter));
        assert_eq!(spec.x_column, Some("date".to_string()));
        assert_eq!(spec.y_columns, vec!["price"]);
    }

    #[test]
    fn parse_content_format_spec_chart_invalid_type() {
        let err = parse_content_format_spec("chart(pie)").unwrap_err();
        assert!(err.to_string().contains("Unknown chart type"));
    }

    #[test]
    fn split_content_chart_format_spec() {
        let (expr, spec) =
            split_expression_and_content_format_spec("data:chart(bar), x(month), y(sales)")
                .unwrap();
        assert_eq!(expr, "data");
        if let Some(InterpolationFormatSpec::ContentStyle(cs)) = spec {
            assert_eq!(cs.chart_type, Some(ChartTypeSpec::Bar));
            assert_eq!(cs.x_column, Some("month".to_string()));
            assert_eq!(cs.y_columns, vec!["sales"]);
        } else {
            panic!("expected ContentStyle");
        }
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

    #[test]
    fn content_interpolation_backslash_escaped_braces() {
        let parts =
            parse_content_interpolation_with_mode("\\{not interp\\}", InterpolationMode::Braces)
                .unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            InterpolationPart::Literal(s) if s == "{not interp}"
        ));
    }
}
