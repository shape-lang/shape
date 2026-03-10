//! HTML renderer — renders ContentNode to HTML output.
//!
//! Produces HTML with:
//! - `<span>` elements with inline styles for text styling
//! - `<table>` elements for tables
//! - `<pre><code>` for code blocks
//! - Placeholder `<div>` for charts
//! - `<dl>` for key-value pairs

use crate::content_renderer::{ContentRenderer, RenderContext, RendererCapabilities};
use shape_value::content::{ChartSpec, Color, ContentNode, ContentTable, NamedColor, Style};
use std::fmt::Write;

/// Renders ContentNode trees to HTML.
///
/// Carries a [`RenderContext`] — when `ctx.interactive` is true, chart nodes
/// emit `data-echarts` attributes for client-side hydration.
pub struct HtmlRenderer {
    pub ctx: RenderContext,
}

impl HtmlRenderer {
    pub fn new() -> Self {
        Self {
            ctx: RenderContext::html(),
        }
    }

    pub fn with_context(ctx: RenderContext) -> Self {
        Self { ctx }
    }
}

impl Default for HtmlRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentRenderer for HtmlRenderer {
    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities::html()
    }

    fn render(&self, content: &ContentNode) -> String {
        render_node(content, self.ctx.interactive)
    }
}

fn render_node(node: &ContentNode, interactive: bool) -> String {
    match node {
        ContentNode::Text(st) => {
            let mut out = String::new();
            for span in &st.spans {
                let css = style_to_css(&span.style);
                if css.is_empty() {
                    let _ = write!(out, "{}", html_escape(&span.text));
                } else {
                    let _ = write!(
                        out,
                        "<span style=\"{}\">{}</span>",
                        css,
                        html_escape(&span.text)
                    );
                }
            }
            out
        }
        ContentNode::Table(table) => render_table(table, interactive),
        ContentNode::Code { language, source } => render_code(language.as_deref(), source),
        ContentNode::Chart(spec) => render_chart(spec, interactive),
        ContentNode::KeyValue(pairs) => render_key_value(pairs, interactive),
        ContentNode::Fragment(parts) => parts.iter().map(|n| render_node(n, interactive)).collect(),
    }
}

fn style_to_css(style: &Style) -> String {
    let mut parts = Vec::new();
    if style.bold {
        parts.push("font-weight:bold".to_string());
    }
    if style.italic {
        parts.push("font-style:italic".to_string());
    }
    if style.underline {
        parts.push("text-decoration:underline".to_string());
    }
    if style.dim {
        parts.push("opacity:0.6".to_string());
    }
    if let Some(ref color) = style.fg {
        parts.push(format!("color:{}", color_to_css(color)));
    }
    if let Some(ref color) = style.bg {
        parts.push(format!("background-color:{}", color_to_css(color)));
    }
    parts.join(";")
}

fn color_to_css(color: &Color) -> String {
    match color {
        Color::Named(named) => named_to_css(*named).to_string(),
        Color::Rgb(r, g, b) => format!("rgb({},{},{})", r, g, b),
    }
}

