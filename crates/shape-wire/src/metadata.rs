//! Wire-format type metadata definitions
//!
//! This module defines the metadata structures that accompany wire TYPES,
//! enabling type-aware display, parsing, and visualization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Kind of type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TypeKind {
    /// Primitive type (Number, String, Bool, etc.)
    Primitive,
    /// Array type
    Array,
    /// Object/struct type
    Object,
    /// Table type
    Table,
    /// Enum type
    Enum,
    /// Union type
    Union,
    /// Function type
    Function,
    /// Result type
    Result,
    /// Optional type
    Optional,
}

/// Type information for a value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeInfo {
    /// Type name (e.g., "Number", "Candle", "Array<Number>")
    pub name: String,

    /// Kind of type
    pub kind: TypeKind,

    /// For object types: field definitions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldInfo>>,

    /// For generic types: type parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic_params: Option<Vec<TypeInfo>>,

    /// For enum types: variant names
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<String>>,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Metadata for the type (formatting, plotting, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TypeMetadata>,
}

/// Information about a field in an object type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Field name
    pub name: String,

    /// Field type
    pub type_info: TypeInfo,

    /// Whether the field is optional
    #[serde(default)]
    pub optional: bool,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Registry of available metadata/formats for a type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeRegistry {
    /// Available metadata items
    pub items: Vec<TypeMetadata>,

    /// Name of the default item
    pub default_item: String,
}

/// Generic metadata for a type
///
/// Stores arbitrary metadata sections (e.g., "plot", "format", "params", "validate").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeMetadata {
    /// Meta name (e.g., "Candle", "Percent", "ISO8601")
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// Arbitrary metadata sections
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sections: HashMap<String, serde_json::Value>,
}

impl TypeInfo {
    /// Create a primitive type info
    pub fn primitive(name: impl Into<String>) -> Self {
        TypeInfo {
            name: name.into(),
            kind: TypeKind::Primitive,
            fields: None,
            generic_params: None,
            variants: None,
            description: None,
            metadata: None,
        }
    }

    /// Create an array type info
    pub fn array(element_type: TypeInfo) -> Self {
        TypeInfo {
            name: format!("Array<{}>", element_type.name),
            kind: TypeKind::Array,
            fields: None,
            generic_params: Some(vec![element_type]),
            variants: None,
            description: None,
            metadata: None,
        }
    }

    /// Create an object type info
    pub fn object(name: impl Into<String>, fields: Vec<FieldInfo>) -> Self {
        TypeInfo {
            name: name.into(),
            kind: TypeKind::Object,
            fields: Some(fields),
            generic_params: None,
            variants: None,
            description: None,
            metadata: None,
        }
    }

    /// Create a table type info (with default Timestamp index)
    pub fn table(element_type: TypeInfo) -> Self {
        TypeInfo {
            name: format!("Table<{}>", element_type.name),
            kind: TypeKind::Table,
            fields: None,
            generic_params: Some(vec![element_type]),
            variants: None,
            description: None,
            metadata: None,
        }
    }

    /// Create a table type info with explicit index type
    pub fn table_with_index(element_type: TypeInfo, index_type: TypeInfo) -> Self {
        TypeInfo {
            name: format!("Table<{}, {}>", element_type.name, index_type.name),
            kind: TypeKind::Table,
            fields: None,
            generic_params: Some(vec![element_type, index_type]),
            variants: None,
            description: None,
            metadata: None,
        }
    }

    /// Add a description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, metadata: TypeMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    // Common primitive types
    pub fn number() -> Self {
        Self::primitive("Number")
    }

    pub fn integer() -> Self {
        Self::primitive("Integer")
    }

    pub fn string() -> Self {
        Self::primitive("String")
    }

    pub fn bool() -> Self {
        Self::primitive("Bool")
    }

    pub fn timestamp() -> Self {
        Self::primitive("Timestamp")
    }

    pub fn null() -> Self {
        Self::primitive("Null")
    }
}

impl FieldInfo {
    /// Create a required field
    pub fn required(name: impl Into<String>, type_info: TypeInfo) -> Self {
        FieldInfo {
            name: name.into(),
            type_info,
            optional: false,
            description: None,
        }
    }

    /// Create an optional field
    pub fn optional(name: impl Into<String>, type_info: TypeInfo) -> Self {
        FieldInfo {
            name: name.into(),
            type_info,
            optional: true,
            description: None,
        }
    }

    /// Add a description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

impl TypeRegistry {
    /// Create an empty registry with a default item
    pub fn new(default_item: impl Into<String>) -> Self {
        TypeRegistry {
            items: Vec::new(),
            default_item: default_item.into(),
        }
    }

