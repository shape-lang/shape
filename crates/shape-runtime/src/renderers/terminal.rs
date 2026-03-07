//! Terminal renderer — renders ContentNode to ANSI-escaped terminal output.
//!
//! Supports:
//! - Styled text with fg/bg colors, bold, italic, underline, dim
//! - Tables with unicode box-drawing characters (6 border styles)
//! - Code blocks with indentation and language label
//! - Charts as placeholder text
//! - Key-value pairs with aligned output
//! - Fragments via concatenation

use crate::content_renderer::{ContentRenderer, RenderContext, RendererCapabilities};
use shape_value::content::{
    BorderStyle, ChartSpec, Color, ContentNode, ContentTable, NamedColor, Style, StyledText,
};
use std::fmt::Write;

/// Renders ContentNode trees to ANSI terminal output.
///
/// Carries a [`RenderContext`] to control terminal width, max rows, etc.
pub struct TerminalRenderer {
    pub ctx: RenderContext,
}

impl TerminalRenderer {
    /// Create a renderer with default terminal context.
    pub fn new() -> Self {
        Self {
            ctx: RenderContext::terminal(),
        }
    }

    /// Create a renderer with a specific context.
    pub fn with_context(ctx: RenderContext) -> Self {
        Self { ctx }
    }
}

impl Default for TerminalRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentRenderer for TerminalRenderer {
    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities::terminal()
    }

    fn render(&self, content: &ContentNode) -> String {
        render_node(content, &self.ctx)
    }
}

fn render_node(node: &ContentNode, ctx: &RenderContext) -> String {
    match node {
        ContentNode::Text(st) => render_styled_text(st),
        ContentNode::Table(table) => render_table(table, ctx),
        ContentNode::Code { language, source } => render_code(language.as_deref(), source),
        ContentNode::Chart(spec) => render_chart(spec),
        ContentNode::KeyValue(pairs) => render_key_value(pairs, ctx),
        ContentNode::Fragment(parts) => parts.iter().map(|n| render_node(n, ctx)).collect(),
    }
}

fn render_styled_text(st: &StyledText) -> String {
    let mut out = String::new();
    for span in &st.spans {
        let codes = style_to_ansi_codes(&span.style);
        if codes.is_empty() {
            out.push_str(&span.text);
        } else {
            let _ = write!(out, "\x1b[{}m{}\x1b[0m", codes, span.text);
        }
    }
    out
}

fn style_to_ansi_codes(style: &Style) -> String {
    let mut codes = Vec::new();
    if style.bold {
        codes.push("1".to_string());
    }
    if style.dim {
        codes.push("2".to_string());
    }
    if style.italic {
        codes.push("3".to_string());
    }
    if style.underline {
        codes.push("4".to_string());
    }
    if let Some(ref color) = style.fg {
        codes.push(color_to_fg_code(color));
    }
    if let Some(ref color) = style.bg {
        codes.push(color_to_bg_code(color));
    }
    codes.join(";")
}

fn color_to_fg_code(color: &Color) -> String {
    match color {
        Color::Named(named) => named_color_fg(*named).to_string(),
        Color::Rgb(r, g, b) => format!("38;2;{};{};{}", r, g, b),
    }
}

fn color_to_bg_code(color: &Color) -> String {
    match color {
        Color::Named(named) => named_color_bg(*named).to_string(),
        Color::Rgb(r, g, b) => format!("48;2;{};{};{}", r, g, b),
    }
}

fn named_color_fg(color: NamedColor) -> u8 {
    match color {
        NamedColor::Red => 31,
        NamedColor::Green => 32,
        NamedColor::Yellow => 33,
        NamedColor::Blue => 34,
        NamedColor::Magenta => 35,
        NamedColor::Cyan => 36,
        NamedColor::White => 37,
        NamedColor::Default => 39,
    }
}

