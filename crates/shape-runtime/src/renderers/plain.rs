//! Plain text renderer — renders ContentNode with no ANSI formatting.
//!
//! Produces clean text output suitable for logging, file output, or
//! environments that don't support ANSI escape codes. Tables use ASCII
//! box-drawing characters (+--+).

use crate::content_renderer::{ContentRenderer, RendererCapabilities};
use shape_value::content::{ChartSpec, ContentNode, ContentTable};
use std::fmt::Write;

/// Renders ContentNode trees to plain text with no ANSI codes.
pub struct PlainRenderer;

impl ContentRenderer for PlainRenderer {
    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities::plain()
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
                out.push_str(&span.text);
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

    // Top border: +------+------+
    write_ascii_border(&mut out, &widths);

    // Header row: | Name | Age  |
    let _ = write!(out, "|");
    for (i, header) in table.headers.iter().enumerate() {
        let _ = write!(out, " {:width$} |", header, width = widths[i]);
    }
    let _ = writeln!(out);

    // Separator
    write_ascii_border(&mut out, &widths);

    // Data rows
    for row in display_rows {
        let _ = write!(out, "|");
        for i in 0..col_count {
            let cell_text = row.get(i).map(|c| c.to_string()).unwrap_or_default();
            let _ = write!(out, " {:width$} |", cell_text, width = widths[i]);
        }
        let _ = writeln!(out);
    }

    // Truncation indicator
    if truncated > 0 {
        let _ = write!(out, "|");
        let msg = format!("... {} more rows", truncated);
        let total_width: usize = widths.iter().sum::<usize>() + (col_count - 1) * 3 + 2;
        let _ = write!(out, " {:width$} |", msg, width = total_width);
        let _ = writeln!(out);
    }

    // Bottom border
    write_ascii_border(&mut out, &widths);

    out
}

fn write_ascii_border(out: &mut String, widths: &[usize]) {
    let _ = write!(out, "+");
    for w in widths {
        for _ in 0..(w + 2) {
            out.push('-');
        }
        out.push('+');
    }
    let _ = writeln!(out);
}

fn render_code(language: Option<&str>, source: &str) -> String {
    let mut out = String::new();
    if let Some(lang) = language {
        let _ = writeln!(out, "[{}]", lang);
    }
    for line in source.lines() {
        let _ = writeln!(out, "    {}", line);
    }
    out
}

fn render_chart(spec: &ChartSpec) -> String {
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

fn render_key_value(pairs: &[(String, ContentNode)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (key, value) in pairs {
        let value_str = render_node(value);
        let _ = writeln!(out, "{:width$}  {}", key, value_str, width = max_key_len);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::{BorderStyle, Color, ContentTable, NamedColor};

    fn renderer() -> PlainRenderer {
        PlainRenderer
    }

    #[test]
    fn test_plain_text() {
        let node = ContentNode::plain("hello world");
        let output = renderer().render(&node);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_styled_text_strips_styles() {
        let node = ContentNode::plain("styled")
            .with_bold()
            .with_fg(Color::Named(NamedColor::Red));
        let output = renderer().render(&node);
        // Should NOT contain any ANSI codes
        assert!(!output.contains("\x1b["));
        assert_eq!(output, "styled");
    }

    #[test]
    fn test_ascii_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["Name".into(), "Age".into()],
            rows: vec![
                vec![ContentNode::plain("Alice"), ContentNode::plain("30")],
                vec![ContentNode::plain("Bob"), ContentNode::plain("25")],
            ],
            border: BorderStyle::Rounded, // Ignored — plain always uses ASCII
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("+-------+-----+"));
        assert!(output.contains("| Alice | 30  |"));
        assert!(output.contains("| Bob   | 25  |"));
    }

    #[test]
    fn test_ascii_table_max_rows() {
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
        assert!(output.contains("| 1 |"));
        assert!(output.contains("... 2 more rows"));
        assert!(!output.contains("| 3 |"));
    }

    #[test]
    fn test_code_block_with_language() {
        let code = ContentNode::Code {
            language: Some("python".into()),
            source: "print(\"hi\")".into(),
        };
        let output = renderer().render(&code);
        assert!(output.contains("[python]"));
        assert!(output.contains("    print(\"hi\")"));
        assert!(!output.contains("\x1b["));
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
            chart_type: shape_value::content::ChartType::Bar,
            channels: vec![],
            x_categories: None,
            title: Some("Sales".into()),
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let output = renderer().render(&chart);
        assert_eq!(output, "[Bar Chart: Sales (0 series)]\n");
    }

    #[test]
    fn test_key_value() {
        let kv = ContentNode::KeyValue(vec![
            ("name".into(), ContentNode::plain("Alice")),
            ("age".into(), ContentNode::plain("30")),
        ]);
        let output = renderer().render(&kv);
        assert!(output.contains("name"));
        assert!(output.contains("Alice"));
        assert!(output.contains("age"));
        assert!(output.contains("30"));
        assert!(!output.contains("\x1b["));
    }

    #[test]
    fn test_fragment() {
        let frag = ContentNode::Fragment(vec![
            ContentNode::plain("hello "),
            ContentNode::plain("world"),
        ]);
        let output = renderer().render(&frag);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_no_ansi_in_any_output() {
        // Comprehensive check: build a complex tree and ensure no ANSI escapes
        let complex = ContentNode::Fragment(vec![
            ContentNode::plain("text")
                .with_bold()
                .with_fg(Color::Named(NamedColor::Red)),
            ContentNode::Table(ContentTable {
                headers: vec!["H".into()],
                rows: vec![vec![ContentNode::plain("v")]],
                border: BorderStyle::default(),
                max_rows: None,
                column_types: None,
                total_rows: None,
                sortable: false,
            }),
            ContentNode::Code {
                language: Some("js".into()),
                source: "1+1".into(),
            },
            ContentNode::KeyValue(vec![("k".into(), ContentNode::plain("v"))]),
        ]);
        let output = renderer().render(&complex);
        assert!(
            !output.contains("\x1b["),
            "Plain renderer must not emit ANSI codes"
        );
    }
}