    /// Add a metadata item
    pub fn with_item(mut self, item: TypeMetadata) -> Self {
        self.items.push(item);
        self
    }

    /// Create a default registry for basic types (JSON-like display)
    pub fn default_for_primitives() -> Self {
        TypeRegistry {
            items: vec![TypeMetadata::simple("Default", "Standard display format")],
            default_item: "Default".to_string(),
        }
    }

    /// Create a registry for number types
    pub fn for_number() -> Self {
        TypeRegistry {
            items: vec![
                TypeMetadata::simple("Default", "Standard numeric display"),
                TypeMetadata::simple("Fixed", "Fixed decimal places")
                    .with_section("params", serde_json::json!({"decimals": 2})),
                TypeMetadata::simple("Scientific", "Scientific notation"),
                TypeMetadata::simple("Percent", "Percentage format")
                    .with_section("params", serde_json::json!({"decimals": 2})),
                TypeMetadata::simple("Currency", "Currency format")
                    .with_section("params", serde_json::json!({"symbol": "$", "decimals": 2})),
            ],
            default_item: "Default".to_string(),
        }
    }

    /// Create a registry for timestamp types
    pub fn for_timestamp() -> Self {
        TypeRegistry {
            items: vec![
                TypeMetadata::simple("ISO8601", "ISO 8601 format (2024-01-15T10:30:00Z)")
                    .with_section("params", serde_json::json!({"timezone": "UTC"})),
                TypeMetadata::simple("Unix", "Unix timestamp (seconds or milliseconds)")
                    .with_section("params", serde_json::json!({"milliseconds": true})),
                TypeMetadata::simple("Relative", "Relative time (e.g., '2 hours ago')"),
                TypeMetadata::simple("Custom", "Custom strftime pattern"),
            ],
            default_item: "ISO8601".to_string(),
        }
    }
}

impl TypeMetadata {
    /// Create a simple metadata item
    pub fn simple(name: impl Into<String>, description: impl Into<String>) -> Self {
        TypeMetadata {
            name: name.into(),
            description: description.into(),
            sections: HashMap::new(),
        }
    }

    /// Add a metadata section
    pub fn with_section(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.sections.insert(key.into(), value);
        self
    }

    /// Get a metadata section
    pub fn get_section(&self, key: &str) -> Option<&serde_json::Value> {
        self.sections.get(key)
    }

    /// Check if this meta has a specific section
    pub fn has_section(&self, key: &str) -> bool {
        self.sections.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_info_primitive() {
        let t = TypeInfo::number();
        assert_eq!(t.name, "Number");
        assert_eq!(t.kind, TypeKind::Primitive);
    }

    #[test]
    fn test_type_info_with_metadata() {
        let meta = TypeMetadata::simple("Percent", "Percentage format")
            .with_section("format", serde_json::json!("(v) => v * 100 + '%'"));

        let t = TypeInfo::number().with_metadata(meta);
        assert!(t.metadata.is_some());
        assert_eq!(t.metadata.as_ref().unwrap().name, "Percent");
    }

    #[test]
    fn test_type_metadata_serialization() {
        let mut meta = TypeMetadata::simple("Candle", "OHLCV candlestick");
        meta = meta.with_section(
            "plot",
            serde_json::json!({
                "type": "range_bar",
                "mapping": {"start": "open", "max": "high", "min": "low", "end": "close"}
            }),
        );

        let json = serde_json::to_string(&meta).unwrap();
        let parsed: TypeMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "Candle");
        assert!(parsed.has_section("plot"));

        let plot = parsed.get_section("plot").unwrap();
        assert_eq!(plot["type"], "range_bar");
    }

    #[test]
    fn test_type_info_table() {
        let table = TypeInfo::table(TypeInfo::number());
        assert_eq!(table.name, "Table<Number>");
        assert_eq!(table.kind, TypeKind::Table);
        assert_eq!(table.generic_params.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_type_info_table_with_index() {
        let table = TypeInfo::table_with_index(TypeInfo::number(), TypeInfo::integer());
        assert_eq!(table.name, "Table<Number, Integer>");
        assert_eq!(table.kind, TypeKind::Table);
        assert_eq!(table.generic_params.as_ref().unwrap().len(), 2);
        assert_eq!(table.generic_params.as_ref().unwrap()[0].name, "Number");
        assert_eq!(table.generic_params.as_ref().unwrap()[1].name, "Integer");
    }

    #[test]
    fn test_type_info_table_with_string_index() {
        let table = TypeInfo::table_with_index(TypeInfo::number(), TypeInfo::string());
        assert_eq!(table.name, "Table<Number, String>");
        assert_eq!(table.generic_params.as_ref().unwrap()[1].name, "String");
    }
}