fn named_color_bg(color: NamedColor) -> u8 {
    match color {
        NamedColor::Red => 41,
        NamedColor::Green => 42,
        NamedColor::Yellow => 43,
        NamedColor::Blue => 44,
        NamedColor::Magenta => 45,
        NamedColor::Cyan => 46,
        NamedColor::White => 47,
        NamedColor::Default => 49,
    }
}

// ========== Table rendering ==========

/// Box-drawing character set for a given border style.
struct BoxChars {
    top_left: &'static str,
    top_mid: &'static str,
    top_right: &'static str,
    mid_left: &'static str,
    mid_mid: &'static str,
    mid_right: &'static str,
    bot_left: &'static str,
    bot_mid: &'static str,
    bot_right: &'static str,
    horizontal: &'static str,
    vertical: &'static str,
}

fn box_chars(style: BorderStyle) -> BoxChars {
    match style {
        BorderStyle::Rounded => BoxChars {
            top_left: "\u{256d}",   // ╭
            top_mid: "\u{252c}",    // ┬
            top_right: "\u{256e}",  // ╮
            mid_left: "\u{251c}",   // ├
            mid_mid: "\u{253c}",    // ┼
            mid_right: "\u{2524}",  // ┤
            bot_left: "\u{2570}",   // ╰
            bot_mid: "\u{2534}",    // ┴
            bot_right: "\u{256f}",  // ╯
            horizontal: "\u{2500}", // ─
            vertical: "\u{2502}",   // │
        },
        BorderStyle::Sharp => BoxChars {
            top_left: "\u{250c}",   // ┌
            top_mid: "\u{252c}",    // ┬
            top_right: "\u{2510}",  // ┐
            mid_left: "\u{251c}",   // ├
            mid_mid: "\u{253c}",    // ┼
            mid_right: "\u{2524}",  // ┤
            bot_left: "\u{2514}",   // └
            bot_mid: "\u{2534}",    // ┴
            bot_right: "\u{2518}",  // ┘
            horizontal: "\u{2500}", // ─
            vertical: "\u{2502}",   // │
        },
        BorderStyle::Heavy => BoxChars {
            top_left: "\u{250f}",   // ┏
            top_mid: "\u{2533}",    // ┳
            top_right: "\u{2513}",  // ┓
            mid_left: "\u{2523}",   // ┣
            mid_mid: "\u{254b}",    // ╋
            mid_right: "\u{252b}",  // ┫
            bot_left: "\u{2517}",   // ┗
            bot_mid: "\u{253b}",    // ┻
            bot_right: "\u{251b}",  // ┛
            horizontal: "\u{2501}", // ━
            vertical: "\u{2503}",   // ┃
        },
        BorderStyle::Double => BoxChars {
            top_left: "\u{2554}",   // ╔
            top_mid: "\u{2566}",    // ╦
            top_right: "\u{2557}",  // ╗
            mid_left: "\u{2560}",   // ╠
            mid_mid: "\u{256c}",    // ╬
            mid_right: "\u{2563}",  // ╣
            bot_left: "\u{255a}",   // ╚
            bot_mid: "\u{2569}",    // ╩
            bot_right: "\u{255d}",  // ╝
            horizontal: "\u{2550}", // ═
            vertical: "\u{2551}",   // ║
        },
        BorderStyle::Minimal => BoxChars {
            top_left: " ",
            top_mid: " ",
            top_right: " ",
            mid_left: " ",
            mid_mid: " ",
            mid_right: " ",
            bot_left: " ",
            bot_mid: " ",
            bot_right: " ",
            horizontal: "-",
            vertical: " ",
        },
        BorderStyle::None => BoxChars {
            top_left: "",
            top_mid: "",
            top_right: "",
            mid_left: "",
            mid_mid: "",
            mid_right: "",
            bot_left: "",
            bot_mid: "",
            bot_right: "",
            horizontal: "",
            vertical: " ",
        },
    }
}

