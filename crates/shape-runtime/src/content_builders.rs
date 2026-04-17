//! Content namespace builder functions.
//!
//! Static constructor functions for creating ContentNode values from the
//! `Content` namespace. These are intended to be registered as native
//! functions accessible from Shape code.
//!
//! - `Content.text(string)` — plain text node
//! - `Content.table(headers, rows)` — table node
//! - `Content.chart(chart_type)` — chart node (empty series)
//! - `Content.code(language, source)` — code block
//! - `Content.kv(pairs)` — key-value pairs
//! - `Content.fragment(parts)` — composition of content nodes

use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use shape_value::content::{
    BorderStyle, ChartSpec, ChartType, ContentNode, ContentTable, NamedColor,
};
use std::sync::Arc;

/// Create a plain text ContentNode.
///
/// `Content.text("hello")` → `ContentNode::plain("hello")`
pub fn content_text(args: &[ValueWord]) -> Result<ValueWord> {
    let text = args
        .first()
        .and_then(|nb| nb.as_str())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Content.text() requires a string argument".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_content(ContentNode::plain(text)))
}

/// Create a table ContentNode.
///
/// `Content.table(["Name", "Value"], [["a", "1"], ["b", "2"]])`
pub fn content_table(args: &[ValueWord]) -> Result<ValueWord> {
    // First arg: array of header strings
    let headers_arr = args
        .first()
        .and_then(|nb| nb.as_any_array())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Content.table() requires an array of headers as first argument".to_string(),
            location: None,
        })?
        .to_generic();

    let mut headers = Vec::new();
    for h in headers_arr.iter() {
        let s = h.as_str().ok_or_else(|| ShapeError::RuntimeError {
            message: "Table headers must be strings".to_string(),
            location: None,
        })?;
        headers.push(s.to_string());
    }

    // Second arg: array of rows (each row is an array of values)
    let rows_arr = args
        .get(1)
        .and_then(|nb| nb.as_any_array())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Content.table() requires an array of rows as second argument".to_string(),
            location: None,
        })?
        .to_generic();

    let mut rows = Vec::new();
    for row_nb in rows_arr.iter() {
        let row_arr = row_nb
            .as_any_array()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Each table row must be an array".to_string(),
                location: None,
            })?
            .to_generic();
        let mut cells = Vec::new();
        for cell in row_arr.iter() {
            // Convert each cell to a plain text ContentNode from its Display representation
            let text = if let Some(s) = cell.as_str() {
                s.to_string()
            } else {
                format!("{}", cell)
            };
            cells.push(ContentNode::plain(text));
        }
        rows.push(cells);
    }

    Ok(ValueWord::from_content(ContentNode::Table(ContentTable {
        headers,
        rows,
        border: BorderStyle::default(),
        max_rows: None,
        column_types: None,
        total_rows: None,
        sortable: false,
    })))
}

/// Create a chart ContentNode with empty series.
///
/// `Content.chart("line")` or `Content.chart("bar")`
pub fn content_chart(args: &[ValueWord]) -> Result<ValueWord> {
    let type_name =
        args.first()
            .and_then(|nb| nb.as_str())
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Content.chart() requires a chart type string".to_string(),
                location: None,
            })?;

    let chart_type = match type_name.to_lowercase().as_str() {
        "line" => ChartType::Line,
        "bar" => ChartType::Bar,
        "scatter" => ChartType::Scatter,
        "area" => ChartType::Area,
        "candlestick" => ChartType::Candlestick,
        "histogram" => ChartType::Histogram,
        "boxplot" | "box_plot" => ChartType::BoxPlot,
        "heatmap" => ChartType::Heatmap,
        "bubble" => ChartType::Bubble,
        other => {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Unknown chart type '{}'. Expected: line, bar, scatter, area, candlestick, histogram, boxplot, heatmap, bubble",
                    other
                ),
                location: None,
            });
        }
    };

    Ok(ValueWord::from_content(ContentNode::Chart(ChartSpec {
        chart_type,
        channels: vec![],
        x_categories: None,
        title: None,
        x_label: None,
        y_label: None,
        width: None,
        height: None,
        echarts_options: None,
        interactive: true,
    })))
}

