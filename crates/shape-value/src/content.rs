//! Structured content nodes — the output of Content.render().
//!
//! ContentNode is a rich, structured representation of rendered output that
//! supports styled text, tables, code blocks, charts, key-value pairs, and
//! fragments (compositions of multiple nodes).

use std::fmt;

/// A structured content node — the output of Content.render()
#[derive(Debug, Clone, PartialEq)]
pub enum ContentNode {
    /// Styled text with spans
    Text(StyledText),
    /// Table with headers, rows, and optional styling
    Table(ContentTable),
    /// Code block with optional language
    Code {
        language: Option<String>,
        source: String,
    },
    /// Chart specification
    Chart(ChartSpec),
    /// Key-value pairs
    KeyValue(Vec<(String, ContentNode)>),
    /// Composition of multiple content nodes
    Fragment(Vec<ContentNode>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyledText {
    pub spans: Vec<StyledSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyledSpan {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Named(NamedColor),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NamedColor {
    Red,
    Green,
    Blue,
    Yellow,
    Magenta,
    Cyan,
    White,
    Default,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContentTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<ContentNode>>,
    pub border: BorderStyle,
    pub max_rows: Option<usize>,
    /// Column type hints: "string", "number", "date", etc.
    pub column_types: Option<Vec<String>>,
    /// Total row count before truncation (for display: "showing 50 of 1000").
    pub total_rows: Option<usize>,
    /// Whether interactive renderers should enable column sorting.
    pub sortable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderStyle {
    Rounded,
    Sharp,
    Heavy,
    Double,
    Minimal,
    None,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::Rounded
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSpec {
    pub chart_type: ChartType,
    pub series: Vec<ChartSeries>,
    pub title: Option<String>,
    pub x_label: Option<String>,
    pub y_label: Option<String>,
    pub width: Option<usize>,
    pub height: Option<usize>,
    /// Full ECharts option JSON override (injected by chart detection).
    pub echarts_options: Option<serde_json::Value>,
    /// Whether this chart should be rendered interactively (default true).
    pub interactive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChartType {
    Line,
    Bar,
    Scatter,
    Area,
    Candlestick,
    Histogram,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    pub label: String,
    pub data: Vec<(f64, f64)>,
    pub color: Option<Color>,
}

impl ContentNode {
    /// Create a plain text node.
    pub fn plain(text: impl Into<String>) -> Self {
        ContentNode::Text(StyledText {
            spans: vec![StyledSpan {
                text: text.into(),
                style: Style::default(),
            }],
        })
    }

    /// Create a styled text node.
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        ContentNode::Text(StyledText {
            spans: vec![StyledSpan {
                text: text.into(),
                style,
            }],
        })
    }

    /// Apply foreground color to this node.
    pub fn with_fg(self, color: Color) -> Self {
        match self {
            ContentNode::Text(mut st) => {
                for span in &mut st.spans {
                    span.style.fg = Some(color.clone());
                }
                ContentNode::Text(st)
            }
            other => other,
        }
    }

    /// Apply background color.
    pub fn with_bg(self, color: Color) -> Self {
        match self {
            ContentNode::Text(mut st) => {
                for span in &mut st.spans {
                    span.style.bg = Some(color.clone());
                }
                ContentNode::Text(st)
            }
            other => other,
        }
    }

    /// Apply bold.
    pub fn with_bold(self) -> Self {
        match self {
            ContentNode::Text(mut st) => {
                for span in &mut st.spans {
                    span.style.bold = true;
                }
                ContentNode::Text(st)
            }
            other => other,
        }
    }

    /// Apply italic.
    pub fn with_italic(self) -> Self {
        match self {
            ContentNode::Text(mut st) => {
                for span in &mut st.spans {
                    span.style.italic = true;
                }
                ContentNode::Text(st)
            }
            other => other,
        }
    }

    /// Apply underline.
    pub fn with_underline(self) -> Self {
        match self {
            ContentNode::Text(mut st) => {
                for span in &mut st.spans {
                    span.style.underline = true;
                }
                ContentNode::Text(st)
            }
            other => other,
        }
    }

    /// Apply dim.
    pub fn with_dim(self) -> Self {
        match self {
            ContentNode::Text(mut st) => {
                for span in &mut st.spans {
                    span.style.dim = true;
                }
                ContentNode::Text(st)
            }
            other => other,
        }
    }
}

impl fmt::Display for ContentNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentNode::Text(st) => {
                for span in &st.spans {
                    write!(f, "{}", span.text)?;
                }
                Ok(())
            }
            ContentNode::Table(table) => {
                if !table.headers.is_empty() {
                    for (i, header) in table.headers.iter().enumerate() {
                        if i > 0 {
                            write!(f, " | ")?;
                        }
                        write!(f, "{}", header)?;
                    }
                    writeln!(f)?;
                    for (i, _) in table.headers.iter().enumerate() {
                        if i > 0 {
                            write!(f, "-+-")?;
                        }
                        write!(f, "---")?;
                    }
                    writeln!(f)?;
                }
                let limit = table.max_rows.unwrap_or(table.rows.len());
                for row in table.rows.iter().take(limit) {
                    for (i, cell) in row.iter().enumerate() {
                        if i > 0 {
                            write!(f, " | ")?;
                        }
                        write!(f, "{}", cell)?;
                    }
                    writeln!(f)?;
                }
                Ok(())
            }
            ContentNode::Code { source, .. } => write!(f, "{}", source),
            ContentNode::Chart(spec) => {
                write!(
                    f,
                    "[Chart: {}]",
                    spec.title.as_deref().unwrap_or("untitled")
                )
            }
            ContentNode::KeyValue(pairs) => {
                for (i, (key, value)) in pairs.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", key, value)?;
                }
                Ok(())
            }
            ContentNode::Fragment(parts) => {
                for part in parts {
                    write!(f, "{}", part)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_node() {
        let node = ContentNode::plain("hello world");
        match &node {
            ContentNode::Text(st) => {
                assert_eq!(st.spans.len(), 1);
                assert_eq!(st.spans[0].text, "hello world");
                assert_eq!(st.spans[0].style, Style::default());
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_styled_text_node() {
        let style = Style {
            bold: true,
            fg: Some(Color::Named(NamedColor::Red)),
            ..Default::default()
        };
        let node = ContentNode::styled("warning", style.clone());
        match &node {
            ContentNode::Text(st) => {
                assert_eq!(st.spans.len(), 1);
                assert_eq!(st.spans[0].text, "warning");
                assert_eq!(st.spans[0].style, style);
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_content_node_display() {
        assert_eq!(ContentNode::plain("hello").to_string(), "hello");

        let code = ContentNode::Code {
            language: Some("rust".into()),
            source: "fn main() {}".into(),
        };
        assert_eq!(code.to_string(), "fn main() {}");

        let chart = ContentNode::Chart(ChartSpec {
            chart_type: ChartType::Line,
            series: vec![],
            title: Some("My Chart".into()),
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        assert_eq!(chart.to_string(), "[Chart: My Chart]");

        let chart_no_title = ContentNode::Chart(ChartSpec {
            chart_type: ChartType::Bar,
            series: vec![],
            title: None,
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        assert_eq!(chart_no_title.to_string(), "[Chart: untitled]");
    }

    #[test]
    fn test_with_fg_color() {
        let node = ContentNode::plain("text").with_fg(Color::Named(NamedColor::Green));
        match &node {
            ContentNode::Text(st) => {
                assert_eq!(st.spans[0].style.fg, Some(Color::Named(NamedColor::Green)));
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_with_bold() {
        let node = ContentNode::plain("text").with_bold();
        match &node {
            ContentNode::Text(st) => {
                assert!(st.spans[0].style.bold);
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_with_italic() {
        let node = ContentNode::plain("text").with_italic();
        match &node {
            ContentNode::Text(st) => {
                assert!(st.spans[0].style.italic);
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_with_underline() {
        let node = ContentNode::plain("text").with_underline();
        match &node {
            ContentNode::Text(st) => {
                assert!(st.spans[0].style.underline);
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_with_dim() {
        let node = ContentNode::plain("text").with_dim();
        match &node {
            ContentNode::Text(st) => {
                assert!(st.spans[0].style.dim);
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_with_bg_color() {
        let node = ContentNode::plain("text").with_bg(Color::Rgb(255, 0, 0));
        match &node {
            ContentNode::Text(st) => {
                assert_eq!(st.spans[0].style.bg, Some(Color::Rgb(255, 0, 0)));
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_style_chaining() {
        let node = ContentNode::plain("text")
            .with_bold()
            .with_fg(Color::Named(NamedColor::Cyan))
            .with_underline();
        match &node {
            ContentNode::Text(st) => {
                assert!(st.spans[0].style.bold);
                assert!(st.spans[0].style.underline);
                assert_eq!(st.spans[0].style.fg, Some(Color::Named(NamedColor::Cyan)));
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_non_text_node_style_passthrough() {
        let code = ContentNode::Code {
            language: None,
            source: "x = 1".into(),
        };
        let result = code.with_bold();
        match &result {
            ContentNode::Code { source, .. } => assert_eq!(source, "x = 1"),
            _ => panic!("expected Code variant"),
        }
    }

    #[test]
    fn test_fragment_composition() {
        let frag = ContentNode::Fragment(vec![
            ContentNode::plain("hello "),
            ContentNode::plain("world"),
        ]);
        assert_eq!(frag.to_string(), "hello world");
    }

    #[test]
    fn test_key_value_display() {
        let kv = ContentNode::KeyValue(vec![
            ("name".into(), ContentNode::plain("Alice")),
            ("age".into(), ContentNode::plain("30")),
        ]);
        assert_eq!(kv.to_string(), "name: Alice\nage: 30");
    }

    #[test]
    fn test_table_display() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["Name".into(), "Value".into()],
            rows: vec![
                vec![ContentNode::plain("a"), ContentNode::plain("1")],
                vec![ContentNode::plain("b"), ContentNode::plain("2")],
            ],
            border: BorderStyle::default(),
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = table.to_string();
        assert!(output.contains("Name"));
        assert!(output.contains("Value"));
        assert!(output.contains("a"));
        assert!(output.contains("1"));
        assert!(output.contains("b"));
        assert!(output.contains("2"));
    }

    #[test]
    fn test_table_max_rows() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![
                vec![ContentNode::plain("1")],
                vec![ContentNode::plain("2")],
                vec![ContentNode::plain("3")],
            ],
            border: BorderStyle::None,
            max_rows: Some(2),
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let output = table.to_string();
        assert!(output.contains("1"));
        assert!(output.contains("2"));
        assert!(!output.contains("3"));
    }

    #[test]
    fn test_content_node_equality() {
        let a = ContentNode::plain("hello");
        let b = ContentNode::plain("hello");
        let c = ContentNode::plain("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_border_style_default() {
        assert_eq!(BorderStyle::default(), BorderStyle::Rounded);
    }
}