fn render_table(table: &ContentTable, ctx: &RenderContext) -> String {
    if table.border == BorderStyle::None {
        return render_table_no_border(table);
    }

    let bc = box_chars(table.border);

    // Compute column widths
    let col_count = table.headers.len();
    let mut widths: Vec<usize> = table.headers.iter().map(|h| h.len()).collect();

    let limit = table.max_rows.or(ctx.max_rows).unwrap_or(table.rows.len());
    let display_rows = &table.rows[..limit.min(table.rows.len())];
    let truncated = table.rows.len().saturating_sub(limit);

    for row in display_rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                let cell_text = cell.to_string();
                if cell_text.len() > widths[i] {
                    widths[i] = cell_text.len();
                }
            }
        }
    }

    // Constrain column widths to ctx.max_width (proportional shrink)
    if let Some(max_w) = ctx.max_width {
        let overhead = col_count + 1 + col_count * 2; // borders + padding
        if overhead < max_w {
            let available = max_w - overhead;
            let total_natural: usize = widths.iter().sum();
            if total_natural > available && total_natural > 0 {
                for w in &mut widths {
                    *w = (*w * available / total_natural).max(3);
                }
            }
        }
    }

    let mut out = String::new();

    // Top border
    let _ = write!(out, "{}", bc.top_left);
    for (i, w) in widths.iter().enumerate() {
        for _ in 0..(w + 2) {
            out.push_str(bc.horizontal);
        }
        if i < col_count - 1 {
            out.push_str(bc.top_mid);
        }
    }
    let _ = writeln!(out, "{}", bc.top_right);

    // Header row
    let _ = write!(out, "{}", bc.vertical);
    for (i, header) in table.headers.iter().enumerate() {
        let _ = write!(out, " {:width$} ", header, width = widths[i]);
        out.push_str(bc.vertical);
    }
    let _ = writeln!(out);

    // Separator
    let _ = write!(out, "{}", bc.mid_left);
    for (i, w) in widths.iter().enumerate() {
        for _ in 0..(w + 2) {
            out.push_str(bc.horizontal);
        }
        if i < col_count - 1 {
            out.push_str(bc.mid_mid);
        }
    }
    let _ = writeln!(out, "{}", bc.mid_right);

    // Data rows
    for row in display_rows {
        let _ = write!(out, "{}", bc.vertical);
        for i in 0..col_count {
            let cell_text = row.get(i).map(|c| c.to_string()).unwrap_or_default();
            let _ = write!(out, " {:width$} ", cell_text, width = widths[i]);
            out.push_str(bc.vertical);
        }
        let _ = writeln!(out);
    }

    // Truncation indicator
    if truncated > 0 {
        let _ = write!(out, "{}", bc.vertical);
        let msg = format!("... {} more rows", truncated);
        let total_width: usize = widths.iter().sum::<usize>() + (col_count - 1) * 3 + 2;
        let _ = write!(out, " {:width$} ", msg, width = total_width);
        out.push_str(bc.vertical);
        let _ = writeln!(out);
    }

    // Bottom border
    let _ = write!(out, "{}", bc.bot_left);
    for (i, w) in widths.iter().enumerate() {
        for _ in 0..(w + 2) {
            out.push_str(bc.horizontal);
        }
        if i < col_count - 1 {
            out.push_str(bc.bot_mid);
        }
    }
    let _ = writeln!(out, "{}", bc.bot_right);

    out
}

fn render_table_no_border(table: &ContentTable) -> String {
    let col_count = table.headers.len();
    let mut widths: Vec<usize> = table.headers.iter().map(|h| h.len()).collect();

    let limit = table.max_rows.unwrap_or(table.rows.len());
    let display_rows = &table.rows[..limit.min(table.rows.len())];
    let truncated = table.rows.len().saturating_sub(limit);

    for row in display_rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                let cell_text = cell.to_string();
                if cell_text.len() > widths[i] {
                    widths[i] = cell_text.len();
                }
            }
        }
    }

    let mut out = String::new();

    // Header row
    for (i, header) in table.headers.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        let _ = write!(out, "{:width$}", header, width = widths[i]);
    }
    let _ = writeln!(out);

    // Data rows
    for row in display_rows {
        for i in 0..col_count {
            if i > 0 {
                out.push_str("  ");
            }
            let cell_text = row.get(i).map(|c| c.to_string()).unwrap_or_default();
            let _ = write!(out, "{:width$}", cell_text, width = widths[i]);
        }
        let _ = writeln!(out);
    }

    if truncated > 0 {
        let _ = writeln!(out, "... {} more rows", truncated);
    }

    out
}

