//! Type schema registry and builder
//!
//! This module provides the shared registry for type schemas and a fluent
//! builder API for creating schemas.

use super::SchemaId;
use super::enum_support::EnumVariantInfo;
use super::field_types::{FieldAnnotation, FieldType};
use super::schema::TypeSchema;
use std::collections::HashMap;

/// Global registry of type schemas
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypeSchemaRegistry {
    /// Schemas indexed by name
    by_name: HashMap<String, TypeSchema>,
    /// Schemas indexed by ID for fast runtime lookup
    by_id: HashMap<SchemaId, String>,
}

impl TypeSchemaRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type schema
    pub fn register(&mut self, schema: TypeSchema) {
        let name = schema.name.clone();
        let id = schema.id;
        self.by_id.insert(id, name.clone());
        self.by_name.insert(name, schema);
    }

    /// Register a type with field definitions
    pub fn register_type(
        &mut self,
        name: impl Into<String>,
        fields: Vec<(String, FieldType)>,
    ) -> SchemaId {
        let schema = TypeSchema::new(name, fields);
        let id = schema.id;
        self.register(schema);
        id
    }

    /// Get schema by name
    pub fn get(&self, name: &str) -> Option<&TypeSchema> {
        self.by_name.get(name)
    }

    /// Get schema by ID
    pub fn get_by_id(&self, id: SchemaId) -> Option<&TypeSchema> {
        self.by_id.get(&id).and_then(|name| self.by_name.get(name))
    }

    /// Highest schema ID currently stored in this registry.
    pub fn max_schema_id(&self) -> Option<SchemaId> {
        self.by_id.keys().copied().max()
    }

    /// Get field offset for a type/field combination
    pub fn field_offset(&self, type_name: &str, field_name: &str) -> Option<usize> {
        self.get(type_name)?.field_offset(field_name)
    }

    /// Check if a type is registered
    pub fn has_type(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Number of registered types
    pub fn type_count(&self) -> usize {
        self.by_name.len()
    }

    /// Iterator over all registered type names
    pub fn type_names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(|s| s.as_str())
    }

    /// Create a registry with common stdlib types pre-registered
    pub fn with_stdlib_types() -> Self {
        let mut registry = Self::new();

        // Register Row type (generic data row)
        registry.register_type(
            "Row",
            vec![
                ("timestamp".to_string(), FieldType::Timestamp),
                ("fields".to_string(), FieldType::Any), // Dynamic fields
            ],
        );

        // Register Option enum type
        registry.register(TypeSchema::new_enum(
            "Option",
            vec![
                EnumVariantInfo::new("Some", 0, 1), // Some(T) has 1 payload field
                EnumVariantInfo::new("None", 1, 0), // None has no payload
            ],
        ));

        // Register Result enum type
        registry.register(TypeSchema::new_enum(
            "Result",
            vec![
                EnumVariantInfo::new("Ok", 0, 1),  // Ok(T) has 1 payload field
                EnumVariantInfo::new("Err", 1, 1), // Err(E) has 1 payload field
            ],
        ));

        // Register builtin fixed-layout schemas (AnyError, TraceFrame, etc.)
        super::builtin_schemas::register_builtin_schemas(&mut registry);

        // Note: Domain-specific types (Candle, Trade, etc.) should be
        // registered by the domain-specific stdlib, not here in core.

        registry
    }

    /// Create a registry with stdlib types and return both registry and builtin IDs.
    pub fn with_stdlib_types_and_builtin_ids() -> (Self, super::builtin_schemas::BuiltinSchemaIds) {
        let mut registry = Self::new();

        // Register Row type
        registry.register_type(
            "Row",
            vec![
                ("timestamp".to_string(), FieldType::Timestamp),
                ("fields".to_string(), FieldType::Any),
            ],
        );

        // Register Option/Result enum types
        registry.register(TypeSchema::new_enum(
            "Option",
            vec![
                EnumVariantInfo::new("Some", 0, 1),
                EnumVariantInfo::new("None", 1, 0),
            ],
        ));
        registry.register(TypeSchema::new_enum(
            "Result",
            vec![
                EnumVariantInfo::new("Ok", 0, 1),
                EnumVariantInfo::new("Err", 1, 1),
            ],
        ));

        // Register builtin schemas and capture IDs
        let ids = super::builtin_schemas::register_builtin_schemas(&mut registry);

        (registry, ids)
    }

    /// Compute content hashes for all registered schemas.
    pub fn compute_all_hashes(&mut self) {
        for schema in self.by_name.values_mut() {
            schema.content_hash();
        }
    }

    /// Look up a schema by its content hash.
    ///
    /// Returns the first schema whose cached or computed content hash matches.
    /// For best performance, call `compute_all_hashes` first.
    pub fn get_by_content_hash(&self, hash: &[u8; 32]) -> Option<&TypeSchema> {
        self.by_name.values().find(|schema| {
            // Use cached hash if available, otherwise compute on the fly
            let schema_hash = match schema.content_hash {
                Some(h) => h,
                None => schema.compute_content_hash(),
            };
            &schema_hash == hash
        })
    }

    /// Merge another registry into this one
    ///
    /// Schemas from `other` are added to this registry. If a schema with the
    /// same name already exists, it is NOT overwritten (first registration wins).
    pub fn merge(&mut self, other: TypeSchemaRegistry) {
        for (name, schema) in other.by_name {
            if !self.by_name.contains_key(&name) {
                let id = schema.id;
                self.by_id.insert(id, name.clone());
                self.by_name.insert(name, schema);
            }
        }
    }
}

