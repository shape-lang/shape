//! Chart auto-detection from Arrow IPC table data.
//!
//! Inspects Arrow schemas to determine appropriate chart types and generates
//! ECharts option JSON with embedded data. Also provides a channel-based
//! `ChartSpec` output for unified rendering.

use arrow_ipc::reader::StreamReader;
use shape_value::ValueWordExt;
use arrow_schema::{DataType, Schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Cursor;
use std::sync::Arc;

/// Column metadata extracted from Arrow IPC data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
}

/// Detected chart type
#[derive(Debug, Clone, PartialEq)]
enum ChartType {
    Candlestick,
    Line,
    Bar,
    Scatter,
    TableOnly,
}

/// Extract column info from Arrow IPC bytes
pub fn extract_columns(ipc_bytes: &[u8]) -> Vec<ColumnInfo> {
    let schema = match read_schema(ipc_bytes) {
        Some(s) => s,
        None => return vec![],
    };

    schema
        .fields()
        .iter()
        .map(|f| ColumnInfo {
            name: f.name().clone(),
            data_type: format_arrow_type(f.data_type()),
        })
        .collect()
}

/// Auto-detect chart type and generate ECharts option JSON from Arrow IPC bytes
pub fn detect_chart(ipc_bytes: &[u8]) -> Option<Value> {
    if ipc_bytes.is_empty() {
        return None;
    }

    let (schema, data) = read_schema_and_data(ipc_bytes)?;
    let chart_type = detect_chart_type(&schema);

    if chart_type == ChartType::TableOnly {
        return None;
    }

    Some(build_echart_option(&chart_type, &schema, &data))
}

/// Read just the Arrow schema from IPC bytes
fn read_schema(ipc_bytes: &[u8]) -> Option<Arc<Schema>> {
    let cursor = Cursor::new(ipc_bytes);
    let reader = StreamReader::try_new(cursor, None).ok()?;
    Some(reader.schema().clone())
}

/// Read schema and all data from Arrow IPC bytes
fn read_schema_and_data(ipc_bytes: &[u8]) -> Option<(Arc<Schema>, Vec<Vec<Value>>)> {
    let cursor = Cursor::new(ipc_bytes);
    let reader = StreamReader::try_new(cursor, None).ok()?;
    let schema = reader.schema().clone();
    let num_cols = schema.fields().len();

    // Collect all data as JSON arrays per column
    let mut columns: Vec<Vec<Value>> = vec![vec![]; num_cols];

    for batch_result in reader {
        let batch = batch_result.ok()?;
        for col_idx in 0..num_cols {
            let array = batch.column(col_idx);
            for row_idx in 0..batch.num_rows() {
                let val = arrow_value_to_json(array, row_idx);
                columns[col_idx].push(val);
            }
        }
    }

    Some((schema, columns))
}

/// Detect chart type from Arrow schema
fn detect_chart_type(schema: &Schema) -> ChartType {
    let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();

    // Check for OHLC candlestick pattern
    let has_ohlc = ["open", "high", "low", "close"]
        .iter()
        .all(|name| field_names.iter().any(|f| f.eq_ignore_ascii_case(name)));

    if has_ohlc {
        return ChartType::Candlestick;
    }

    // Classify columns
    let mut has_timestamp = false;
    let mut numeric_count = 0;
    let mut string_count = 0;

    for field in schema.fields() {
        match field.data_type() {
            DataType::Timestamp(_, _) | DataType::Date32 | DataType::Date64 => {
                has_timestamp = true;
            }
            DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64 => {
                numeric_count += 1;
            }
            DataType::Utf8 | DataType::LargeUtf8 => {
                string_count += 1;
            }
            _ => {}
        }
    }

    // Timestamp + numeric → line chart
    if has_timestamp && numeric_count >= 1 {
        return ChartType::Line;
    }

    // Categorical (string) + numeric → bar chart
    if string_count >= 1 && numeric_count >= 1 {
        return ChartType::Bar;
    }

    // Two+ numeric columns → scatter
    if numeric_count >= 2 {
        return ChartType::Scatter;
    }

    ChartType::TableOnly
}

/// Build an ECharts option JSON from chart type and data
fn build_echart_option(chart_type: &ChartType, schema: &Schema, columns: &[Vec<Value>]) -> Value {
    match chart_type {
        ChartType::Candlestick => build_candlestick(schema, columns),
        ChartType::Line => build_line(schema, columns),
        ChartType::Bar => build_bar(schema, columns),
        ChartType::Scatter => build_scatter(schema, columns),
        ChartType::TableOnly => json!(null),
    }
}