fn render_code(language: Option<&str>, source: &str) -> String {
    let mut out = String::new();
    if let Some(lang) = language {
        let _ = writeln!(out, "\x1b[2m[{}]\x1b[0m", lang);
    }
    for line in source.lines() {
        let _ = writeln!(out, "    {}", line);
    }
    out
}

fn render_chart(spec: &ChartSpec) -> String {
    // If the chart has actual data, render with braille/block characters
    let has_data = !spec.channels.is_empty()
        && spec.channels.iter().any(|c| !c.values.is_empty());
    if has_data {
        return super::terminal_chart::render_chart_text(spec);
    }

    // Fallback: placeholder text
    let title = spec.title.as_deref().unwrap_or("untitled");
    let type_name = chart_type_display_name(spec.chart_type);
    let y_count = spec.channels_by_name("y").len();
    format!(
        "[{} Chart: {} ({} series)]\n",
        type_name, title, y_count
    )
}

fn chart_type_display_name(ct: shape_value::content::ChartType) -> &'static str {
    use shape_value::content::ChartType;
    match ct {
        ChartType::Line => "Line",
        ChartType::Bar => "Bar",
        ChartType::Scatter => "Scatter",
        ChartType::Area => "Area",
        ChartType::Candlestick => "Candlestick",
        ChartType::Histogram => "Histogram",
        ChartType::BoxPlot => "BoxPlot",
        ChartType::Heatmap => "Heatmap",
        ChartType::Bubble => "Bubble",
    }
}

