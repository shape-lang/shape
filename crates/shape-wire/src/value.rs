//! Wire format value types
//!
//! These types represent the serializable subset of Shape values.
//! Non-serializable runtime constructs (closures, references) are
//! converted to their serializable representations.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Duration unit for time spans
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DurationUnit {
    Nanoseconds,
    Microseconds,
    Milliseconds,
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
}

/// The universal wire format for Shape values
///
/// This enum represents all Shape values in a serializable form.
/// It is the core data structure exchanged between components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireValue {
    /// Null/None value
    Null,

    /// Boolean value
    Bool(bool),

    /// 64-bit floating point number
    Number(f64),

    /// 64-bit signed integer
    Integer(i64),

    /// 8-bit signed integer (ABI-preserving native scalar)
    I8(i8),
    /// 8-bit unsigned integer (ABI-preserving native scalar)
    U8(u8),
    /// 16-bit signed integer (ABI-preserving native scalar)
    I16(i16),
    /// 16-bit unsigned integer (ABI-preserving native scalar)
    U16(u16),
    /// 32-bit signed integer (ABI-preserving native scalar)
    I32(i32),
    /// 32-bit unsigned integer (ABI-preserving native scalar)
    U32(u32),
    /// 64-bit signed integer (ABI-preserving native scalar)
    I64(i64),
    /// 64-bit unsigned integer (ABI-preserving native scalar)
    U64(u64),
    /// Pointer-width signed integer normalized to i64 for portability
    Isize(i64),
    /// Pointer-width unsigned integer normalized to u64 for portability
    Usize(u64),
    /// C pointer value normalized to u64 for portability
    Ptr(u64),
    /// 32-bit float (ABI-preserving native scalar)
    F32(f32),

    /// UTF-8 string
    String(String),

    /// Timestamp as Unix milliseconds (UTC)
    Timestamp(i64),

    /// Duration with unit
    Duration { value: f64, unit: DurationUnit },

    /// Homogeneous array of values
    Array(Vec<WireValue>),

    /// Object with string keys (ordered for deterministic serialization)
    Object(BTreeMap<String, WireValue>),

    /// Table data with Arrow IPC serialization
    Table(WireTable),

    /// Result type (Ok or Err)
    Result { ok: bool, value: Box<WireValue> },

    /// Range value
    Range {
        start: Option<Box<WireValue>>,
        end: Option<Box<WireValue>>,
        inclusive: bool,
    },

    /// Function reference (name only, not callable)
    FunctionRef { name: String },

    /// Print result with structured spans
    PrintResult(crate::print_result::WirePrintResult),

    /// Structured content node for rich rendering
    Content(shape_value::content::ContentNode),
}

/// Wire format for table data
///
/// Stores Arrow IPC bytes for exact schema + data roundtripping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireTable {
    /// Arrow IPC bytes for a single RecordBatch
    pub ipc_bytes: Vec<u8>,
    /// Optional type name (e.g., "Candle", "SensorReading")
    pub type_name: Option<String>,
    /// Optional schema id for typed tables
    pub schema_id: Option<u32>,
    /// Number of rows
    pub row_count: usize,
    /// Number of columns
    pub column_count: usize,
}

/// Column data in a wire series
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireColumn {
    /// Array of f64 values (may contain NaN for missing)
    Numbers(Vec<f64>),

    /// Array of i64 values
    Integers(Vec<i64>),

    /// Array of boolean values
    Booleans(Vec<bool>),

    /// Array of strings
    Strings(Vec<String>),

    /// Array of nested objects
    Objects(Vec<WireValue>),
}

impl WireValue {
    /// Create a null value
    pub fn null() -> Self {
        WireValue::Null
    }

    /// Check if this value is null
    pub fn is_null(&self) -> bool {
        matches!(self, WireValue::Null)
    }

