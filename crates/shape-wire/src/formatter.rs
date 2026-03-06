//! Value formatting implementation
//!
//! This module provides basic/fallback formatting for wire values.
//!
//! **Important**: Complex format implementations (Percent, Currency, Scientific, etc.)
//! are now defined in Shape stdlib (stdlib/core/formats.shape) and executed
//! via the Shape runtime. This Rust code provides only:
//!
//! 1. Basic "Default" format for each type (simple display)
//! 2. Fallback for when Shape runtime is not available
//! 3. Wire protocol serialization helpers
//!
//! For rich formatting with custom parameters, use Shape's `.format()` method:
//! ```shape
//! let value = 0.1234;
//! value.format("Percent")           // "12.34%"
//! value.format({ format: "Percent", decimals: 1 })  // "12.3%"
//! ```

use crate::error::{Result, WireError};
use crate::value::WireValue;
use chrono::DateTime;
use std::collections::HashMap;

/// Format a wire value to a string using the specified format
///
/// For basic "Default" format, this provides simple display.
/// For named formats (Percent, Currency, etc.), this provides fallback
/// behavior. Use Shape runtime for full format support.
pub fn format_value(
    value: &WireValue,
    format_name: &str,
    params: &HashMap<String, serde_json::Value>,
) -> Result<String> {
    match value {
        WireValue::Null => Ok("null".to_string()),
        WireValue::Bool(b) => Ok(b.to_string()),
        WireValue::Number(n) => format_number(*n, format_name, params),
        WireValue::Integer(i) => format_integer(*i, format_name, params),
        WireValue::I8(v) => format_integer(*v as i64, format_name, params),
        WireValue::U8(v) => format_unsigned(*v as u64, format_name, params),
        WireValue::I16(v) => format_integer(*v as i64, format_name, params),
        WireValue::U16(v) => format_unsigned(*v as u64, format_name, params),
        WireValue::I32(v) => format_integer(*v as i64, format_name, params),
        WireValue::U32(v) => format_unsigned(*v as u64, format_name, params),
        WireValue::I64(v) => format_integer(*v, format_name, params),
        WireValue::U64(v) => format_unsigned(*v, format_name, params),
        WireValue::Isize(v) => format_integer(*v, format_name, params),
        WireValue::Usize(v) => format_unsigned(*v, format_name, params),
        WireValue::Ptr(v) => {
            if matches!(format_name, "Default" | "") {
                Ok(format!("0x{v:x}"))
            } else {
                format_unsigned(*v, format_name, params)
            }
        }
        WireValue::F32(v) => format_number(*v as f64, format_name, params),
        WireValue::String(s) => Ok(s.clone()),
        WireValue::Timestamp(ts) => format_timestamp(*ts, format_name),
        WireValue::Duration { value, unit } => format_duration(*value, unit),
        WireValue::Array(arr) => format_array(arr),
        WireValue::Object(obj) => format_object(obj),
        WireValue::Table(series) => format_table(series),
        WireValue::Result { ok, value } => format_result(*ok, value),
        WireValue::Range {
            start,
            end,
            inclusive,
        } => format_range(start, end, *inclusive),
        WireValue::FunctionRef { name } => Ok(format!("<function {}>", name)),
        WireValue::PrintResult(result) => Ok(result.rendered.clone()),
    }
}

/// Format a number value (basic fallback)
///
/// For rich formatting (Percent, Currency, etc.), use Shape runtime.
fn format_number(
    n: f64,
    format_name: &str,
    params: &HashMap<String, serde_json::Value>,
) -> Result<String> {
    match format_name {
        // Basic display - smart integer detection
        "Default" | "" => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                Ok(format!("{}", n as i64))
            } else {
                Ok(format!("{}", n))
            }
        }
        // Fallback implementations for common formats
        // (Full support via Shape runtime)
        "Fixed" => {
            let decimals = params.get("decimals").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
            Ok(format!("{:.1$}", n, decimals))
        }
        "Percent" => {
            let decimals = params.get("decimals").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
            Ok(format!("{:.1$}%", n * 100.0, decimals))
        }
        "Currency" => {
            let symbol = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("$");
            let decimals = params.get("decimals").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
            Ok(format!("{}{:.*}", symbol, decimals, n))
        }
        // Unknown format - use default
        _ => Ok(format!("{}", n)),
    }
}

/// Format an integer value (basic fallback)
fn format_integer(
    i: i64,
    format_name: &str,
    params: &HashMap<String, serde_json::Value>,
) -> Result<String> {
    match format_name {
        "Default" | "" => Ok(format!("{}", i)),
        "Hex" => Ok(format!("0x{:x}", i)),
        "Binary" => Ok(format!("0b{:b}", i)),
        "Octal" => Ok(format!("0o{:o}", i)),
        _ => format_number(i as f64, format_name, params),
    }
}

