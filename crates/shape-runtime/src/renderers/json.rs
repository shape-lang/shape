//! JSON renderer — renders ContentNode to a structured JSON tree.
//!
//! Produces a JSON representation that preserves the full structure
//! of the ContentNode tree, including styles, colors, and metadata.

use crate::content_renderer::{ContentRenderer, RendererCapabilities};
use shape_value::content::{
    BorderStyle, ChartSpec, Color, ContentNode, ContentTable, NamedColor, Style,
};
use std::fmt::Write;

/// Renders ContentNode trees to structured JSON.
pub struct JsonRenderer;

impl ContentRenderer for JsonRenderer {
    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities {
            ansi: false,
            unicode: true,
            color: true,
            interactive: false,
        }
    }

    fn render(&self, content: &ContentNode) -> String {
        render_node(content)
    }
}

fn render_node(node: &ContentNode) -> String {
    match node {
        ContentNode::Text(st) => {
            let spans: Vec<String> = st
                .spans
                .iter()
                .map(|span| {
                    let style = render_style(&span.style);
                    format!(
                        "{{\"text\":{},\"style\":{}}}",
                        json_string(&span.text),
                        style
                    )
                })
                .collect();
            format!("{{\"type\":\"text\",\"spans\":[{}]}}", spans.join(","))
        }
        ContentNode::Table(table) => render_table(table),
        ContentNode::Code { language, source } => {
            let lang = language
                .as_deref()
                .map(|l| json_string(l))
                .unwrap_or_else(|| "null".to_string());
            format!(
                "{{\"type\":\"code\",\"language\":{},\"source\":{}}}",
                lang,
                json_string(source)
            )
        }
        ContentNode::Chart(spec) => render_chart(spec),
        ContentNode::KeyValue(pairs) => {
            let entries: Vec<String> = pairs
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{{\"key\":{},\"value\":{}}}",
                        json_string(k),
                        render_node(v)
                    )
                })
                .collect();
            format!("{{\"type\":\"kv\",\"pairs\":[{}]}}", entries.join(","))
        }
        ContentNode::Fragment(parts) => {
            let children: Vec<String> = parts.iter().map(render_node).collect();
            format!(
                "{{\"type\":\"fragment\",\"children\":[{}]}}",
                children.join(",")
            )
        }
    }
}

fn render_style(style: &Style) -> String {
    let mut parts = Vec::new();
    if style.bold {
        parts.push("\"bold\":true".to_string());
    }
    if style.italic {
        parts.push("\"italic\":true".to_string());
    }
    if style.underline {
        parts.push("\"underline\":true".to_string());
    }
    if style.dim {
        parts.push("\"dim\":true".to_string());
    }
    if let Some(ref color) = style.fg {
        parts.push(format!("\"fg\":{}", render_color(color)));
    }
    if let Some(ref color) = style.bg {
        parts.push(format!("\"bg\":{}", render_color(color)));
    }
    if parts.is_empty() {
        "{}".to_string()
    } else {
        format!("{{{}}}", parts.join(","))
    }
}

fn render_color(color: &Color) -> String {
    match color {
        Color::Named(named) => json_string(named_to_str(*named)),
        Color::Rgb(r, g, b) => format!("{{\"r\":{},\"g\":{},\"b\":{}}}", r, g, b),
    }
}

fn named_to_str(color: NamedColor) -> &'static str {
    match color {
        NamedColor::Red => "red",
        NamedColor::Green => "green",
        NamedColor::Blue => "blue",
        NamedColor::Yellow => "yellow",
        NamedColor::Magenta => "magenta",
        NamedColor::Cyan => "cyan",
        NamedColor::White => "white",
        NamedColor::Default => "default",
    }
}

fn render_table(table: &ContentTable) -> String {
    let headers: Vec<String> = table.headers.iter().map(|h| json_string(h)).collect();

    let limit = table.max_rows.unwrap_or(table.rows.len());
    let display_rows = &table.rows[..limit.min(table.rows.len())];

    let rows: Vec<String> = display_rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = row.iter().map(render_node).collect();
            format!("[{}]", cells.join(","))
        })
        .collect();

    let border = match table.border {
        BorderStyle::Rounded => "\"rounded\"",
        BorderStyle::Sharp => "\"sharp\"",
        BorderStyle::Heavy => "\"heavy\"",
        BorderStyle::Double => "\"double\"",
        BorderStyle::Minimal => "\"minimal\"",
        BorderStyle::None => "\"none\"",
    };

    let max_rows = table
        .max_rows
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".to_string());

    format!(
        "{{\"type\":\"table\",\"headers\":[{}],\"rows\":[{}],\"border\":{},\"max_rows\":{},\"total_rows\":{}}}",
        headers.join(","),
        rows.join(","),
        border,
        max_rows,
        table.rows.len()
    )
}