fn render_key_value(pairs: &[(String, ContentNode)], ctx: &RenderContext) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (key, value) in pairs {
        let value_str = render_node(value, ctx);
        let _ = writeln!(out, "{:width$}  {}", key, value_str, width = max_key_len);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::ContentTable;

    fn renderer() -> TerminalRenderer {
        TerminalRenderer::new()
    }

    #[test]
    fn test_plain_text_no_ansi() {
        let node = ContentNode::plain("hello world");
        let output = renderer().render(&node);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_bold_text_ansi() {
        let node = ContentNode::plain("bold").with_bold();
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[1m"));
        assert!(output.contains("bold"));
        assert!(output.contains("\x1b[0m"));
    }

    #[test]
    fn test_fg_color_ansi() {
        let node = ContentNode::plain("red").with_fg(Color::Named(NamedColor::Red));
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[31m"));
        assert!(output.contains("red"));
        assert!(output.contains("\x1b[0m"));
    }

    #[test]
    fn test_bg_color_ansi() {
        let node = ContentNode::plain("bg").with_bg(Color::Named(NamedColor::Blue));
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[44m"));
    }

    #[test]
    fn test_rgb_fg_color() {
        let node = ContentNode::plain("rgb").with_fg(Color::Rgb(255, 128, 0));
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[38;2;255;128;0m"));
    }

    #[test]
    fn test_rgb_bg_color() {
        let node = ContentNode::plain("rgb").with_bg(Color::Rgb(0, 255, 128));
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[48;2;0;255;128m"));
    }

    #[test]
    fn test_italic_ansi() {
        let node = ContentNode::plain("italic").with_italic();
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[3m"));
    }

    #[test]
    fn test_underline_ansi() {
        let node = ContentNode::plain("underline").with_underline();
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[4m"));
    }

    #[test]
    fn test_dim_ansi() {
        let node = ContentNode::plain("dim").with_dim();
        let output = renderer().render(&node);
        assert!(output.contains("\x1b[2m"));
    }

    #[test]
    fn test_combined_styles() {
        let node = ContentNode::plain("styled")
            .with_bold()
            .with_fg(Color::Named(NamedColor::Green));
        let output = renderer().render(&node);
        // Should contain both bold (1) and green fg (32)
        assert!(output.contains("1;32") || output.contains("32;1"));
        assert!(output.contains("styled"));
    }

    #[test]
    fn test_rounded_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["Name".into(), "Age".into()],
            rows: vec![
                vec![ContentNode::plain("Alice"), ContentNode::plain("30")],
                vec![ContentNode::plain("Bob"), ContentNode::plain("25")],
            ],
            border: BorderStyle::Rounded,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("\u{256d}")); // ╭
        assert!(output.contains("\u{256f}")); // ╯
        assert!(output.contains("Alice"));
        assert!(output.contains("Bob"));
    }

    #[test]
    fn test_heavy_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![vec![ContentNode::plain("1")]],
            border: BorderStyle::Heavy,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("\u{250f}")); // ┏
        assert!(output.contains("\u{251b}")); // ┛
    }

    #[test]
    fn test_double_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![vec![ContentNode::plain("1")]],
            border: BorderStyle::Double,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("\u{2554}")); // ╔
        assert!(output.contains("\u{255d}")); // ╝
    }

    #[test]
    fn test_table_max_rows_truncation() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![
                vec![ContentNode::plain("1")],
                vec![ContentNode::plain("2")],
                vec![ContentNode::plain("3")],
                vec![ContentNode::plain("4")],
            ],
            border: BorderStyle::Rounded,
            max_rows: Some(2),
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("1"));
        assert!(output.contains("2"));
        assert!(!output.contains(" 3 "));
        assert!(output.contains("... 2 more rows"));
    }

    #[test]
    fn test_no_border_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["A".into(), "B".into()],
            rows: vec![vec![ContentNode::plain("x"), ContentNode::plain("y")]],
            border: BorderStyle::None,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("A"));
        assert!(output.contains("B"));
        assert!(output.contains("x"));
        assert!(output.contains("y"));
        // Should not contain box-drawing characters
        assert!(!output.contains("\u{256d}"));
        assert!(!output.contains("\u{2500}"));
    }

    #[test]
    fn test_code_block_with_language() {
        let code = ContentNode::Code {
            language: Some("rust".into()),
            source: "fn main() {\n    println!(\"hi\");\n}".into(),
        };
        let output = renderer().render(&code);
        assert!(output.contains("[rust]"));
        assert!(output.contains("    fn main() {"));
    }

    #[test]
    fn test_code_block_no_language() {
        let code = ContentNode::Code {
            language: None,
            source: "hello".into(),
        };
        let output = renderer().render(&code);
        assert!(!output.contains("["));
        assert!(output.contains("    hello"));
    }

    #[test]
    fn test_chart_placeholder() {
        let chart = ContentNode::Chart(shape_value::content::ChartSpec {
            chart_type: shape_value::content::ChartType::Line,
            channels: vec![],
            x_categories: None,
            title: Some("Revenue".into()),
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let output = renderer().render(&chart);
        assert!(output.contains("Line Chart: Revenue (0 series)"));
    }

    #[test]
    fn test_key_value_aligned() {
        let kv = ContentNode::KeyValue(vec![
            ("name".into(), ContentNode::plain("Alice")),
            ("age".into(), ContentNode::plain("30")),
            ("location".into(), ContentNode::plain("NYC")),
        ]);
        let output = renderer().render(&kv);
        assert!(output.contains("name"));
        assert!(output.contains("Alice"));
        assert!(output.contains("location"));
        assert!(output.contains("NYC"));
    }

    #[test]
    fn test_fragment_concatenation() {
        let frag = ContentNode::Fragment(vec![
            ContentNode::plain("hello "),
            ContentNode::plain("world"),
        ]);
        let output = renderer().render(&frag);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_sharp_table_borders() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![vec![ContentNode::plain("1")]],
            border: BorderStyle::Sharp,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("\u{250c}")); // ┌
        assert!(output.contains("\u{2518}")); // ┘
    }
}