/// Format an unsigned integer value.
fn format_unsigned(
    i: u64,
    format_name: &str,
    params: &HashMap<String, serde_json::Value>,
) -> Result<String> {
    match format_name {
        "Default" | "" => Ok(format!("{i}")),
        "Hex" => Ok(format!("0x{i:x}")),
        "Binary" => Ok(format!("0b{i:b}")),
        "Octal" => Ok(format!("0o{i:o}")),
        _ => format_number(i as f64, format_name, params),
    }
}

/// Format a timestamp value (basic fallback - ISO8601)
fn format_timestamp(ts_millis: i64, format_name: &str) -> Result<String> {
    let dt = DateTime::from_timestamp_millis(ts_millis)
        .ok_or_else(|| WireError::InvalidValue(format!("Invalid timestamp: {}", ts_millis)))?;

    match format_name {
        "Unix" => Ok(format!("{}", ts_millis)),
        // Default to ISO8601 for all other formats
        _ => Ok(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
    }
}

/// Format a duration value
fn format_duration(value: f64, unit: &crate::value::DurationUnit) -> Result<String> {
    use crate::value::DurationUnit;

    let unit_str = match unit {
        DurationUnit::Nanoseconds => "ns",
        DurationUnit::Microseconds => "µs",
        DurationUnit::Milliseconds => "ms",
        DurationUnit::Seconds => "s",
        DurationUnit::Minutes => "min",
        DurationUnit::Hours => "h",
        DurationUnit::Days => "d",
        DurationUnit::Weeks => "w",
    };

    Ok(format!("{}{}", value, unit_str))
}

/// Format an array (basic display)
fn format_array(arr: &[WireValue]) -> Result<String> {
    let formatted: Result<Vec<String>> = arr
        .iter()
        .map(|v| format_value(v, "Default", &HashMap::new()))
        .collect();

    Ok(format!("[{}]", formatted?.join(", ")))
}

/// Format an object (basic display)
fn format_object(obj: &std::collections::BTreeMap<String, WireValue>) -> Result<String> {
    let formatted: Result<Vec<String>> = obj
        .iter()
        .map(|(k, v)| {
            let formatted_val = format_value(v, "Default", &HashMap::new())?;
            Ok(format!("{}: {}", k, formatted_val))
        })
        .collect();

    Ok(format!("{{ {} }}", formatted?.join(", ")))
}

/// Format a series (summary display)
fn format_table(series: &crate::value::WireTable) -> Result<String> {
    let type_name = series.type_name.as_deref().unwrap_or("Table");
    Ok(format!(
        "<{} ({} rows, {} columns)>",
        type_name, series.row_count, series.column_count
    ))
}

/// Format a Result value
fn format_result(ok: bool, value: &WireValue) -> Result<String> {
    let formatted_value = format_value(value, "Default", &HashMap::new())?;
    if ok {
        Ok(format!("Ok({})", formatted_value))
    } else {
        Ok(format!("Err({})", formatted_value))
    }
}

/// Format a Range value
fn format_range(
    start: &Option<Box<WireValue>>,
    end: &Option<Box<WireValue>>,
    inclusive: bool,
) -> Result<String> {
    let start_str = match start {
        Some(v) => format_value(v, "Default", &HashMap::new())?,
        None => "".to_string(),
    };
    let end_str = match end {
        Some(v) => format_value(v, "Default", &HashMap::new())?,
        None => "".to_string(),
    };
    let op = if inclusive { "..=" } else { ".." };
    Ok(format!("{}{}{}", start_str, op, end_str))
}

/// Parse a string into a wire value (basic fallback)
///
/// For rich parsing with format-specific logic, use Shape runtime.
pub fn parse_value(
    text: &str,
    target_type: &str,
    format_name: &str,
    _params: &HashMap<String, serde_json::Value>,
) -> Result<WireValue> {
    match target_type {
        "Number" => parse_number(text, format_name),
        "Integer" => parse_integer(text, format_name),
        "Bool" => parse_bool(text),
        "Timestamp" => parse_timestamp(text),
        "String" => Ok(WireValue::String(text.to_string())),
        _ => Err(WireError::TypeMismatch {
            expected: target_type.to_string(),
            actual: "String".to_string(),
        }),
    }
}

fn parse_number(text: &str, format_name: &str) -> Result<WireValue> {
    // Basic cleanup for common formats
    let cleaned = match format_name {
        "Percent" => text.trim_end_matches('%'),
        "Currency" => {
            text.trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '.')
        }
        _ => text,
    };

    let n: f64 = cleaned
        .parse()
        .map_err(|_| WireError::InvalidValue(format!("Cannot parse '{}' as number", text)))?;

    // Adjust for percent format
    let n = if format_name == "Percent" {
        n / 100.0
    } else {
        n
    };

    Ok(WireValue::Number(n))
}