    /// Try to get this value as a number
    pub fn as_number(&self) -> Option<f64> {
        match self {
            WireValue::Number(n) => Some(*n),
            WireValue::Integer(i) => Some(*i as f64),
            WireValue::I8(v) => Some(*v as f64),
            WireValue::U8(v) => Some(*v as f64),
            WireValue::I16(v) => Some(*v as f64),
            WireValue::U16(v) => Some(*v as f64),
            WireValue::I32(v) => Some(*v as f64),
            WireValue::U32(v) => Some(*v as f64),
            // Keep 64-bit/native-width integers exact; callers should use
            // type-specific accessors instead of lossy number coercion.
            WireValue::I64(_)
            | WireValue::U64(_)
            | WireValue::Isize(_)
            | WireValue::Usize(_)
            | WireValue::Ptr(_) => None,
            WireValue::F32(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// Try to get this value as a string
    pub fn as_str(&self) -> Option<&str> {
        match self {
            WireValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get this value as a boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            WireValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get the type name for display purposes
    pub fn type_name(&self) -> &'static str {
        match self {
            WireValue::Null => "Null",
            WireValue::Bool(_) => "Bool",
            WireValue::Number(_) => "Number",
            WireValue::Integer(_) => "Integer",
            WireValue::I8(_) => "i8",
            WireValue::U8(_) => "u8",
            WireValue::I16(_) => "i16",
            WireValue::U16(_) => "u16",
            WireValue::I32(_) => "i32",
            WireValue::U32(_) => "u32",
            WireValue::I64(_) => "i64",
            WireValue::U64(_) => "u64",
            WireValue::Isize(_) => "isize",
            WireValue::Usize(_) => "usize",
            WireValue::Ptr(_) => "ptr",
            WireValue::F32(_) => "f32",
            WireValue::String(_) => "String",
            WireValue::Timestamp(_) => "Timestamp",
            WireValue::Duration { .. } => "Duration",
            WireValue::Array(_) => "Array",
            WireValue::Object(_) => "Object",
            WireValue::Table(_) => "Table",
            WireValue::Result { .. } => "Result",
            WireValue::Range { .. } => "Range",
            WireValue::FunctionRef { .. } => "Function",
            WireValue::PrintResult(_) => "PrintResult",
            WireValue::Content(_) => "Content",
        }
    }
}

impl WireTable {
    /// Create an empty table
    pub fn empty() -> Self {
        WireTable {
            ipc_bytes: Vec::new(),
            type_name: None,
            schema_id: None,
            row_count: 0,
            column_count: 0,
        }
    }
}

impl WireColumn {
    /// Get the number of elements in this column
    pub fn len(&self) -> usize {
        match self {
            WireColumn::Numbers(v) => v.len(),
            WireColumn::Integers(v) => v.len(),
            WireColumn::Booleans(v) => v.len(),
            WireColumn::Strings(v) => v.len(),
            WireColumn::Objects(v) => v.len(),
        }
    }

    /// Check if the column is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the element type name
    pub fn element_type(&self) -> &'static str {
        match self {
            WireColumn::Numbers(_) => "Number",
            WireColumn::Integers(_) => "Integer",
            WireColumn::Booleans(_) => "Bool",
            WireColumn::Strings(_) => "String",
            WireColumn::Objects(_) => "Object",
        }
    }
}

// Conversion from common types
impl From<bool> for WireValue {
    fn from(b: bool) -> Self {
        WireValue::Bool(b)
    }
}

impl From<f64> for WireValue {
    fn from(n: f64) -> Self {
        WireValue::Number(n)
    }
}

impl From<i64> for WireValue {
    fn from(n: i64) -> Self {
        WireValue::Integer(n)
    }
}

impl From<u64> for WireValue {
    fn from(n: u64) -> Self {
        WireValue::U64(n)
    }
}

impl From<i32> for WireValue {
    fn from(n: i32) -> Self {
        WireValue::I32(n)
    }
}

impl From<u32> for WireValue {
    fn from(n: u32) -> Self {
        WireValue::U32(n)
    }
}

impl From<i16> for WireValue {
    fn from(n: i16) -> Self {
        WireValue::I16(n)
    }
}

impl From<u16> for WireValue {
    fn from(n: u16) -> Self {
        WireValue::U16(n)
    }
}

impl From<i8> for WireValue {
    fn from(n: i8) -> Self {
        WireValue::I8(n)
    }
}

impl From<u8> for WireValue {
    fn from(n: u8) -> Self {
        WireValue::U8(n)
    }
}

impl From<f32> for WireValue {
    fn from(n: f32) -> Self {
        WireValue::F32(n)
    }
}

impl From<String> for WireValue {
    fn from(s: String) -> Self {
        WireValue::String(s)
    }
}

impl From<&str> for WireValue {
    fn from(s: &str) -> Self {
        WireValue::String(s.to_string())
    }
}

impl<T: Into<WireValue>> From<Vec<T>> for WireValue {
    fn from(v: Vec<T>) -> Self {
        WireValue::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<WireValue>> From<Option<T>> for WireValue {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => WireValue::Null,
        }
    }
}

/// Convert from serde_json::Value to WireValue
///
/// This allows creating envelopes from JSON values for display purposes.
/// Note: This conversion is lossy - JSON doesn't have type information
/// for things like Timestamp vs Integer, so we use heuristics.
impl From<serde_json::Value> for WireValue {
    fn from(json: serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => WireValue::Null,
            serde_json::Value::Bool(b) => WireValue::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    WireValue::Integer(i)
                } else if let Some(u) = n.as_u64() {
                    WireValue::U64(u)
                } else if let Some(f) = n.as_f64() {
                    WireValue::Number(f)
                } else {
                    WireValue::Null
                }
            }
            serde_json::Value::String(s) => WireValue::String(s),
            serde_json::Value::Array(arr) => {
                WireValue::Array(arr.into_iter().map(WireValue::from).collect())
            }
            serde_json::Value::Object(obj) => {
                // Regular object
                let map: BTreeMap<String, WireValue> = obj
                    .into_iter()
                    .map(|(k, v)| (k, WireValue::from(v)))
                    .collect();
                WireValue::Object(map)
            }
        }
    }
}

