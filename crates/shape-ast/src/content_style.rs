//! Content styling spec types — shared module per supervisor 2026-05-24
//! disposition (a-modified) REVIVE-WITH-SHARED-MODULE.
//!
//! This module hosts the f-string / builder-API content styling spec types
//! revived from pre-W18.3 (`git show 19de5ef2^:crates/shape-ast/src/
//! interpolation.rs`). Both W18.4 (f-string styling parser/lowering) and
//! W18.5 (`Content.table` / `Content.code` / `Content.kv` builder API)
//! consume these types from this single shared module — no parallel
//! implementations per CLAUDE.md §Parallel-implementation.
//!
//! Module placement rationale: the supervisor's preferred location was
//! `crates/shape-value/src/content_style.rs` (next to `ContentNode`). The
//! dependency graph forbids that placement: `shape-ast` cannot import from
//! `shape-value` (shape-value depends on shape-ast, not the reverse), and
//! the parsers MUST live in shape-ast because the f-string interpolation
//! parser produces typed `InterpolationFormatSpec::ContentStyle(spec)`
//! parts during AST parsing. Placing the types in shape-ast satisfies the
//! "one source of truth" constraint while preserving the workspace
//! dependency invariant. shape-vm's compiler lowering and
//! shape-runtime's W18.5 builders both import from `shape_ast::
//! content_style::*`.
//!
//! The spec types here are SYNTACTIC descriptors — they describe what the
//! user wrote in `f"{x:bold,red}"` or `Content.table(...).border(rounded)`.
//! They are NOT runtime `ContentNode`/`Style`/`Color` types from
//! `shape_value::content`. Conversion happens at the lowering boundary.

use crate::{Result, ShapeError};

// =============================================================================
// SPEC TYPES (revived from 19de5ef2^ — W18.3 deletion source)
// =============================================================================

/// Content-string format specification for rich terminal/HTML output.
///
/// Parsed from `:bold,red` / `:fg(green),bg(blue)` / `:border:rounded` style
/// syntax in f-string interpolations, and constructed by builder methods
/// (`Content.table(...).border(rounded)`).
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Chart type hint for content format spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartTypeSpec {
    Line,
    Bar,
    Scatter,
    Area,
    Histogram,
}

// =============================================================================
// PARSERS (revived from 19de5ef2^)
// =============================================================================

/// Parse a content-string format spec like `"fg(red), bold, fixed(2)"`.
///
/// Used by both the f-string interpolation parser (when a `:spec` part is
/// recognised as content-styling syntax) and any builder-API path that
/// accepts a stringly-typed style descriptor.
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

        // Named-color shorthand: `red` / `green` / ... at top level → fg(named)
        if let Ok(color) = parse_color_spec(entry) {
            spec.fg = Some(color);
            continue;
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

pub fn parse_color_spec(s: &str) -> Result<ColorSpec> {
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

pub fn parse_border_style_spec(s: &str) -> Result<BorderStyleSpec> {
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

pub fn parse_align_spec(s: &str) -> Result<AlignSpec> {
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

pub fn parse_chart_type_spec(s: &str) -> Result<ChartTypeSpec> {
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

// =============================================================================
// SHARED PARSE HELPERS (used by parse_content_format_spec)
// =============================================================================

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
            message: "Unclosed delimiter in content format spec".to_string(),
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

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_content_format_spec_chart_invalid_type() {
        let err = parse_content_format_spec("chart(pie)").unwrap_err();
        assert!(err.to_string().contains("Unknown chart type"));
    }

    #[test]
    fn parse_color_spec_named() {
        assert_eq!(
            parse_color_spec("red").unwrap(),
            ColorSpec::Named(NamedContentColor::Red)
        );
        assert_eq!(
            parse_color_spec("default").unwrap(),
            ColorSpec::Named(NamedContentColor::Default)
        );
    }

    #[test]
    fn parse_color_spec_rgb() {
        assert_eq!(
            parse_color_spec("rgb(10, 20, 30)").unwrap(),
            ColorSpec::Rgb(10, 20, 30)
        );
    }

    #[test]
    fn parse_color_spec_invalid() {
        assert!(parse_color_spec("octarine").is_err());
        assert!(parse_color_spec("rgb(1, 2)").is_err());
        assert!(parse_color_spec("rgb(300, 0, 0)").is_err());
    }

    #[test]
    fn parse_border_style_all() {
        assert_eq!(
            parse_border_style_spec("rounded").unwrap(),
            BorderStyleSpec::Rounded
        );
        assert_eq!(
            parse_border_style_spec("sharp").unwrap(),
            BorderStyleSpec::Sharp
        );
        assert_eq!(
            parse_border_style_spec("heavy").unwrap(),
            BorderStyleSpec::Heavy
        );
        assert_eq!(
            parse_border_style_spec("double").unwrap(),
            BorderStyleSpec::Double
        );
        assert_eq!(
            parse_border_style_spec("minimal").unwrap(),
            BorderStyleSpec::Minimal
        );
        assert_eq!(
            parse_border_style_spec("none").unwrap(),
            BorderStyleSpec::None
        );
        assert!(parse_border_style_spec("triple").is_err());
    }

    #[test]
    fn parse_align_all() {
        assert_eq!(parse_align_spec("left").unwrap(), AlignSpec::Left);
        assert_eq!(parse_align_spec("center").unwrap(), AlignSpec::Center);
        assert_eq!(parse_align_spec("right").unwrap(), AlignSpec::Right);
        assert!(parse_align_spec("justify").is_err());
    }

    #[test]
    fn parse_chart_type_all() {
        assert_eq!(parse_chart_type_spec("line").unwrap(), ChartTypeSpec::Line);
        assert_eq!(parse_chart_type_spec("bar").unwrap(), ChartTypeSpec::Bar);
        assert_eq!(
            parse_chart_type_spec("scatter").unwrap(),
            ChartTypeSpec::Scatter
        );
        assert_eq!(parse_chart_type_spec("area").unwrap(), ChartTypeSpec::Area);
        assert_eq!(
            parse_chart_type_spec("histogram").unwrap(),
            ChartTypeSpec::Histogram
        );
        assert!(parse_chart_type_spec("pie").is_err());
    }

    #[test]
    fn parse_content_format_spec_top_level_color_shorthand() {
        // `{x:red}` → ColorSpec::Named(Red) as fg
        let spec = parse_content_format_spec("red").unwrap();
        assert_eq!(spec.fg, Some(ColorSpec::Named(NamedContentColor::Red)));
    }

    #[test]
    fn parse_content_format_spec_bold_red_shorthand() {
        // `{x:bold,red}` → bold + fg(red) — the canonical W18.4 example
        let spec = parse_content_format_spec("bold, red").unwrap();
        assert!(spec.bold);
        assert_eq!(spec.fg, Some(ColorSpec::Named(NamedContentColor::Red)));
    }
}
