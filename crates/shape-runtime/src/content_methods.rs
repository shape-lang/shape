//! Content method dispatch for ContentNode instance methods.
//!
//! Provides method handler functions for ContentNode values. These follow the
//! same signature pattern as other method handlers in the codebase: they receive
//! a receiver (the ContentNode) + args as `Vec<ValueWord>` and return `Result<ValueWord>`.
//!
//! Methods:
//! - Style: `fg(color)`, `bg(color)`, `bold()`, `italic()`, `underline()`, `dim()`
//! - Table: `border(style)`, `max_rows(n)`
//! - Chart: `series(label, data)`, `title(s)`, `x_label(s)`, `y_label(s)`

use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use shape_value::content::{BorderStyle, ChartChannel, Color, ContentNode, NamedColor};

/// Look up and call a content method by name.
///
/// Returns `Some(result)` if the method was found, `None` if not recognized.
pub fn call_content_method(
    method_name: &str,
    receiver: ValueWord,
    args: Vec<ValueWord>,
) -> Option<Result<ValueWord>> {
    match method_name {
        // Style methods
        "fg" => Some(handle_fg(receiver, args)),
        "bg" => Some(handle_bg(receiver, args)),
        "bold" => Some(handle_bold(receiver, args)),
        "italic" => Some(handle_italic(receiver, args)),
        "underline" => Some(handle_underline(receiver, args)),
        "dim" => Some(handle_dim(receiver, args)),
        // Table methods
        "border" => Some(handle_border(receiver, args)),
        "max_rows" | "maxRows" => Some(handle_max_rows(receiver, args)),
        // Chart methods
        "series" => Some(handle_series(receiver, args)),
        "title" => Some(handle_title(receiver, args)),
        "x_label" | "xLabel" => Some(handle_x_label(receiver, args)),
        "y_label" | "yLabel" => Some(handle_y_label(receiver, args)),
        _ => None,
    }
}

/// Parse a color string into a Color value.
fn parse_color(s: &str) -> Result<Color> {
    match s.to_lowercase().as_str() {
        "red" => Ok(Color::Named(NamedColor::Red)),
        "green" => Ok(Color::Named(NamedColor::Green)),
        "blue" => Ok(Color::Named(NamedColor::Blue)),
        "yellow" => Ok(Color::Named(NamedColor::Yellow)),
        "magenta" => Ok(Color::Named(NamedColor::Magenta)),
        "cyan" => Ok(Color::Named(NamedColor::Cyan)),
        "white" => Ok(Color::Named(NamedColor::White)),
        "default" => Ok(Color::Named(NamedColor::Default)),
        other => Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown color '{}'. Expected: red, green, blue, yellow, magenta, cyan, white, default",
                other
            ),
            location: None,
        }),
    }
}

/// Extract a ContentNode from the receiver ValueWord.
fn extract_content(receiver: &ValueWord) -> Result<ContentNode> {
    receiver
        .as_content()
        .cloned()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Expected a ContentNode receiver".to_string(),
            location: None,
        })
}

/// Extract a required string argument.
fn require_string_arg(args: &[ValueWord], index: usize, label: &str) -> Result<String> {
    args.get(index)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| ShapeError::RuntimeError {
            message: format!("{} requires a string argument", label),
            location: None,
        })
}

fn handle_fg(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let color_name = require_string_arg(&args, 0, "fg")?;
    let color = parse_color(&color_name)?;
    Ok(ValueWord::from_content(node.with_fg(color)))
}

fn handle_bg(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let color_name = require_string_arg(&args, 0, "bg")?;
    let color = parse_color(&color_name)?;
    Ok(ValueWord::from_content(node.with_bg(color)))
}

fn handle_bold(receiver: ValueWord, _args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    Ok(ValueWord::from_content(node.with_bold()))
}

fn handle_italic(receiver: ValueWord, _args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    Ok(ValueWord::from_content(node.with_italic()))
}

fn handle_underline(receiver: ValueWord, _args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    Ok(ValueWord::from_content(node.with_underline()))
}

fn handle_dim(receiver: ValueWord, _args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    Ok(ValueWord::from_content(node.with_dim()))
}

fn handle_border(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let style_name = require_string_arg(&args, 0, "border")?;
    let border = match style_name.to_lowercase().as_str() {
        "rounded" => BorderStyle::Rounded,
        "sharp" => BorderStyle::Sharp,
        "heavy" => BorderStyle::Heavy,
        "double" => BorderStyle::Double,
        "minimal" => BorderStyle::Minimal,
        "none" => BorderStyle::None,
        other => {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Unknown border style '{}'. Expected: rounded, sharp, heavy, double, minimal, none",
                    other
                ),
                location: None,
            });
        }
    };
    match node {
        ContentNode::Table(mut table) => {
            table.border = border;
            Ok(ValueWord::from_content(ContentNode::Table(table)))
        }
        other => Ok(ValueWord::from_content(other)),
    }
}