impl From<&serde_json::Value> for WireValue {
    fn from(json: &serde_json::Value) -> Self {
        WireValue::from(json.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_value_type_names() {
        assert_eq!(WireValue::Null.type_name(), "Null");
        assert_eq!(WireValue::Bool(true).type_name(), "Bool");
        assert_eq!(WireValue::Number(1.0).type_name(), "Number");
        assert_eq!(WireValue::String("test".into()).type_name(), "String");
    }

    #[test]
    fn test_wire_value_conversions() {
        let v: WireValue = 42.0.into();
        assert_eq!(v.as_number(), Some(42.0));

        let v = WireValue::I64(42);
        assert_eq!(v.as_number(), None, "i64 should not coerce to number");

        let v: WireValue = "hello".into();
        assert_eq!(v.as_str(), Some("hello"));

        let v: WireValue = true.into();
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn test_wire_series_empty() {
        let series = WireTable::empty();
        assert_eq!(series.row_count, 0);
        assert_eq!(series.column_count, 0);
        assert!(series.ipc_bytes.is_empty());
    }

    #[test]
    fn test_json_to_wire_conversion() {
        // Null
        let json = serde_json::json!(null);
        let wire = WireValue::from(json);
        assert!(wire.is_null());

        // Bool
        let json = serde_json::json!(true);
        let wire = WireValue::from(json);
        assert_eq!(wire.as_bool(), Some(true));

        // Integer
        let json = serde_json::json!(42);
        let wire = WireValue::from(json);
        assert!(matches!(wire, WireValue::Integer(42)));

        // Float
        let json = serde_json::json!(3.14);
        let wire = WireValue::from(json);
        assert!(matches!(wire, WireValue::Number(n) if (n - 3.14).abs() < 0.001));

        // String
        let json = serde_json::json!("hello");
        let wire = WireValue::from(json);
        assert_eq!(wire.as_str(), Some("hello"));

        // Array
        let json = serde_json::json!([1, 2, 3]);
        let wire = WireValue::from(json);
        assert!(matches!(wire, WireValue::Array(arr) if arr.len() == 3));

        // Object
        let json = serde_json::json!({"x": 10, "y": 20});
        let wire = WireValue::from(json);
        if let WireValue::Object(map) = wire {
            assert_eq!(map.len(), 2);
        } else {
            panic!("Expected Object");
        }
    }
}