fn build_candlestick(schema: &Schema, columns: &[Vec<Value>]) -> Value {
    let find_col = |name: &str| -> Option<usize> {
        schema
            .fields()
            .iter()
            .position(|f| f.name().eq_ignore_ascii_case(name))
    };

    let open_idx = find_col("open").unwrap_or(0);
    let close_idx = find_col("close").unwrap_or(1);
    let low_idx = find_col("low").unwrap_or(2);
    let high_idx = find_col("high").unwrap_or(3);

    // Look for a timestamp/date column for x-axis
    let x_idx = schema
        .fields()
        .iter()
        .position(|f| {
            matches!(
                f.data_type(),
                DataType::Timestamp(_, _) | DataType::Date32 | DataType::Date64
            )
        })
        .or_else(|| find_col("timestamp"))
        .or_else(|| find_col("date"));

    let row_count = columns.first().map(|c| c.len()).unwrap_or(0);

    let x_data: Vec<Value> = if let Some(xi) = x_idx {
        columns[xi].clone()
    } else {
        (0..row_count).map(|i| json!(i)).collect()
    };

    // ECharts candlestick format: [open, close, low, high]
    let ohlc_data: Vec<Value> = (0..row_count)
        .map(|i| {
            json!([
                columns[open_idx].get(i).unwrap_or(&json!(0)),
                columns[close_idx].get(i).unwrap_or(&json!(0)),
                columns[low_idx].get(i).unwrap_or(&json!(0)),
                columns[high_idx].get(i).unwrap_or(&json!(0)),
            ])
        })
        .collect();

    json!({
        "xAxis": {
            "type": "category",
            "data": x_data,
            "axisLine": { "lineStyle": { "color": "#8392A5" } }
        },
        "yAxis": {
            "scale": true,
            "splitArea": { "show": true }
        },
        "series": [{
            "type": "candlestick",
            "data": ohlc_data,
            "itemStyle": {
                "color": "#26a69a",
                "color0": "#ef5350",
                "borderColor": "#26a69a",
                "borderColor0": "#ef5350"
            }
        }],
        "tooltip": { "trigger": "axis", "axisPointer": { "type": "cross" } },
        "dataZoom": [
            { "type": "inside", "start": 0, "end": 100 },
            { "type": "slider", "start": 0, "end": 100 }
        ],
        "grid": { "left": "10%", "right": "10%", "bottom": "15%" }
    })
}

fn build_line(schema: &Schema, columns: &[Vec<Value>]) -> Value {
    // Find timestamp column for x-axis
    let x_idx = schema
        .fields()
        .iter()
        .position(|f| {
            matches!(
                f.data_type(),
                DataType::Timestamp(_, _) | DataType::Date32 | DataType::Date64
            )
        })
        .unwrap_or(0);

    let row_count = columns.first().map(|c| c.len()).unwrap_or(0);
    let x_data: Vec<Value> = columns.get(x_idx).cloned().unwrap_or_default();

    // All numeric columns become line series
    let mut series = Vec::new();
    for (i, field) in schema.fields().iter().enumerate() {
        if i == x_idx {
            continue;
        }
        if is_numeric_type(field.data_type()) {
            let data: Vec<Value> = columns.get(i).cloned().unwrap_or_default();
            series.push(json!({
                "name": field.name(),
                "type": "line",
                "data": data,
                "sampling": "lttb",
                "smooth": false,
                "symbol": if row_count > 100 { "none" } else { "circle" },
            }));
        }
    }

    json!({
        "xAxis": {
            "type": "category",
            "data": x_data,
            "axisLine": { "lineStyle": { "color": "#8392A5" } }
        },
        "yAxis": { "type": "value", "scale": true },
        "series": series,
        "tooltip": { "trigger": "axis" },
        "legend": { "show": series.len() > 1 },
        "dataZoom": [
            { "type": "inside", "start": 0, "end": 100 },
            { "type": "slider", "start": 0, "end": 100 }
        ],
        "grid": { "left": "10%", "right": "10%", "bottom": "15%" }
    })
}

fn build_bar(schema: &Schema, columns: &[Vec<Value>]) -> Value {
    // Find string column for categories
    let cat_idx = schema
        .fields()
        .iter()
        .position(|f| matches!(f.data_type(), DataType::Utf8 | DataType::LargeUtf8))
        .unwrap_or(0);

    let categories: Vec<Value> = columns.get(cat_idx).cloned().unwrap_or_default();

    let mut series = Vec::new();
    for (i, field) in schema.fields().iter().enumerate() {
        if i == cat_idx {
            continue;
        }
        if is_numeric_type(field.data_type()) {
            let data: Vec<Value> = columns.get(i).cloned().unwrap_or_default();
            series.push(json!({
                "name": field.name(),
                "type": "bar",
                "data": data,
            }));
        }
    }

    json!({
        "xAxis": { "type": "category", "data": categories },
        "yAxis": { "type": "value" },
        "series": series,
        "tooltip": { "trigger": "axis" },
        "legend": { "show": series.len() > 1 },
        "grid": { "left": "10%", "right": "10%", "bottom": "10%" }
    })
}