fn handle_max_rows(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let n = args
        .first()
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "max_rows requires a numeric argument".to_string(),
            location: None,
        })? as usize;
    match node {
        ContentNode::Table(mut table) => {
            table.max_rows = Some(n);
            Ok(ValueWord::from_content(ContentNode::Table(table)))
        }
        other => Ok(ValueWord::from_content(other)),
    }
}

fn handle_series(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let label = require_string_arg(&args, 0, "series")?;
    // Second arg: data as array of [x, y] pairs
    let mut x_values = Vec::new();
    let mut y_values = Vec::new();
    if let Some(view) = args.get(1).and_then(|nb| nb.as_any_array()) {
        let arr = view.to_generic();
        for item in arr.iter() {
            if let Some(inner) = item.as_any_array() {
                let inner = inner.to_generic();
                if inner.len() >= 2 {
                    if let (Some(x), Some(y)) =
                        (inner[0].as_number_coerce(), inner[1].as_number_coerce())
                    {
                        x_values.push(x);
                        y_values.push(y);
                    }
                }
            }
        }
    }
    match node {
        ContentNode::Chart(mut spec) => {
            // Add x channel if not already present
            if spec.channel("x").is_none() && !x_values.is_empty() {
                spec.channels.push(ChartChannel {
                    name: "x".to_string(),
                    label: "x".to_string(),
                    values: x_values,
                    color: None,
                });
            }
            spec.channels.push(ChartChannel {
                name: "y".to_string(),
                label,
                values: y_values,
                color: None,
            });
            Ok(ValueWord::from_content(ContentNode::Chart(spec)))
        }
        other => Ok(ValueWord::from_content(other)),
    }
}

fn handle_title(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let title = require_string_arg(&args, 0, "title")?;
    match node {
        ContentNode::Chart(mut spec) => {
            spec.title = Some(title);
            Ok(ValueWord::from_content(ContentNode::Chart(spec)))
        }
        other => Ok(ValueWord::from_content(other)),
    }
}

fn handle_x_label(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let label = require_string_arg(&args, 0, "x_label")?;
    match node {
        ContentNode::Chart(mut spec) => {
            spec.x_label = Some(label);
            Ok(ValueWord::from_content(ContentNode::Chart(spec)))
        }
        other => Ok(ValueWord::from_content(other)),
    }
}

