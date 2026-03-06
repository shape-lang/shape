//! Core TypeSchema struct and methods
//!
//! This module defines the TypeSchema structure that describes the memory layout
//! of a declared type, with computed field offsets for JIT optimization.

use super::SchemaId;
use super::enum_support::{EnumInfo, EnumVariantInfo};
use super::field_types::{FieldDef, FieldType, semantic_to_field_type};
use arrow_schema::{DataType, Schema as ArrowSchema};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Schema describing the memory layout of a declared type
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypeSchema {
    /// Unique schema identifier
    pub id: SchemaId,
    /// Type name (e.g., "Candle", "Trade")
    pub name: String,
    /// Field definitions with computed offsets
    pub fields: Vec<FieldDef>,
    /// Field lookup by name
    pub(crate) field_map: HashMap<String, usize>,
    /// Total size of the object data in bytes (excluding header)
    pub data_size: usize,
    /// Component types (for intersection types, tracks which types were merged)
    /// Maps field name to the source type name for decomposition
    pub component_types: Option<Vec<String>>,
    /// Maps each field to its source component type (for decomposition)
    pub(crate) field_sources: HashMap<String, String>,
    /// Enum-specific information (if this is an enum type)
    pub enum_info: Option<EnumInfo>,
    /// Content hash (SHA-256) derived from structural definition.
    /// Computed lazily and cached. Skipped during serialization since it is derived.
    #[serde(skip)]
    pub content_hash: Option<[u8; 32]>,
}

impl TypeSchema {
    /// Create a new type schema with the given fields
    pub fn new(name: impl Into<String>, field_defs: Vec<(String, FieldType)>) -> Self {
        let id = super::next_schema_id();
        let name = name.into();

        let mut fields = Vec::with_capacity(field_defs.len());
        let mut field_map = HashMap::with_capacity(field_defs.len());
        let mut offset = 0;

        for (index, (field_name, field_type)) in field_defs.into_iter().enumerate() {
            // Align offset to field's alignment requirement
            let alignment = field_type.alignment();
            offset = (offset + alignment - 1) & !(alignment - 1);

            let field = FieldDef::new(&field_name, field_type.clone(), offset, index as u16);
            field_map.insert(field_name, index);
            offset += field_type.size();
            fields.push(field);
        }

        // Round up total size to 8-byte alignment
        let data_size = (offset + 7) & !7;

        Self {
            id,
            name,
            fields,
            field_map,
            data_size,
            component_types: None,
            field_sources: HashMap::new(),
            enum_info: None,
            content_hash: None,
        }
    }

    /// Get field definition by name
    pub fn get_field(&self, name: &str) -> Option<&FieldDef> {
        self.field_map.get(name).map(|&idx| &self.fields[idx])
    }

    /// Get field offset by name (returns None if field doesn't exist)
    pub fn field_offset(&self, name: &str) -> Option<usize> {
        self.get_field(name).map(|f| f.offset)
    }

    /// Get field index by name
    pub fn field_index(&self, name: &str) -> Option<u16> {
        self.get_field(name).map(|f| f.index)
    }

    /// Get field by index
    pub fn field_by_index(&self, index: u16) -> Option<&FieldDef> {
        self.fields.get(index as usize)
    }

    /// Number of fields in this schema
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Check if schema has a field with the given name
    pub fn has_field(&self, name: &str) -> bool {
        self.field_map.contains_key(name)
    }