fn named_to_css(color: NamedColor) -> &'static str {
    match color {
        NamedColor::Red => "red",
        NamedColor::Green => "green",
        NamedColor::Blue => "blue",
        NamedColor::Yellow => "yellow",
        NamedColor::Magenta => "magenta",
        NamedColor::Cyan => "cyan",
        NamedColor::White => "white",
        NamedColor::Default => "inherit",
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_table(table: &ContentTable, interactive: bool) -> String {
    let mut out = String::from("<table>\n");

    // Header
    if !table.headers.is_empty() {
        out.push_str("<thead><tr>");
        for header in &table.headers {
            let _ = write!(out, "<th>{}</th>", html_escape(header));
        }
        out.push_str("</tr></thead>\n");
    }

    // Body
    let limit = table.max_rows.unwrap_or(table.rows.len());
    let display_rows = &table.rows[..limit.min(table.rows.len())];
    let truncated = table.rows.len().saturating_sub(limit);

    out.push_str("<tbody>\n");
    for row in display_rows {
        out.push_str("<tr>");
        for cell in row {
            let _ = write!(out, "<td>{}</td>", render_node(cell, interactive));
        }
        out.push_str("</tr>\n");
    }
    if truncated > 0 {
        let _ = write!(
            out,
            "<tr><td colspan=\"{}\">... {} more rows</td></tr>\n",
            table.headers.len(),
            truncated
        );
    }
    out.push_str("</tbody>\n</table>");
    out
}

fn render_code(language: Option<&str>, source: &str) -> String {
    let lang_attr = language
        .map(|l| format!(" class=\"language-{}\"", html_escape(l)))
        .unwrap_or_default();
    format!(
        "<pre><code{}>{}</code></pre>",
        lang_attr,
        html_escape(source)
    )
}

fn render_chart(spec: &ChartSpec, interactive: bool) -> String {
    let title = spec.title.as_deref().unwrap_or("untitled");
    let type_name = chart_type_display_name(spec.chart_type);
    let y_count = spec.channels_by_name("y").len();
    if interactive {
        // Build ECharts option JSON for client-side hydration
        let echarts_json = build_echarts_option(spec, type_name);
        let escaped_json = html_escape(&echarts_json);
        format!(
            "<div class=\"chart\" data-echarts=\"true\" data-type=\"{}\" data-title=\"{}\" data-chart-options=\"{}\">[{} Chart: {}]</div>",
            type_name.to_lowercase(),
            html_escape(title),
            escaped_json,
            type_name,
            html_escape(title)
        )
    } else {
        format!(
            "<div class=\"chart\" data-type=\"{}\" data-series=\"{}\">[{} Chart: {}]</div>",
            type_name.to_lowercase(),
            y_count,
            type_name,
            html_escape(title)
        )
    }
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

/// Build an ECharts option JSON string from a ChartSpec.
fn build_echarts_option(spec: &ChartSpec, type_name: &str) -> String {
    // If echarts_options is already set, use it directly
    if let Some(ref opts) = spec.echarts_options {
        return serde_json::to_string(opts).unwrap_or_default();
    }

    let chart_type = type_name.to_lowercase();

    // Bar/histogram charts use category xAxis; line/scatter/area use value xAxis
    let use_category = matches!(chart_type.as_str(), "bar" | "histogram");

    // Get x channel data and y channels
    let x_channel = spec.channel("x");
    let y_channels = spec.channels_by_name("y");

    // Extract x-axis categories from x channel
    let categories: Vec<serde_json::Value> = if use_category {
        if let Some(ref cats) = spec.x_categories {
            cats.iter().map(|c| serde_json::json!(c)).collect()
        } else if let Some(xc) = x_channel {
            xc.values
                .iter()
                .map(|x| {
                    if x.fract() == 0.0 {
                        serde_json::json!(*x as i64)
                    } else {
                        serde_json::json!(x)
                    }
                })
                .collect()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Build ECharts series array from y channels
    let series: Vec<serde_json::Value> = if y_channels.is_empty() {
        vec![serde_json::json!({"type": chart_type, "data": []})]
    } else {
        y_channels
            .iter()
            .map(|yc| {
                if use_category {
                    let data: Vec<serde_json::Value> =
                        yc.values.iter().map(|y| serde_json::json!(y)).collect();
                    serde_json::json!({
                        "name": yc.label,
                        "type": chart_type,
                        "data": data,
                    })
                } else {
                    // Pair x and y values
                    let x_vals = x_channel.map(|xc| &xc.values[..]).unwrap_or(&[]);
                    let data: Vec<serde_json::Value> = yc
                        .values
                        .iter()
                        .enumerate()
                        .map(|(i, y)| {
                            let x = x_vals.get(i).copied().unwrap_or(i as f64);
                            serde_json::json!([x, y])
                        })
                        .collect();
                    serde_json::json!({
                        "name": yc.label,
                        "type": chart_type,
                        "data": data,
                        "smooth": false,
                    })
                }
            })
            .collect()
    };

    let mut option = serde_json::json!({
        "tooltip": {"trigger": "axis"},
        "series": series,
        "backgroundColor": "transparent",
    });

    if let Some(ref t) = spec.title {
        option["title"] =
            serde_json::json!({"text": t, "textStyle": {"color": "#ccc", "fontSize": 14}});
    }

    // xAxis: category for bar/histogram, value for others
    let x_axis_type = if use_category { "category" } else { "value" };
    let mut x_axis = serde_json::json!({
        "type": x_axis_type,
        "axisLabel": {"color": "#888"},
        "axisLine": {"lineStyle": {"color": "#555"}},
    });
    if use_category && !categories.is_empty() {
        x_axis["data"] = serde_json::json!(categories);
    }
    if let Some(ref xl) = spec.x_label {
        x_axis["name"] = serde_json::json!(xl);
        x_axis["nameTextStyle"] = serde_json::json!({"color": "#888"});
    }
    option["xAxis"] = x_axis;

    let mut y_axis = serde_json::json!({
        "type": "value",
        "axisLabel": {"color": "#888"},
        "splitLine": {"lineStyle": {"color": "#333"}},
    });
    if let Some(ref yl) = spec.y_label {
        y_axis["name"] = serde_json::json!(yl);
        y_axis["nameTextStyle"] = serde_json::json!({"color": "#888"});
    }
    option["yAxis"] = y_axis;

    if y_channels.len() > 1 {
        option["legend"] = serde_json::json!({"show": true, "textStyle": {"color": "#ccc"}});
    }

    option["grid"] =
        serde_json::json!({"left": "10%", "right": "10%", "bottom": "10%", "top": "15%"});

    serde_json::to_string(&option).unwrap_or_default()
}

fn render_key_value(pairs: &[(String, ContentNode)], interactive: bool) -> String {
    let mut out = String::from("<dl>\n");
    for (key, value) in pairs {
        let _ = write!(
            out,
            "<dt>{}</dt><dd>{}</dd>\n",
            html_escape(key),
            render_node(value, interactive)
        );
    }
    out.push_str("</dl>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::{BorderStyle, ContentTable};

    fn renderer() -> HtmlRenderer {
        HtmlRenderer::new()
    }

    #[test]
    fn test_plain_text_html() {
        let node = ContentNode::plain("hello world");
        let output = renderer().render(&node);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_bold_text_html() {
        let node = ContentNode::plain("bold").with_bold();
        let output = renderer().render(&node);
        assert!(output.contains("font-weight:bold"));
        assert!(output.contains("<span"));
        assert!(output.contains("bold"));
    }

    #[test]
    fn test_fg_color_html() {
        let node = ContentNode::plain("red").with_fg(Color::Named(NamedColor::Red));
        let output = renderer().render(&node);
        assert!(output.contains("color:red"));
    }

    #[test]
    fn test_rgb_color_html() {
        let node = ContentNode::plain("custom").with_fg(Color::Rgb(255, 128, 0));
        let output = renderer().render(&node);
        assert!(output.contains("color:rgb(255,128,0)"));
    }

    #[test]
    fn test_html_table() {
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
        assert!(output.contains("<table>"));
        assert!(output.contains("<th>Name</th>"));
        assert!(output.contains("<td>Alice</td>"));
        assert!(output.contains("</table>"));
    }

    #[test]
    fn test_html_table_truncation() {
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
        assert!(output.contains("... 2 more rows"));
    }

    #[test]
    fn test_html_code() {
        let code = ContentNode::Code {
            language: Some("rust".into()),
            source: "fn main() {}".into(),
        };
        let output = renderer().render(&code);
        assert!(output.contains("<pre><code class=\"language-rust\">"));
        assert!(output.contains("fn main() {}"));
    }

    #[test]
    fn test_html_escape() {
        let node = ContentNode::plain("<script>alert('xss')</script>");
        let output = renderer().render(&node);
        assert!(!output.contains("<script>"));
        assert!(output.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_html_kv() {
        let kv = ContentNode::KeyValue(vec![("name".into(), ContentNode::plain("Alice"))]);
        let output = renderer().render(&kv);
        assert!(output.contains("<dl>"));
        assert!(output.contains("<dt>name</dt>"));
        assert!(output.contains("<dd>Alice</dd>"));
    }

    #[test]
    fn test_html_fragment() {
        let frag = ContentNode::Fragment(vec![
            ContentNode::plain("hello "),
            ContentNode::plain("world"),
        ]);
        let output = renderer().render(&frag);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_html_chart() {
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
        assert!(output.contains("data-type=\"bar\""));
        assert!(output.contains("Sales"));
    }
}