/// Create a code block ContentNode.
///
/// `Content.code("rust", "fn main() {}")` or `Content.code(none, "plain code")`
pub fn content_code(args: &[ValueWord]) -> Result<ValueWord> {
    let language = args.first().and_then(|nb| {
        if nb.is_none() {
            None
        } else {
            nb.as_str().map(|s| s.to_string())
        }
    });

    let source =
        args.get(1)
            .and_then(|nb| nb.as_str())
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Content.code() requires a source string as second argument".to_string(),
                location: None,
            })?;

    Ok(ValueWord::from_content(ContentNode::Code {
        language,
        source: source.to_string(),
    }))
}

/// Create a key-value ContentNode.
///
/// Accepts an array of [key, value] pairs:
/// `Content.kv([["name", "Alice"], ["age", "30"]])`
pub fn content_kv(args: &[ValueWord]) -> Result<ValueWord> {
    let pairs_arr = args
        .first()
        .and_then(|nb| nb.as_any_array())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Content.kv() requires an array of [key, value] pairs".to_string(),
            location: None,
        })?
        .to_generic();

    let mut pairs = Vec::new();
    for pair_nb in pairs_arr.iter() {
        let pair_arr = pair_nb
            .as_any_array()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Each kv pair must be a [key, value] array".to_string(),
                location: None,
            })?
            .to_generic();
        let key = pair_arr
            .first()
            .and_then(|nb| nb.as_str())
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Key in kv pair must be a string".to_string(),
                location: None,
            })?
            .to_string();

        let value_nb = pair_arr.get(1).ok_or_else(|| ShapeError::RuntimeError {
            message: "Each kv pair must have both key and value".to_string(),
            location: None,
        })?;

        // If the value is already a ContentNode, use it directly; otherwise make plain text
        let value = if let Some(content) = value_nb.as_content() {
            content.clone()
        } else if let Some(s) = value_nb.as_str() {
            ContentNode::plain(s)
        } else {
            ContentNode::plain(format!("{}", shape_value::ValueWordDisplay(*value_nb)))
        };

        pairs.push((key, value));
    }

    Ok(ValueWord::from_content(ContentNode::KeyValue(pairs)))
}

/// Create a fragment ContentNode (composition of multiple nodes).
///
/// `Content.fragment([node1, node2, node3])`
pub fn content_fragment(args: &[ValueWord]) -> Result<ValueWord> {
    let parts_arr = args
        .first()
        .and_then(|nb| nb.as_any_array())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Content.fragment() requires an array of content nodes".to_string(),
            location: None,
        })?
        .to_generic();

    let mut parts = Vec::new();
    for part_nb in parts_arr.iter() {
        if let Some(content) = part_nb.as_content() {
            parts.push(content.clone());
        } else if let Some(s) = part_nb.as_str() {
            parts.push(ContentNode::plain(s));
        } else {
            parts.push(ContentNode::plain(format!("{}", shape_value::ValueWordDisplay(*part_nb))));
        }
    }

    Ok(ValueWord::from_content(ContentNode::Fragment(parts)))
}

// ========== Namespace value constructors ==========
// These produce ValueWord values for Color, Border, ChartType, and Align
// namespaces. They are designed to be called from the VM when accessing
// static properties like `Color.red` or `Border.rounded`.

/// Color namespace: `Color.red`, `Color.green`, etc.
/// Returns a ValueWord string tag that content methods accept as color args.
pub fn color_named(name: &str) -> Result<ValueWord> {
    // Validate the color name
    let _: NamedColor = match name.to_lowercase().as_str() {
        "red" => NamedColor::Red,
        "green" => NamedColor::Green,
        "blue" => NamedColor::Blue,
        "yellow" => NamedColor::Yellow,
        "magenta" => NamedColor::Magenta,
        "cyan" => NamedColor::Cyan,
        "white" => NamedColor::White,
        "default" => NamedColor::Default,
        _ => {
            return Err(ShapeError::RuntimeError {
                message: format!("Unknown color name '{}'", name),
                location: None,
            });
        }
    };
    Ok(ValueWord::from_string(Arc::new(name.to_lowercase())))
}