    /// Iterator over field names
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|f| f.name.as_str())
    }

    /// Check if this schema is for an enum type
    pub fn is_enum(&self) -> bool {
        self.enum_info.is_some()
    }

    /// Get enum info if this is an enum type
    pub fn get_enum_info(&self) -> Option<&EnumInfo> {
        self.enum_info.as_ref()
    }

    /// Get variant ID by name (for enum types)
    pub fn variant_id(&self, variant_name: &str) -> Option<u16> {
        self.enum_info.as_ref()?.variant_id(variant_name)
    }

    /// Create an enum schema with variant information
    ///
    /// Layout:
    /// - Field 0: __variant (I64) - variant discriminator at offset 0
    /// - Field 1+: __payload_N (Any) - payload fields at offset 8, 16, etc.
    pub fn new_enum(name: impl Into<String>, variants: Vec<EnumVariantInfo>) -> Self {
        let id = super::next_schema_id();
        let name = name.into();
        let enum_info = EnumInfo::new(variants);
        let max_payload = enum_info.max_payload_fields();

        // Build fields: __variant + __payload_0..N
        let mut fields = Vec::with_capacity(1 + max_payload as usize);
        let mut field_map = HashMap::with_capacity(1 + max_payload as usize);

        // Variant discriminator at offset 0
        fields.push(FieldDef::new("__variant", FieldType::I64, 0, 0));
        field_map.insert("__variant".to_string(), 0);

        // Payload fields at offsets 8, 16, etc.
        for i in 0..max_payload {
            let field_name = format!("__payload_{}", i);
            let offset = 8 + (i as usize * 8);
            fields.push(FieldDef::new(&field_name, FieldType::Any, offset, i + 1));
            field_map.insert(field_name, i as usize + 1);
        }

        let data_size = 8 + (max_payload as usize * 8);

        Self {
            id,
            name,
            fields,
            field_map,
            data_size,
            component_types: None,
            field_sources: HashMap::new(),
            enum_info: Some(enum_info),
            content_hash: None,
        }
    }

    /// Compute the content hash (SHA-256) from the structural definition.
    ///
    /// The hash is derived deterministically from:
    /// - The type name
    /// - Fields sorted by name, each contributing field name + field type string
    /// - Enum variant info (if present), sorted by variant name
    ///
    /// For recursive type references (`Object("Foo")`), only the type name is
    /// hashed to avoid infinite recursion.
    pub fn compute_content_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();

        // Hash the type name
        hasher.update(b"name:");
        hasher.update(self.name.as_bytes());

        // Hash fields in deterministic order (sorted by name)
        let mut sorted_fields: Vec<&FieldDef> = self.fields.iter().collect();
        sorted_fields.sort_by(|a, b| a.name.cmp(&b.name));

        hasher.update(b"|fields:");
        for field in &sorted_fields {
            hasher.update(b"(");
            hasher.update(field.name.as_bytes());
            hasher.update(b":");
            hasher.update(field.field_type.to_string().as_bytes());
            hasher.update(b")");
        }

        // Hash enum variant info if present
        if let Some(enum_info) = &self.enum_info {
            let mut sorted_variants: Vec<&super::enum_support::EnumVariantInfo> =
                enum_info.variants.iter().collect();
            sorted_variants.sort_by(|a, b| a.name.cmp(&b.name));

            hasher.update(b"|variants:");
            for variant in &sorted_variants {
                hasher.update(b"(");
                hasher.update(variant.name.as_bytes());
                hasher.update(b":");
                hasher.update(variant.payload_fields.to_string().as_bytes());
                hasher.update(b")");
            }
        }

        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }

    /// Return the cached content hash, computing and caching it if needed.
    pub fn content_hash(&mut self) -> [u8; 32] {
        if let Some(hash) = self.content_hash {
            return hash;
        }
        let hash = self.compute_content_hash();
        self.content_hash = Some(hash);
        hash
    }

    /// Bind this TypeSchema to an Arrow schema, producing a TypeBinding.
    ///
    /// Validates that every field in the TypeSchema has a compatible column in the
    /// Arrow schema. Returns a mapping from TypeSchema field index → Arrow column index.
    pub fn bind_to_arrow_schema(
        &self,
        arrow_schema: &ArrowSchema,
    ) -> Result<TypeBinding, TypeBindingError> {
        let mut field_to_column = Vec::with_capacity(self.fields.len());

        for field in &self.fields {
            // Skip internal enum fields
            if field.name.starts_with("__") {
                field_to_column.push(0); // placeholder
                continue;
            }

            let col_name = field.wire_name();
            let col_idx =
                arrow_schema
                    .index_of(col_name)
                    .map_err(|_| TypeBindingError::MissingColumn {
                        field_name: col_name.to_string(),
                        type_name: self.name.clone(),
                    })?;

            let arrow_field = &arrow_schema.fields()[col_idx];
            if !is_compatible(&field.field_type, arrow_field.data_type()) {
                return Err(TypeBindingError::TypeMismatch {
                    field_name: field.name.clone(),
                    expected: format!("{:?}", field.field_type),
                    actual: format!("{:?}", arrow_field.data_type()),
                });
            }

            field_to_column.push(col_idx);
        }

        Ok(TypeBinding {
            schema_name: self.name.clone(),
            field_to_column,
        })
    }

    /// Create a type schema from a canonical type (for evolved types)
    ///
    /// This converts the semantic CanonicalType representation into a JIT-ready
    /// TypeSchema with proper field offsets and types.
    pub fn from_canonical(canonical: &crate::type_system::environment::CanonicalType) -> Self {
        let id = super::next_schema_id();
        let name = canonical.name.clone();

        let mut fields = Vec::with_capacity(canonical.fields.len());
        let mut field_map = HashMap::with_capacity(canonical.fields.len());

        for (index, cf) in canonical.fields.iter().enumerate() {
            // Convert SemanticType to FieldType
            let field_type = semantic_to_field_type(&cf.field_type, cf.optional);

            let field = FieldDef::new(&cf.name, field_type, cf.offset, index as u16);
            field_map.insert(cf.name.clone(), index);
            fields.push(field);
        }

        Self {
            id,
            name,
            fields,
            field_map,
            data_size: canonical.data_size,
            component_types: None,
            field_sources: HashMap::new(),
            enum_info: None,
            content_hash: None,
        }
    }
}