fn render_chart(spec: &ChartSpec) -> String {
    let chart_type = match spec.chart_type {
        shape_value::content::ChartType::Line => "\"line\"",
        shape_value::content::ChartType::Bar => "\"bar\"",
        shape_value::content::ChartType::Scatter => "\"scatter\"",
        shape_value::content::ChartType::Area => "\"area\"",
        shape_value::content::ChartType::Candlestick => "\"candlestick\"",
        shape_value::content::ChartType::Histogram => "\"histogram\"",
    };

    let title = spec
        .title
        .as_deref()
        .map(|t| json_string(t))
        .unwrap_or_else(|| "null".to_string());

    let mut parts = vec![
        format!("\"type\":\"chart\""),
        format!("\"chart_type\":{}", chart_type),
        format!("\"title\":{}", title),
        format!("\"series_count\":{}", spec.series.len()),
    ];

    if let Some(ref xl) = spec.x_label {
        parts.push(format!("\"x_label\":{}", json_string(xl)));
    }
    if let Some(ref yl) = spec.y_label {
        parts.push(format!("\"y_label\":{}", json_string(yl)));
    }

    format!("{{{}}}", parts.join(","))
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::{ContentTable, NamedColor};

    fn renderer() -> JsonRenderer {
        JsonRenderer
    }

    #[test]
    fn test_plain_text_json() {
        let node = ContentNode::plain("hello");
        let output = renderer().render(&node);
        assert!(output.contains("\"type\":\"text\""));
        assert!(output.contains("\"text\":\"hello\""));
    }

    #[test]
    fn test_styled_text_json() {
        let node = ContentNode::plain("bold")
            .with_bold()
            .with_fg(Color::Named(NamedColor::Red));
        let output = renderer().render(&node);
        assert!(output.contains("\"bold\":true"));
        assert!(output.contains("\"fg\":\"red\""));
    }

    #[test]
    fn test_rgb_color_json() {
        let node = ContentNode::plain("rgb").with_fg(Color::Rgb(255, 0, 128));
        let output = renderer().render(&node);
        assert!(output.contains("\"r\":255"));
        assert!(output.contains("\"g\":0"));
        assert!(output.contains("\"b\":128"));
    }

    #[test]
    fn test_table_json() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["A".into()],
            rows: vec![vec![ContentNode::plain("1")]],
            border: BorderStyle::Rounded,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = renderer().render(&table);
        assert!(output.contains("\"type\":\"table\""));
        assert!(output.contains("\"headers\":[\"A\"]"));
        assert!(output.contains("\"border\":\"rounded\""));
        assert!(output.contains("\"total_rows\":1"));
    }

    #[test]
    fn test_code_json() {
        let code = ContentNode::Code {
            language: Some("rust".into()),
            source: "fn main() {}".into(),
        };
        let output = renderer().render(&code);
        assert!(output.contains("\"type\":\"code\""));
        assert!(output.contains("\"language\":\"rust\""));
        assert!(output.contains("\"source\":\"fn main() {}\""));
    }

    #[test]
    fn test_code_no_language_json() {
        let code = ContentNode::Code {
            language: None,
            source: "x".into(),
        };
        let output = renderer().render(&code);
        assert!(output.contains("\"language\":null"));
    }

    #[test]
    fn test_kv_json() {
        let kv = ContentNode::KeyValue(vec![("k".into(), ContentNode::plain("v"))]);
        let output = renderer().render(&kv);
        assert!(output.contains("\"type\":\"kv\""));
        assert!(output.contains("\"key\":\"k\""));
    }

    #[test]
    fn test_fragment_json() {
        let frag = ContentNode::Fragment(vec![ContentNode::plain("a"), ContentNode::plain("b")]);
        let output = renderer().render(&frag);
        assert!(output.contains("\"type\":\"fragment\""));
        assert!(output.contains("\"children\":["));
    }

    #[test]
    fn test_chart_json() {
        let chart = ContentNode::Chart(shape_value::content::ChartSpec {
            chart_type: shape_value::content::ChartType::Bar,
            series: vec![],
            title: Some("Sales".into()),
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let output = renderer().render(&chart);
        assert!(output.contains("\"chart_type\":\"bar\""));
        assert!(output.contains("\"title\":\"Sales\""));
    }

    #[test]
    fn test_json_string_escaping() {
        let node = ContentNode::plain("he said \"hello\" \\ \n\t");
        let output = renderer().render(&node);
        assert!(output.contains("\\\"hello\\\""));
        assert!(output.contains("\\\\"));
        assert!(output.contains("\\n"));
        assert!(output.contains("\\t"));
    }
}
