//! Markdown renderer — renders ContentNode to GitHub-Flavored Markdown.
//!
//! Produces:
//! - Plain text for styled text (markdown has limited inline styling)
//! - GFM pipe tables
//! - Fenced code blocks with language tags
//! - Placeholder text for charts
//! - Key-value as bold key / plain value lines

use crate::content_renderer::{ContentRenderer, RendererCapabilities};
use shape_value::content::{ChartSpec, ContentNode, ContentTable};
use std::fmt::Write;

/// Renders ContentNode trees to GitHub-Flavored Markdown.
pub struct MarkdownRenderer;

impl ContentRenderer for MarkdownRenderer {
    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities::markdown()
    }

    fn render(&self, content: &ContentNode) -> String {
        render_node(content)
    }
}

fn render_node(node: &ContentNode) -> String {
    match node {
        ContentNode::Text(st) => {
            let mut out = String::new();
            for span in &st.spans {
                let mut text = span.text.clone();
                if span.style.bold {
                    text = format!("**{}**", text);
                }
                if span.style.italic {
                    text = format!("*{}*", text);
                }
                out.push_str(&text);
            }
            out
        }
        ContentNode::Table(table) => render_table(table),
        ContentNode::Code { language, source } => render_code(language.as_deref(), source),
        ContentNode::Chart(spec) => render_chart(spec),
        ContentNode::KeyValue(pairs) => render_key_value(pairs),
        ContentNode::Fragment(parts) => parts.iter().map(render_node).collect(),
    }
}

fn render_table(table: &ContentTable) -> String {
    let col_count = table.headers.len();
    if col_count == 0 {
        return String::new();
    }

    let mut widths: Vec<usize> = table.headers.iter().map(|h| h.len().max(3)).collect();

    let limit = table.max_rows.unwrap_or(table.rows.len());
    let display_rows = &table.rows[..limit.min(table.rows.len())];
    let truncated = table.rows.len().saturating_sub(limit);

    for row in display_rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                let cell_text = cell.to_string();
                widths[i] = widths[i].max(cell_text.len());
            }
        }
    }

    let mut out = String::new();

    // Header row
    out.push('|');
    for (i, header) in table.headers.iter().enumerate() {
        let _ = write!(out, " {:width$} |", header, width = widths[i]);
    }
    let _ = writeln!(out);

    // Separator
    out.push('|');
    for w in &widths {
        out.push(' ');
        for _ in 0..*w {
            out.push('-');
        }
        out.push_str(" |");
    }
    let _ = writeln!(out);

    // Data rows
    for row in display_rows {
        out.push('|');
        for i in 0..col_count {
            let cell_text = row.get(i).map(|c| c.to_string()).unwrap_or_default();
            let _ = write!(out, " {:width$} |", cell_text, width = widths[i]);
        }
        let _ = writeln!(out);
    }

    if truncated > 0 {
        let _ = writeln!(out, "\n*... {} more rows*", truncated);
    }

    out
}

fn render_code(language: Option<&str>, source: &str) -> String {
    let lang = language.unwrap_or("");
    format!("```{}\n{}\n```\n", lang, source)
}

fn render_chart(spec: &ChartSpec) -> String {
    let title = spec.title.as_deref().unwrap_or("untitled");
    let type_name = chart_type_display_name(spec.chart_type);
    let y_count = spec.channels_by_name("y").len();
    format!("*[{} Chart: {} ({} series)]*\n", type_name, title, y_count)
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

fn render_key_value(pairs: &[(String, ContentNode)]) -> String {
    let mut out = String::new();
    for (key, value) in pairs {
        let _ = writeln!(out, "**{}**: {}", key, render_node(value));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::{BorderStyle, Color, ContentTable, NamedColor};

    fn renderer() -> MarkdownRenderer {
        MarkdownRenderer
    }

    #[test]
    fn test_plain_text_md() {
        let node = ContentNode::plain("hello");
        assert_eq!(renderer().render(&node), "hello");
    }

    #[test]
    fn test_bold_text_md() {
        let node = ContentNode::plain("bold").with_bold();
        let output = renderer().render(&node);
        assert_eq!(output, "**bold**");
    }

    #[test]
    fn test_italic_text_md() {
        let node = ContentNode::plain("italic").with_italic();
        let output = renderer().render(&node);
        assert_eq!(output, "*italic*");
    }

    #[test]
    fn test_bold_italic_md() {
        let node = ContentNode::plain("both").with_bold().with_italic();
        let output = renderer().render(&node);
        assert_eq!(output, "***both***");
    }

    #[test]
    fn test_gfm_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["Name".into(), "Age".into()],
            rows: vec![vec![ContentNode::plain("Alice"), ContentNode::plain("30")]],
            border: BorderStyle::default(),
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("| Name"));
        assert!(output.contains("| ---"));
        assert!(output.contains("| Alice"));
    }

    #[test]
    fn test_gfm_table_truncation() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![
                vec![ContentNode::plain("1")],
                vec![ContentNode::plain("2")],
                vec![ContentNode::plain("3")],
            ],
            border: BorderStyle::default(),
            max_rows: Some(1),
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("*... 2 more rows*"));
    }

    #[test]
    fn test_fenced_code_block() {
        let code = ContentNode::Code {
            language: Some("rust".into()),
            source: "fn main() {}".into(),
        };
        let output = renderer().render(&code);
        assert!(output.starts_with("```rust\n"));
        assert!(output.contains("fn main() {}"));
        assert!(output.contains("```"));
    }

    #[test]
    fn test_code_block_no_language() {
        let code = ContentNode::Code {
            language: None,
            source: "hello".into(),
        };
        let output = renderer().render(&code);
        assert!(output.starts_with("```\n"));
    }

    #[test]
    fn test_kv_md() {
        let kv = ContentNode::KeyValue(vec![("name".into(), ContentNode::plain("Alice"))]);
        let output = renderer().render(&kv);
        assert!(output.contains("**name**: Alice"));
    }

    #[test]
    fn test_chart_placeholder_md() {
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
        assert!(output.contains("Line Chart: Revenue"));
    }

    #[test]
    fn test_fragment_md() {
        let frag = ContentNode::Fragment(vec![
            ContentNode::plain("hello "),
            ContentNode::plain("world"),
        ]);
        assert_eq!(renderer().render(&frag), "hello world");
    }

    #[test]
    fn test_no_ansi_in_md() {
        let node = ContentNode::plain("colored").with_fg(Color::Named(NamedColor::Red));
        let output = renderer().render(&node);
        assert!(!output.contains("\x1b["));
    }
}