fn build_scatter(schema: &Schema, columns: &[Vec<Value>]) -> Value {
    // First two numeric columns become x and y
    let numeric_indices: Vec<usize> = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| is_numeric_type(f.data_type()))
        .map(|(i, _)| i)
        .collect();

    let x_idx = numeric_indices.first().copied().unwrap_or(0);
    let y_idx = numeric_indices.get(1).copied().unwrap_or(1);

    let row_count = columns.first().map(|c| c.len()).unwrap_or(0);
    let scatter_data: Vec<Value> = (0..row_count)
        .map(|i| {
            json!([
                columns
                    .get(x_idx)
                    .and_then(|c| c.get(i))
                    .unwrap_or(&json!(0)),
                columns
                    .get(y_idx)
                    .and_then(|c| c.get(i))
                    .unwrap_or(&json!(0)),
            ])
        })
        .collect();

    let x_name = schema
        .fields()
        .get(x_idx)
        .map(|f| f.name().as_str())
        .unwrap_or("x");
    let y_name = schema
        .fields()
        .get(y_idx)
        .map(|f| f.name().as_str())
        .unwrap_or("y");

    json!({
        "xAxis": { "type": "value", "name": x_name, "scale": true },
        "yAxis": { "type": "value", "name": y_name, "scale": true },
        "series": [{
            "type": "scatter",
            "data": scatter_data,
            "symbolSize": 5,
        }],
        "tooltip": { "trigger": "item" },
        "grid": { "left": "10%", "right": "10%", "bottom": "10%" }
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_numeric_type(dt: &DataType) -> bool {
    matches!(
        dt,
        DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}

fn format_arrow_type(dt: &DataType) -> String {
    match dt {
        DataType::Float32 | DataType::Float64 | DataType::Float16 => "Number".to_string(),
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
            "Integer".to_string()
        }
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            "Integer".to_string()
        }
        DataType::Utf8 | DataType::LargeUtf8 => "String".to_string(),
        DataType::Boolean => "Bool".to_string(),
        DataType::Timestamp(_, _) | DataType::Date32 | DataType::Date64 => "Timestamp".to_string(),
        other => format!("{:?}", other),
    }
}

/// Extract a single value from an Arrow array at the given index as JSON
fn arrow_value_to_json(array: &dyn arrow_array::Array, idx: usize) -> Value {
    use arrow_array::*;

    if array.is_null(idx) {
        return Value::Null;
    }

    if let Some(a) = array.as_any().downcast_ref::<Float64Array>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<Float32Array>() {
        return json!(a.value(idx) as f64);
    }
    if let Some(a) = array.as_any().downcast_ref::<Int64Array>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<Int32Array>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt64Array>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt32Array>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<StringArray>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<BooleanArray>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<TimestampMillisecondArray>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<TimestampMicrosecondArray>() {
        return json!(a.value(idx) / 1000); // Convert to ms
    }
    if let Some(a) = array.as_any().downcast_ref::<TimestampNanosecondArray>() {
        return json!(a.value(idx) / 1_000_000); // Convert to ms
    }
    if let Some(a) = array.as_any().downcast_ref::<Date32Array>() {
        return json!(a.value(idx));
    }
    if let Some(a) = array.as_any().downcast_ref::<Date64Array>() {
        return json!(a.value(idx));
    }

    // Fallback
    json!(null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_chart_type_ohlc() {
        let schema = Schema::new(vec![
            arrow_schema::Field::new(
                "timestamp",
                DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None),
                false,
            ),
            arrow_schema::Field::new("open", DataType::Float64, false),
            arrow_schema::Field::new("high", DataType::Float64, false),
            arrow_schema::Field::new("low", DataType::Float64, false),
            arrow_schema::Field::new("close", DataType::Float64, false),
            arrow_schema::Field::new("volume", DataType::Float64, false),
        ]);
        assert_eq!(detect_chart_type(&schema), ChartType::Candlestick);
    }

    #[test]
    fn test_detect_chart_type_line() {
        let schema = Schema::new(vec![
            arrow_schema::Field::new(
                "time",
                DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None),
                false,
            ),
            arrow_schema::Field::new("value", DataType::Float64, false),
        ]);
        assert_eq!(detect_chart_type(&schema), ChartType::Line);
    }

    #[test]
    fn test_detect_chart_type_bar() {
        let schema = Schema::new(vec![
            arrow_schema::Field::new("category", DataType::Utf8, false),
            arrow_schema::Field::new("count", DataType::Int64, false),
        ]);
        assert_eq!(detect_chart_type(&schema), ChartType::Bar);
    }

    #[test]
    fn test_detect_chart_type_scatter() {
        let schema = Schema::new(vec![
            arrow_schema::Field::new("x", DataType::Float64, false),
            arrow_schema::Field::new("y", DataType::Float64, false),
        ]);
        assert_eq!(detect_chart_type(&schema), ChartType::Scatter);
    }

    #[test]
    fn test_extract_columns_empty() {
        let cols = extract_columns(&[]);
        assert!(cols.is_empty());
    }

    #[test]
    fn test_detect_chart_empty() {
        assert!(detect_chart(&[]).is_none());
    }

    #[test]
    fn test_format_arrow_type() {
        assert_eq!(format_arrow_type(&DataType::Float64), "Number");
        assert_eq!(format_arrow_type(&DataType::Int64), "Integer");
        assert_eq!(format_arrow_type(&DataType::Utf8), "String");
        assert_eq!(format_arrow_type(&DataType::Boolean), "Bool");
    }
}