fn parse_integer(text: &str, format_name: &str) -> Result<WireValue> {
    let i = match format_name {
        "Hex" => {
            let cleaned = text.trim_start_matches("0x").trim_start_matches("0X");
            i64::from_str_radix(cleaned, 16)
        }
        "Binary" => {
            let cleaned = text.trim_start_matches("0b").trim_start_matches("0B");
            i64::from_str_radix(cleaned, 2)
        }
        "Octal" => {
            let cleaned = text.trim_start_matches("0o").trim_start_matches("0O");
            i64::from_str_radix(cleaned, 8)
        }
        _ => text.parse(),
    }
    .map_err(|_| WireError::InvalidValue(format!("Cannot parse '{}' as integer", text)))?;

    Ok(WireValue::Integer(i))
}

fn parse_bool(text: &str) -> Result<WireValue> {
    match text.to_lowercase().as_str() {
        "true" | "yes" | "1" => Ok(WireValue::Bool(true)),
        "false" | "no" | "0" => Ok(WireValue::Bool(false)),
        _ => Err(WireError::InvalidValue(format!(
            "Cannot parse '{}' as boolean",
            text
        ))),
    }
}

fn parse_timestamp(text: &str) -> Result<WireValue> {
    // Try RFC3339/ISO8601 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Ok(WireValue::Timestamp(dt.timestamp_millis()));
    }
    // Try date-only
    if let Ok(nd) = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let dt = nd
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| WireError::InvalidValue("Invalid date".to_string()))?;
        return Ok(WireValue::Timestamp(dt.and_utc().timestamp_millis()));
    }
    // Try unix timestamp
    if let Ok(n) = text.parse::<i64>() {
        return Ok(WireValue::Timestamp(n));
    }
    Err(WireError::InvalidValue(format!(
        "Cannot parse '{}' as timestamp",
        text
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number_default() {
        let result = format_value(&WireValue::Number(42.5), "Default", &HashMap::new()).unwrap();
        assert_eq!(result, "42.5");

        let result = format_value(&WireValue::Number(42.0), "Default", &HashMap::new()).unwrap();
        assert_eq!(result, "42");
    }

    #[test]
    fn test_format_number_fixed() {
        let mut params = HashMap::new();
        params.insert("decimals".to_string(), serde_json::json!(4));

        let result = format_value(&WireValue::Number(3.14159), "Fixed", &params).unwrap();
        assert_eq!(result, "3.1416");
    }

    #[test]
    fn test_format_number_percent() {
        let result = format_value(&WireValue::Number(0.1234), "Percent", &HashMap::new()).unwrap();
        assert_eq!(result, "12.34%");
    }

    #[test]
    fn test_format_number_currency() {
        let mut params = HashMap::new();
        params.insert("symbol".to_string(), serde_json::json!("€"));
        params.insert("decimals".to_string(), serde_json::json!(2));

        let result = format_value(&WireValue::Number(1234.567), "Currency", &params).unwrap();
        assert_eq!(result, "€1234.57");
    }

    #[test]
    fn test_format_timestamp_iso8601() {
        // 2024-01-15T10:30:00Z in milliseconds
        let ts = 1705314600000_i64;
        let result = format_value(&WireValue::Timestamp(ts), "ISO8601", &HashMap::new()).unwrap();
        assert_eq!(result, "2024-01-15T10:30:00Z");
    }

    #[test]
    fn test_format_timestamp_unix() {
        let ts = 1705314600000_i64;

        // Simplified formatter always uses milliseconds
        let result = format_value(&WireValue::Timestamp(ts), "Unix", &HashMap::new()).unwrap();
        assert_eq!(result, "1705314600000");
    }

    #[test]
    fn test_format_timestamp_date_only() {
        // Note: Date-only format now falls back to ISO8601 in simplified formatter.
        // Use Shape runtime for rich format support.
        let ts = 1705314600000_i64;
        let result = format_value(&WireValue::Timestamp(ts), "Date", &HashMap::new()).unwrap();
        // Falls back to ISO8601
        assert_eq!(result, "2024-01-15T10:30:00Z");
    }

    #[test]
    fn test_parse_timestamp_iso8601() {
        let result = parse_value(
            "2024-01-15T10:30:00Z",
            "Timestamp",
            "ISO8601",
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(result, WireValue::Timestamp(1705314600000));
    }

    #[test]
    fn test_parse_number_percent() {
        let result = parse_value("12.34%", "Number", "Percent", &HashMap::new()).unwrap();
        if let WireValue::Number(n) = result {
            assert!((n - 0.1234).abs() < 0.0001);
        } else {
            panic!("Expected Number");
        }
    }

    #[test]
    fn test_format_array() {
        let arr = WireValue::Array(vec![
            WireValue::Number(1.0),
            WireValue::Number(2.0),
            WireValue::Number(3.0),
        ]);
        let result = format_value(&arr, "Default", &HashMap::new()).unwrap();
        assert_eq!(result, "[1, 2, 3]");
    }

    #[test]
    fn test_format_integer_hex() {
        let result = format_value(&WireValue::Integer(255), "Hex", &HashMap::new()).unwrap();
        assert_eq!(result, "0xff");
    }

    #[test]
    fn test_parse_integer_hex() {
        let result = parse_value("0xff", "Integer", "Hex", &HashMap::new()).unwrap();
        assert_eq!(result, WireValue::Integer(255));
    }
}