/// Color.rgb(r, g, b) — returns an RGB color string tag "rgb(r,g,b)".
pub fn color_rgb(args: &[ValueWord]) -> Result<ValueWord> {
    let r = args
        .first()
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Color.rgb() requires numeric r argument".to_string(),
            location: None,
        })? as u8;
    let g = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Color.rgb() requires numeric g argument".to_string(),
            location: None,
        })? as u8;
    let b = args
        .get(2)
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Color.rgb() requires numeric b argument".to_string(),
            location: None,
        })? as u8;
    Ok(ValueWord::from_string(Arc::new(format!(
        "rgb({},{},{})",
        r, g, b
    ))))
}

/// Border namespace: `Border.rounded`, `Border.sharp`, etc.
/// Returns a ValueWord string tag that content methods accept as border args.
pub fn border_named(name: &str) -> Result<ValueWord> {
    match name.to_lowercase().as_str() {
        "rounded" | "sharp" | "heavy" | "double" | "minimal" | "none" => {}
        _ => {
            return Err(ShapeError::RuntimeError {
                message: format!("Unknown border style '{}'", name),
                location: None,
            });
        }
    }
    Ok(ValueWord::from_string(Arc::new(name.to_lowercase())))
}

/// ChartType namespace: `ChartType.line`, `ChartType.bar`, etc.
/// Returns a ValueWord string tag that content builders accept as chart type args.
pub fn chart_type_named(name: &str) -> Result<ValueWord> {
    match name.to_lowercase().as_str() {
        "line" | "bar" | "scatter" | "area" | "candlestick" | "histogram" | "boxplot"
        | "heatmap" | "bubble" => {}
        _ => {
            return Err(ShapeError::RuntimeError {
                message: format!("Unknown chart type '{}'", name),
                location: None,
            });
        }
    }
    Ok(ValueWord::from_string(Arc::new(name.to_lowercase())))
}