/// Mapping from TypeSchema field indices to Arrow column indices.
///
/// Used for O(1) field→column resolution when accessing DataTable columns
/// through a typed view.
#[derive(Debug, Clone)]
pub struct TypeBinding {
    /// The type name this binding is for.
    pub schema_name: String,
    /// Maps TypeSchema field index → Arrow column index.
    pub field_to_column: Vec<usize>,
}

impl TypeBinding {
    /// Get the Arrow column index for a given TypeSchema field index.
    pub fn column_index(&self, field_index: usize) -> Option<usize> {
        self.field_to_column.get(field_index).copied()
    }
}

/// Error during type binding to Arrow schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypeBindingError {
    /// Arrow schema is missing a column required by the TypeSchema.
    #[error("Type '{type_name}' requires column '{field_name}' which is not in the DataTable")]
    MissingColumn {
        field_name: String,
        type_name: String,
    },
    /// Arrow column type is incompatible with the TypeSchema field type.
    #[error("Column '{field_name}' has type {actual} but expected {expected}")]
    TypeMismatch {
        field_name: String,
        expected: String,
        actual: String,
    },
}

/// Check if a Shape FieldType is compatible with an Arrow DataType.
fn is_compatible(field_type: &FieldType, arrow_type: &DataType) -> bool {
    match (field_type, arrow_type) {
        (FieldType::F64, DataType::Float64) => true,
        (FieldType::F64, DataType::Float32) => true, // widening is ok
        (FieldType::F64, DataType::Int64) => true,   // numeric promotion
        (FieldType::I64, DataType::Int64) => true,
        (FieldType::I64, DataType::Int32) => true, // widening is ok
        (FieldType::Bool, DataType::Boolean) => true,
        (FieldType::String, DataType::Utf8) => true,
        (FieldType::String, DataType::LargeUtf8) => true,
        (FieldType::Timestamp, DataType::Timestamp(_, _)) => true,
        (FieldType::Timestamp, DataType::Int64) => true, // timestamps are i64 internally
        (FieldType::Decimal, DataType::Float64) => true, // Decimal stored as f64
        (FieldType::Decimal, DataType::Int64) => true,   // numeric promotion
        (FieldType::Any, _) => true,                     // Any matches everything
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_schema_creation() {
        let schema = TypeSchema::new(
            "TestType",
            vec![
                ("a".to_string(), FieldType::F64),
                ("b".to_string(), FieldType::I64),
                ("c".to_string(), FieldType::String),
            ],
        );

        assert_eq!(schema.name, "TestType");
        assert_eq!(schema.field_count(), 3);
        assert_eq!(schema.data_size, 24); // 3 * 8 bytes
    }

    #[test]
    fn test_field_offsets() {
        let schema = TypeSchema::new(
            "OffsetTest",
            vec![
                ("first".to_string(), FieldType::F64),
                ("second".to_string(), FieldType::I64),
                ("third".to_string(), FieldType::Bool),
            ],
        );

        assert_eq!(schema.field_offset("first"), Some(0));
        assert_eq!(schema.field_offset("second"), Some(8));
        assert_eq!(schema.field_offset("third"), Some(16));
        assert_eq!(schema.field_offset("nonexistent"), None);
    }

    #[test]
    fn test_field_index() {
        let schema = TypeSchema::new(
            "IndexTest",
            vec![
                ("a".to_string(), FieldType::F64),
                ("b".to_string(), FieldType::F64),
                ("c".to_string(), FieldType::F64),
            ],
        );

        assert_eq!(schema.field_index("a"), Some(0));
        assert_eq!(schema.field_index("b"), Some(1));
        assert_eq!(schema.field_index("c"), Some(2));
    }

    #[test]
    fn test_unique_schema_ids() {
        let schema1 = TypeSchema::new("Type1", vec![]);
        let schema2 = TypeSchema::new("Type2", vec![]);
        let schema3 = TypeSchema::new("Type3", vec![]);

        // IDs should be unique
        assert_ne!(schema1.id, schema2.id);
        assert_ne!(schema2.id, schema3.id);
        assert_ne!(schema1.id, schema3.id);
    }

    // ==========================================================================
    // Enum Schema Tests
    // ==========================================================================

    #[test]
    fn test_enum_schema_creation() {
        let schema = TypeSchema::new_enum(
            "Option",
            vec![
                EnumVariantInfo::new("Some", 0, 1),
                EnumVariantInfo::new("None", 1, 0),
            ],
        );

        assert_eq!(schema.name, "Option");
        assert!(schema.is_enum());

        // Check variant info
        let enum_info = schema.get_enum_info().unwrap();
        assert_eq!(enum_info.variants.len(), 2);
        assert_eq!(enum_info.variant_id("Some"), Some(0));
        assert_eq!(enum_info.variant_id("None"), Some(1));
        assert_eq!(enum_info.max_payload_fields(), 1);
    }

    #[test]
    fn test_enum_schema_layout() {
        let schema = TypeSchema::new_enum(
            "Result",
            vec![
                EnumVariantInfo::new("Ok", 0, 1),
                EnumVariantInfo::new("Err", 1, 1),
            ],
        );

        // Layout: __variant (8 bytes) + __payload_0 (8 bytes) = 16 bytes
        assert_eq!(schema.data_size, 16);
        assert_eq!(schema.field_count(), 2);

        // Check field offsets
        assert_eq!(schema.field_offset("__variant"), Some(0));
        assert_eq!(schema.field_offset("__payload_0"), Some(8));
    }

    #[test]
    fn test_enum_schema_multiple_payloads() {
        // Enum with variants having different payload counts
        let schema = TypeSchema::new_enum(
            "Shape",
            vec![
                EnumVariantInfo::new("Circle", 0, 1),    // radius only
                EnumVariantInfo::new("Rectangle", 1, 2), // width, height
                EnumVariantInfo::new("Point", 2, 0),     // no payload
            ],
        );

        // Layout should accommodate max payload (2 fields)
        // __variant (8) + __payload_0 (8) + __payload_1 (8) = 24 bytes
        assert_eq!(schema.data_size, 24);
        assert_eq!(schema.field_count(), 3);

        assert_eq!(schema.field_offset("__variant"), Some(0));
        assert_eq!(schema.field_offset("__payload_0"), Some(8));
        assert_eq!(schema.field_offset("__payload_1"), Some(16));
    }

    #[test]
    fn test_enum_variant_lookup() {
        let schema = TypeSchema::new_enum(
            "Status",
            vec![
                EnumVariantInfo::new("Pending", 0, 0),
                EnumVariantInfo::new("Running", 1, 1),
                EnumVariantInfo::new("Complete", 2, 1),
                EnumVariantInfo::new("Failed", 3, 1),
            ],
        );

        let enum_info = schema.get_enum_info().unwrap();

        // Lookup by ID
        let running = enum_info.variant_by_id(1).unwrap();
        assert_eq!(running.name, "Running");
        assert_eq!(running.payload_fields, 1);

        // Lookup by name
        let complete = enum_info.variant_by_name("Complete").unwrap();
        assert_eq!(complete.id, 2);

        // Non-existent variants
        assert!(enum_info.variant_by_id(99).is_none());
        assert!(enum_info.variant_by_name("Unknown").is_none());
    }

    // ==========================================================================
    // TypeBinding Tests
    // ==========================================================================

    #[test]
    fn test_bind_to_arrow_schema_success() {
        use arrow_schema::{Field, Schema as ArrowSchema};

        let type_schema = TypeSchema::new(
            "Candle",
            vec![
                ("open".to_string(), FieldType::F64),
                ("close".to_string(), FieldType::F64),
                ("volume".to_string(), FieldType::I64),
            ],
        );

        let arrow_schema = ArrowSchema::new(vec![
            Field::new("date", DataType::Utf8, false),
            Field::new("open", DataType::Float64, false),
            Field::new("close", DataType::Float64, false),
            Field::new("volume", DataType::Int64, false),
        ]);

        let binding = type_schema.bind_to_arrow_schema(&arrow_schema).unwrap();
        assert_eq!(binding.schema_name, "Candle");
        // "open" is field 0 in TypeSchema, column 1 in Arrow
        assert_eq!(binding.column_index(0), Some(1));
        // "close" is field 1, column 2
        assert_eq!(binding.column_index(1), Some(2));
        // "volume" is field 2, column 3
        assert_eq!(binding.column_index(2), Some(3));
    }

    #[test]
    fn test_bind_missing_column() {
        use arrow_schema::{Field, Schema as ArrowSchema};

        let type_schema = TypeSchema::new(
            "Candle",
            vec![
                ("open".to_string(), FieldType::F64),
                ("missing_field".to_string(), FieldType::F64),
            ],
        );

        let arrow_schema = ArrowSchema::new(vec![Field::new("open", DataType::Float64, false)]);

        let err = type_schema.bind_to_arrow_schema(&arrow_schema).unwrap_err();
        assert!(matches!(err, TypeBindingError::MissingColumn { .. }));
    }

    #[test]
    fn test_bind_type_mismatch() {
        use arrow_schema::{Field, Schema as ArrowSchema};

        let type_schema = TypeSchema::new("Test", vec![("name".to_string(), FieldType::F64)]);

        let arrow_schema = ArrowSchema::new(vec![
            Field::new("name", DataType::Utf8, false), // String, not Float64
        ]);

        let err = type_schema.bind_to_arrow_schema(&arrow_schema).unwrap_err();
        assert!(matches!(err, TypeBindingError::TypeMismatch { .. }));
    }

    #[test]
    fn test_bind_compatible_types() {
        use arrow_schema::{Field, Schema as ArrowSchema, TimeUnit};

        // Test widening and promotion rules
        let type_schema = TypeSchema::new(
            "Wide",
            vec![
                ("f32_as_f64".to_string(), FieldType::F64),
                ("i32_as_i64".to_string(), FieldType::I64),
                ("ts".to_string(), FieldType::Timestamp),
                ("any_field".to_string(), FieldType::Any),
            ],
        );

        let arrow_schema = ArrowSchema::new(vec![
            Field::new("f32_as_f64", DataType::Float32, false),
            Field::new("i32_as_i64", DataType::Int32, false),
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new("any_field", DataType::Boolean, false),
        ]);

        let binding = type_schema.bind_to_arrow_schema(&arrow_schema);
        assert!(binding.is_ok());
    }
}