impl shape_value::external_value::SchemaLookup for TypeSchemaRegistry {
    fn type_name(&self, schema_id: u64) -> Option<&str> {
        self.get_by_id(schema_id as SchemaId)
            .map(|s| s.name.as_str())
    }

    fn field_names(&self, schema_id: u64) -> Option<Vec<&str>> {
        self.get_by_id(schema_id as SchemaId)
            .map(|s| s.fields.iter().map(|f| f.name.as_str()).collect())
    }
}

/// Builder for creating type schemas fluently
pub struct TypeSchemaBuilder {
    name: String,
    fields: Vec<(String, FieldType)>,
    field_meta: Vec<Vec<FieldAnnotation>>,
}

impl TypeSchemaBuilder {
    /// Start building a new type schema
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
            field_meta: Vec::new(),
        }
    }

    /// Add a f64 field
    pub fn f64_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::F64));
        self.field_meta.push(vec![]);
        self
    }

    /// Add an i64 field
    pub fn i64_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::I64));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a decimal field (stored as f64, reconstructed as Decimal on read)
    pub fn decimal_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::Decimal));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a boolean field
    pub fn bool_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::Bool));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a string field
    pub fn string_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::String));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a timestamp field
    pub fn timestamp_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::Timestamp));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a nested object field
    pub fn object_field(mut self, name: impl Into<String>, type_name: impl Into<String>) -> Self {
        self.fields
            .push((name.into(), FieldType::Object(type_name.into())));
        self.field_meta.push(vec![]);
        self
    }

    /// Add an array field
    pub fn array_field(mut self, name: impl Into<String>, element_type: FieldType) -> Self {
        self.fields
            .push((name.into(), FieldType::Array(Box::new(element_type))));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a dynamic/any field
    pub fn any_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::Any));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a field with annotation metadata
    pub fn field_with_meta(
        mut self,
        name: impl Into<String>,
        field_type: FieldType,
        annotations: Vec<FieldAnnotation>,
    ) -> Self {
        self.fields.push((name.into(), field_type));
        self.field_meta.push(annotations);
        self
    }

    /// Build the type schema
    pub fn build(self) -> TypeSchema {
        let mut schema = TypeSchema::new(self.name, self.fields);
        // Apply annotations to fields
        for (i, annotations) in self.field_meta.into_iter().enumerate() {
            if i < schema.fields.len() {
                schema.fields[i].annotations = annotations;
            }
        }
        schema
    }

    /// Build and register in a registry
    pub fn register(self, registry: &mut TypeSchemaRegistry) -> SchemaId {
        let schema = self.build();
        let id = schema.id;
        registry.register(schema);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry() {
        let mut registry = TypeSchemaRegistry::new();

        let schema_id = registry.register_type(
            "MyType",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
            ],
        );

        assert!(registry.has_type("MyType"));
        assert!(!registry.has_type("OtherType"));

        let schema = registry.get("MyType").unwrap();
        assert_eq!(schema.id, schema_id);
        assert_eq!(schema.field_count(), 2);

        // Test lookup by ID
        let schema_by_id = registry.get_by_id(schema_id).unwrap();
        assert_eq!(schema_by_id.name, "MyType");
    }

    #[test]
    fn test_builder() {
        let mut registry = TypeSchemaRegistry::new();

        let schema_id = TypeSchemaBuilder::new("Point")
            .f64_field("x")
            .f64_field("y")
            .f64_field("z")
            .register(&mut registry);

        let schema = registry.get_by_id(schema_id).unwrap();
        assert_eq!(schema.name, "Point");
        assert_eq!(schema.field_count(), 3);
        assert_eq!(schema.field_offset("x"), Some(0));
        assert_eq!(schema.field_offset("y"), Some(8));
        assert_eq!(schema.field_offset("z"), Some(16));
    }

    #[test]
    fn test_stdlib_types() {
        let registry = TypeSchemaRegistry::with_stdlib_types();

        assert!(registry.has_type("Row"));
        let row_schema = registry.get("Row").unwrap();
        assert!(row_schema.has_field("timestamp"));
    }

    #[test]
    fn test_ohlcv_schema() {
        // Example: registering an OHLCV-like type (would be done by finance stdlib)
        let mut registry = TypeSchemaRegistry::new();

        TypeSchemaBuilder::new("Candle")
            .timestamp_field("timestamp")
            .f64_field("open")
            .f64_field("high")
            .f64_field("low")
            .f64_field("close")
            .f64_field("volume")
            .register(&mut registry);

        let schema = registry.get("Candle").unwrap();
        assert_eq!(schema.field_count(), 6);
        assert_eq!(schema.data_size, 48); // 6 * 8 bytes

        // Check offsets are sequential
        assert_eq!(schema.field_offset("timestamp"), Some(0));
        assert_eq!(schema.field_offset("open"), Some(8));
        assert_eq!(schema.field_offset("high"), Some(16));
        assert_eq!(schema.field_offset("low"), Some(24));
        assert_eq!(schema.field_offset("close"), Some(32));
        assert_eq!(schema.field_offset("volume"), Some(40));
    }

    #[test]
    fn test_stdlib_enum_types() {
        let registry = TypeSchemaRegistry::with_stdlib_types();

        // Check Option is registered
        assert!(registry.has_type("Option"));
        let option_schema = registry.get("Option").unwrap();
        assert!(option_schema.is_enum());
        assert_eq!(option_schema.variant_id("Some"), Some(0));
        assert_eq!(option_schema.variant_id("None"), Some(1));

        // Check Result is registered
        assert!(registry.has_type("Result"));
        let result_schema = registry.get("Result").unwrap();
        assert!(result_schema.is_enum());
        assert_eq!(result_schema.variant_id("Ok"), Some(0));
        assert_eq!(result_schema.variant_id("Err"), Some(1));
    }

    #[test]
    fn test_max_schema_id() {
        let mut registry = TypeSchemaRegistry::new();
        let a = registry.register_type("A", vec![("x".to_string(), FieldType::F64)]);
        let b = registry.register_type("B", vec![("y".to_string(), FieldType::F64)]);
        assert_eq!(registry.max_schema_id(), Some(a.max(b)));
    }
}
