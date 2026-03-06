//! Value envelope - combines value with metadata
//!
//! The envelope is the primary data structure for exchanging Shape values
//! between components. It bundles the raw value with type information and
//! available format options.

use crate::metadata::{TypeInfo, TypeKind, TypeRegistry};
use crate::value::WireValue;
use serde::{Deserialize, Serialize};

/// Complete value envelope with metadata
///
/// This is the primary exchange format for Shape values.
/// It contains:
/// - The raw value (for lossless data transfer)
/// - Type information (for proper interpretation)
/// - Type registry (for metadata and display options)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueEnvelope {
    /// The wire-format value
    pub value: WireValue,

    /// Type information
    pub type_info: TypeInfo,

    /// Available metadata items for display/parsing
    pub type_registry: TypeRegistry,
}

impl ValueEnvelope {
    /// Create a new envelope
    pub fn new(value: WireValue, type_info: TypeInfo, type_registry: TypeRegistry) -> Self {
        ValueEnvelope {
            value,
            type_info,
            type_registry,
        }
    }

    /// Create an envelope with default metadata inferred from the value
    pub fn from_value(value: WireValue) -> Self {
        let (type_info, type_registry) = Self::infer_metadata(&value);
        ValueEnvelope {
            value,
            type_info,
            type_registry,
        }
    }

    /// Infer type and registry metadata from a value
    fn infer_metadata(value: &WireValue) -> (TypeInfo, TypeRegistry) {
        match value {
            WireValue::Null => (TypeInfo::null(), TypeRegistry::default_for_primitives()),

            WireValue::Bool(_) => (TypeInfo::bool(), TypeRegistry::default_for_primitives()),

            WireValue::Number(_) => (TypeInfo::number(), TypeRegistry::for_number()),

            WireValue::Integer(_) => (TypeInfo::integer(), TypeRegistry::for_number()),

            WireValue::I8(_) => (TypeInfo::primitive("i8"), TypeRegistry::for_number()),
            WireValue::U8(_) => (TypeInfo::primitive("u8"), TypeRegistry::for_number()),
            WireValue::I16(_) => (TypeInfo::primitive("i16"), TypeRegistry::for_number()),
            WireValue::U16(_) => (TypeInfo::primitive("u16"), TypeRegistry::for_number()),
            WireValue::I32(_) => (TypeInfo::primitive("i32"), TypeRegistry::for_number()),
            WireValue::U32(_) => (TypeInfo::primitive("u32"), TypeRegistry::for_number()),
            WireValue::I64(_) => (TypeInfo::primitive("i64"), TypeRegistry::for_number()),
            WireValue::U64(_) => (TypeInfo::primitive("u64"), TypeRegistry::for_number()),
            WireValue::Isize(_) => (TypeInfo::primitive("isize"), TypeRegistry::for_number()),
            WireValue::Usize(_) => (TypeInfo::primitive("usize"), TypeRegistry::for_number()),
            WireValue::Ptr(_) => (
                TypeInfo::primitive("ptr"),
                TypeRegistry::default_for_primitives(),
            ),
            WireValue::F32(_) => (TypeInfo::primitive("f32"), TypeRegistry::for_number()),

            WireValue::String(_) => (TypeInfo::string(), TypeRegistry::default_for_primitives()),

            WireValue::Timestamp(_) => (TypeInfo::timestamp(), TypeRegistry::for_timestamp()),

            WireValue::Duration { .. } => (
                TypeInfo::primitive("Duration"),
                TypeRegistry::default_for_primitives(),
            ),

            WireValue::Array(items) => {
                let element_type = if items.is_empty() {
                    TypeInfo::primitive("Unknown")
                } else {
                    Self::infer_metadata(&items[0]).0
                };
                (
                    TypeInfo::array(element_type),
                    TypeRegistry::default_for_primitives(),
                )
            }

            WireValue::Object(fields) => {
                let field_infos: Vec<_> = fields
                    .iter()
                    .map(|(name, v)| {
                        let (field_type, _) = Self::infer_metadata(v);
                        crate::metadata::FieldInfo::required(name, field_type)
                    })
                    .collect();
                (
                    TypeInfo::object("Object", field_infos),
                    TypeRegistry::default_for_primitives(),
                )
            }

            WireValue::Table(series) => {
                let element_type = series
                    .type_name
                    .as_ref()
                    .map(|n| TypeInfo::primitive(n.clone()))
                    .unwrap_or_else(|| TypeInfo::primitive("Row"));
                (
                    TypeInfo::table(element_type),
                    TypeRegistry::default_for_primitives(),
                )
            }

            WireValue::Result { ok, value } => {
                let (inner_type, _) = Self::infer_metadata(value);
                let name = if *ok {
                    format!("Ok<{}>", inner_type.name)
                } else {
                    format!("Err<{}>", inner_type.name)
                };
                (
                    TypeInfo {
                        name,
                        kind: TypeKind::Result,
                        fields: None,
                        generic_params: Some(vec![inner_type]),
                        variants: None,
                        description: None,
                        metadata: None,
                    },
                    TypeRegistry::default_for_primitives(),
                )
            }

            WireValue::Range { .. } => (
                TypeInfo::primitive("Range"),
                TypeRegistry::default_for_primitives(),
            ),

            WireValue::FunctionRef { name } => (
                TypeInfo {
                    name: format!("Function<{}>", name),
                    kind: TypeKind::Function,
                    fields: None,
                    generic_params: None,
                    variants: None,
                    description: None,
                    metadata: None,
                },
                TypeRegistry::default_for_primitives(),
            ),

            WireValue::PrintResult(_) => (
                TypeInfo::primitive("PrintResult"),
                TypeRegistry::default_for_primitives(),
            ),
        }
    }