fn handle_y_label(receiver: ValueWord, args: Vec<ValueWord>) -> Result<ValueWord> {
    let node = extract_content(&receiver)?;
    let label = require_string_arg(&args, 0, "y_label")?;
    match node {
        ContentNode::Chart(mut spec) => {
            spec.y_label = Some(label);
            Ok(ValueWord::from_content(ContentNode::Chart(spec)))
        }
        other => Ok(ValueWord::from_content(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::ContentTable;
    use std::sync::Arc;

    fn nb_str(s: &str) -> ValueWord {
        ValueWord::from_string(Arc::new(s.to_string()))
    }

    #[test]
    fn test_call_content_method_lookup() {
        let node = ValueWord::from_content(ContentNode::plain("hello"));
        assert!(call_content_method("bold", node.clone(), vec![]).is_some());
        assert!(call_content_method("italic", node.clone(), vec![]).is_some());
        assert!(call_content_method("underline", node.clone(), vec![]).is_some());
        assert!(call_content_method("dim", node.clone(), vec![]).is_some());
        assert!(call_content_method("unknown", node, vec![]).is_none());
    }

    #[test]
    fn test_fg_method() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_fg(node, vec![nb_str("red")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => {
                assert_eq!(st.spans[0].style.fg, Some(Color::Named(NamedColor::Red)));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_bg_method() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_bg(node, vec![nb_str("blue")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => {
                assert_eq!(st.spans[0].style.bg, Some(Color::Named(NamedColor::Blue)));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_bold_method() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_bold(node, vec![]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => assert!(st.spans[0].style.bold),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_italic_method() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_italic(node, vec![]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => assert!(st.spans[0].style.italic),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_underline_method() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_underline(node, vec![]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => assert!(st.spans[0].style.underline),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_dim_method() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_dim(node, vec![]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => assert!(st.spans[0].style.dim),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_border_method_on_table() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["A".into()],
            rows: vec![vec![ContentNode::plain("1")]],
            border: BorderStyle::Rounded,
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let node = ValueWord::from_content(table);
        let result = handle_border(node, vec![nb_str("heavy")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Table(t) => assert_eq!(t.border, BorderStyle::Heavy),
            _ => panic!("expected Table"),
        }
    }

    #[test]
    fn test_border_method_on_non_table() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_border(node, vec![nb_str("sharp")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => assert_eq!(st.spans[0].text, "text"),
            _ => panic!("expected Text passthrough"),
        }
    }

    #[test]
    fn test_max_rows_method() {
        let table = ContentNode::Table(ContentTable {
            headers: vec!["X".into()],
            rows: vec![
                vec![ContentNode::plain("1")],
                vec![ContentNode::plain("2")],
                vec![ContentNode::plain("3")],
            ],
            border: BorderStyle::default(),
            max_rows: None,
            column_types: None,
            total_rows: None,
            sortable: false,
        });
        let node = ValueWord::from_content(table);
        let result = handle_max_rows(node, vec![ValueWord::from_i64(2)]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Table(t) => assert_eq!(t.max_rows, Some(2)),
            _ => panic!("expected Table"),
        }
    }

    #[test]
    fn test_parse_color_valid() {
        assert_eq!(parse_color("red").unwrap(), Color::Named(NamedColor::Red));
        assert_eq!(
            parse_color("GREEN").unwrap(),
            Color::Named(NamedColor::Green)
        );
        assert_eq!(parse_color("Blue").unwrap(), Color::Named(NamedColor::Blue));
    }

    #[test]
    fn test_parse_color_invalid() {
        assert!(parse_color("purple").is_err());
    }

    #[test]
    fn test_fg_invalid_color() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let result = handle_fg(node, vec![nb_str("purple")]);
        assert!(result.is_err());
    }

    #[test]
    fn test_style_chaining_via_methods() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        let bold_result = handle_bold(node, vec![]).unwrap();
        let fg_result = handle_fg(bold_result, vec![nb_str("cyan")]).unwrap();
        let content = fg_result.as_content().unwrap();
        match content {
            ContentNode::Text(st) => {
                assert!(st.spans[0].style.bold);
                assert_eq!(st.spans[0].style.fg, Some(Color::Named(NamedColor::Cyan)));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_chart_title_method() {
        use shape_value::content::{ChartSpec, ChartType};
        let chart = ContentNode::Chart(ChartSpec {
            chart_type: ChartType::Line,
            channels: vec![],
            x_categories: None,
            title: None,
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let node = ValueWord::from_content(chart);
        let result = handle_title(node, vec![nb_str("Revenue")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Chart(spec) => assert_eq!(spec.title.as_deref(), Some("Revenue")),
            _ => panic!("expected Chart"),
        }
    }

    #[test]
    fn test_chart_x_label_method() {
        use shape_value::content::{ChartSpec, ChartType};
        let chart = ContentNode::Chart(ChartSpec {
            chart_type: ChartType::Bar,
            channels: vec![],
            x_categories: None,
            title: None,
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let node = ValueWord::from_content(chart);
        let result = handle_x_label(node, vec![nb_str("Time")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Chart(spec) => assert_eq!(spec.x_label.as_deref(), Some("Time")),
            _ => panic!("expected Chart"),
        }
    }

    #[test]
    fn test_chart_y_label_method() {
        use shape_value::content::{ChartSpec, ChartType};
        let chart = ContentNode::Chart(ChartSpec {
            chart_type: ChartType::Line,
            channels: vec![],
            x_categories: None,
            title: None,
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let node = ValueWord::from_content(chart);
        let result = handle_y_label(node, vec![nb_str("Value")]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Chart(spec) => assert_eq!(spec.y_label.as_deref(), Some("Value")),
            _ => panic!("expected Chart"),
        }
    }

    #[test]
    fn test_chart_series_method() {
        use shape_value::content::{ChartSpec, ChartType};
        let chart = ContentNode::Chart(ChartSpec {
            chart_type: ChartType::Line,
            channels: vec![],
            x_categories: None,
            title: None,
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: true,
        });
        let node = ValueWord::from_content(chart);
        let data_points = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_array(shape_value::vmarray_from_vec(vec![
                ValueWord::from_f64(1.0),
                ValueWord::from_f64(10.0),
            ])),
            ValueWord::from_array(shape_value::vmarray_from_vec(vec![
                ValueWord::from_f64(2.0),
                ValueWord::from_f64(20.0),
            ])),
        ]));
        let result = handle_series(node, vec![nb_str("Sales"), data_points]).unwrap();
        let content = result.as_content().unwrap();
        match content {
            ContentNode::Chart(spec) => {
                // x channel + y channel = 2 channels
                assert_eq!(spec.channels.len(), 2);
                assert_eq!(spec.channel("x").unwrap().values, vec![1.0, 2.0]);
                let y = spec.channels_by_name("y");
                assert_eq!(y.len(), 1);
                assert_eq!(y[0].label, "Sales");
                assert_eq!(y[0].values, vec![10.0, 20.0]);
            }
            _ => panic!("expected Chart"),
        }
    }

    #[test]
    fn test_chart_method_lookup() {
        let node = ValueWord::from_content(ContentNode::plain("text"));
        assert!(call_content_method("title", node.clone(), vec![nb_str("t")]).is_some());
        assert!(call_content_method("series", node.clone(), vec![]).is_some());
        assert!(call_content_method("xLabel", node.clone(), vec![nb_str("x")]).is_some());
        assert!(call_content_method("yLabel", node, vec![nb_str("y")]).is_some());
    }
}