/// Align namespace: `Align.left`, `Align.center`, `Align.right`.
/// Returns a ValueWord string tag for alignment directives.
pub fn align_named(name: &str) -> Result<ValueWord> {
    match name.to_lowercase().as_str() {
        "left" | "center" | "right" => {}
        _ => {
            return Err(ShapeError::RuntimeError {
                message: format!("Unknown alignment '{}'", name),
                location: None,
            });
        }
    }
    Ok(ValueWord::from_string(Arc::new(name.to_lowercase())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::ChartType;
    use std::sync::Arc;

    fn nb_str(s: &str) -> ValueWord {
        ValueWord::from_string(Arc::new(s.to_string()))
    }

    #[test]
    fn test_content_text() {
        let result = content_text(&[nb_str("hello")]).unwrap();
        let node = result.as_content().unwrap();
        assert_eq!(node.to_string(), "hello");
    }

    #[test]
    fn test_content_text_missing_arg() {
        assert!(content_text(&[]).is_err());
    }

    #[test]
    fn test_content_table() {
        let headers = ValueWord::from_array(shape_value::vmarray_from_vec(vec![nb_str("Name"), nb_str("Value")]));
        let row1 = ValueWord::from_array(shape_value::vmarray_from_vec(vec![nb_str("a"), nb_str("1")]));
        let rows = ValueWord::from_array(shape_value::vmarray_from_vec(vec![row1]));
        let result = content_table(&[headers, rows]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::Table(t) => {
                assert_eq!(t.headers, vec!["Name", "Value"]);
                assert_eq!(t.rows.len(), 1);
                assert_eq!(t.border, BorderStyle::Rounded);
            }
            _ => panic!("expected Table"),
        }
    }

    #[test]
    fn test_content_chart() {
        let result = content_chart(&[nb_str("line")]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::Chart(spec) => {
                assert_eq!(spec.chart_type, ChartType::Line);
                assert!(spec.channels.is_empty());
            }
            _ => panic!("expected Chart"),
        }
    }

    #[test]
    fn test_content_chart_invalid_type() {
        assert!(content_chart(&[nb_str("pie")]).is_err());
    }

    #[test]
    fn test_content_code() {
        let result = content_code(&[nb_str("rust"), nb_str("fn main() {}")]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::Code { language, source } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(source, "fn main() {}");
            }
            _ => panic!("expected Code"),
        }
    }

    #[test]
    fn test_content_code_no_language() {
        let result = content_code(&[ValueWord::none(), nb_str("plain text")]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::Code { language, source } => {
                assert!(language.is_none());
                assert_eq!(source, "plain text");
            }
            _ => panic!("expected Code"),
        }
    }

    #[test]
    fn test_content_kv() {
        let pair1 = ValueWord::from_array(shape_value::vmarray_from_vec(vec![nb_str("name"), nb_str("Alice")]));
        let pair2 = ValueWord::from_array(shape_value::vmarray_from_vec(vec![nb_str("age"), nb_str("30")]));
        let pairs = ValueWord::from_array(shape_value::vmarray_from_vec(vec![pair1, pair2]));
        let result = content_kv(&[pairs]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::KeyValue(kv) => {
                assert_eq!(kv.len(), 2);
                assert_eq!(kv[0].0, "name");
                assert_eq!(kv[1].0, "age");
            }
            _ => panic!("expected KeyValue"),
        }
    }

    #[test]
    fn test_content_fragment() {
        let n1 = ValueWord::from_content(ContentNode::plain("hello "));
        let n2 = ValueWord::from_content(ContentNode::plain("world"));
        let parts = ValueWord::from_array(shape_value::vmarray_from_vec(vec![n1, n2]));
        let result = content_fragment(&[parts]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::Fragment(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0].to_string(), "hello ");
                assert_eq!(parts[1].to_string(), "world");
            }
            _ => panic!("expected Fragment"),
        }
    }

    #[test]
    fn test_content_fragment_with_string_coercion() {
        let parts = ValueWord::from_array(shape_value::vmarray_from_vec(vec![nb_str("text")]));
        let result = content_fragment(&[parts]).unwrap();
        let node = result.as_content().unwrap();
        match node {
            ContentNode::Fragment(parts) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].to_string(), "text");
            }
            _ => panic!("expected Fragment"),
        }
    }

    #[test]
    fn test_color_named_valid() {
        let result = color_named("red").unwrap();
        assert_eq!(result.as_str().unwrap(), "red");
    }

    #[test]
    fn test_color_named_case_insensitive() {
        let result = color_named("GREEN").unwrap();
        assert_eq!(result.as_str().unwrap(), "green");
    }

    #[test]
    fn test_color_named_invalid() {
        assert!(color_named("purple").is_err());
    }

    #[test]
    fn test_color_rgb() {
        let result = color_rgb(&[
            ValueWord::from_f64(255.0),
            ValueWord::from_f64(128.0),
            ValueWord::from_f64(0.0),
        ])
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "rgb(255,128,0)");
    }

    #[test]
    fn test_border_named_valid() {
        assert_eq!(
            border_named("rounded").unwrap().as_str().unwrap(),
            "rounded"
        );
        assert_eq!(border_named("Heavy").unwrap().as_str().unwrap(), "heavy");
    }

    #[test]
    fn test_border_named_invalid() {
        assert!(border_named("dotted").is_err());
    }

    #[test]
    fn test_chart_type_named_valid() {
        assert_eq!(chart_type_named("line").unwrap().as_str().unwrap(), "line");
        assert_eq!(chart_type_named("Bar").unwrap().as_str().unwrap(), "bar");
    }

    #[test]
    fn test_chart_type_named_invalid() {
        assert!(chart_type_named("pie").is_err());
    }

    #[test]
    fn test_align_named_valid() {
        assert_eq!(align_named("left").unwrap().as_str().unwrap(), "left");
        assert_eq!(align_named("Center").unwrap().as_str().unwrap(), "center");
        assert_eq!(align_named("RIGHT").unwrap().as_str().unwrap(), "right");
    }

    #[test]
    fn test_align_named_invalid() {
        assert!(align_named("justify").is_err());
    }
}