    /// Get the default format name
    pub fn default_format(&self) -> &str {
        &self.type_registry.default_item
    }

    /// Get available metadata/format names
    pub fn available_formats(&self) -> Vec<&str> {
        self.type_registry
            .items
            .iter()
            .map(|f| f.name.as_str())
            .collect()
    }

    /// Check if a metadata item is available
    pub fn has_format(&self, name: &str) -> bool {
        self.type_registry.items.iter().any(|f| f.name == name)
    }

    /// Format the value using the default format
    pub fn format_default(&self) -> crate::error::Result<String> {
        self.format(
            &self.type_registry.default_item,
            &std::collections::HashMap::new(),
        )
    }

    /// Format the value using a specific metadata item
    pub fn format(
        &self,
        format_name: &str,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<String> {
        crate::formatter::format_value(&self.value, format_name, params)
    }

    /// Format the value with specific metadata and parameters as JSON
    pub fn format_with_json_params(
        &self,
        format_name: &str,
        params: &serde_json::Value,
    ) -> crate::error::Result<String> {
        let params_map: std::collections::HashMap<String, serde_json::Value> = match params {
            serde_json::Value::Object(map) => {
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            }
            _ => std::collections::HashMap::new(),
        };
        self.format(format_name, &params_map)
    }
}

// Convenience constructors for common types
impl ValueEnvelope {
    /// Create a null envelope
    pub fn null() -> Self {
        Self::from_value(WireValue::Null)
    }

    /// Create a number envelope
    pub fn number(n: f64) -> Self {
        Self::from_value(WireValue::Number(n))
    }

    /// Create a string envelope
    pub fn string(s: impl Into<String>) -> Self {
        Self::from_value(WireValue::String(s.into()))
    }

    /// Create a boolean envelope
    pub fn bool(b: bool) -> Self {
        Self::from_value(WireValue::Bool(b))
    }

    /// Create a timestamp envelope
    pub fn timestamp(millis: i64) -> Self {
        Self::from_value(WireValue::Timestamp(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_from_value() {
        let env = ValueEnvelope::from_value(WireValue::Number(42.0));
        assert_eq!(env.type_info.name, "Number");
        assert!(env.has_format("Default"));
        assert!(env.has_format("Fixed"));
    }

    #[test]
    fn test_envelope_timestamp() {
        let env = ValueEnvelope::timestamp(1704067200000);
        assert_eq!(env.type_info.name, "Timestamp");
        assert_eq!(env.default_format(), "ISO8601");
        assert!(env.has_format("Unix"));
        assert!(env.has_format("Relative"));
    }

    #[test]
    fn test_envelope_array() {
        let env = ValueEnvelope::from_value(WireValue::Array(vec![
            WireValue::Number(1.0),
            WireValue::Number(2.0),
        ]));
        assert_eq!(env.type_info.name, "Array<Number>");
    }

    #[test]
    fn test_envelope_convenience() {
        let env = ValueEnvelope::number(3.14);
        assert_eq!(env.value.as_number(), Some(3.14));

        let env = ValueEnvelope::string("hello");
        assert_eq!(env.value.as_str(), Some("hello"));
    }
}
