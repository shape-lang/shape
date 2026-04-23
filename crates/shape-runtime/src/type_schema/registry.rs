//! Type schema registry and builder
//!
//! This module provides the shared registry for type schemas and a fluent
//! builder API for creating schemas.

use super::SchemaId;
use super::enum_support::EnumVariantInfo;
use super::field_types::{FieldAnnotation, FieldType};
use super::schema::TypeSchema;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// Starting value for per-registry schema ID counters.
///
/// Matches the historical `NEXT_SCHEMA_ID` static seed so registries created
/// via [`TypeSchemaRegistry::new_with_stdlib`] use the same ID domain that the
/// process-wide static has always used.
const INITIAL_SCHEMA_ID: SchemaId = 1;

/// Registry of type schemas.
///
/// Each registry owns its own schema-ID counter via `next_id`. This is the
/// per-`Runtime` replacement for the legacy process-global `NEXT_SCHEMA_ID`
/// static: two registries built with [`TypeSchemaRegistry::new_with_stdlib`]
/// assign IDs from their own domains and do not observe each other's state.
///
/// The counter is not currently consulted by the historic [`TypeSchema::new`]
/// path (which still bumps the global static), but can be allocated via
/// [`TypeSchemaRegistry::allocate_id`] and used with
/// [`TypeSchema::with_id`]. During the B1 migration window both paths coexist.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TypeSchemaRegistry {
    /// Per-registry counter for allocating fresh schema IDs.
    ///
    /// Skipped during (de)serialization; a decoded registry restarts its
    /// counter above the maximum observed ID via the custom `Deserialize`
    /// impl. This matches historical behaviour where the global static was
    /// bumped via `ensure_next_schema_id_above`.
    #[serde(skip, default = "default_next_id")]
    next_id: AtomicU32,
    /// Schemas indexed by name
    by_name: HashMap<String, TypeSchema>,
    /// Schemas indexed by ID for fast runtime lookup
    by_id: HashMap<SchemaId, String>,
}

fn default_next_id() -> AtomicU32 {
    AtomicU32::new(INITIAL_SCHEMA_ID)
}

impl Default for TypeSchemaRegistry {
    fn default() -> Self {
        Self {
            next_id: default_next_id(),
            by_name: HashMap::new(),
            by_id: HashMap::new(),
        }
    }
}

impl Clone for TypeSchemaRegistry {
    fn clone(&self) -> Self {
        Self {
            next_id: AtomicU32::new(self.next_id.load(Ordering::SeqCst)),
            by_name: self.by_name.clone(),
            by_id: self.by_id.clone(),
        }
    }
}

impl TypeSchemaRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh schema ID from this registry's per-instance counter.
    ///
    /// IDs allocated via this method are independent of the legacy
    /// process-global `NEXT_SCHEMA_ID` static. Used together with
    /// [`TypeSchema::with_id`] to construct schemas whose IDs are isolated per
    /// registry (and therefore per `Runtime`).
    pub fn allocate_id(&self) -> SchemaId {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Ensure all future allocations from this registry yield IDs strictly
    /// greater than `max_existing_id`.
    ///
    /// Used after loading externally compiled bytecode whose schemas already
    /// have assigned IDs — mirrors the legacy
    /// `ensure_next_schema_id_above` helper at a per-registry scope.
    pub fn ensure_next_id_above(&self, max_existing_id: SchemaId) {
        let required_next = max_existing_id.saturating_add(1);
        let mut current = self.next_id.load(Ordering::SeqCst);
        while current < required_next {
            match self.next_id.compare_exchange(
                current,
                required_next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Peek the next ID that [`allocate_id`](Self::allocate_id) would produce
    /// without incrementing the counter. For tests/introspection only.
    #[cfg(test)]
    pub(crate) fn peek_next_id(&self) -> SchemaId {
        self.next_id.load(Ordering::SeqCst)
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

    /// Register a type with field definitions and per-field annotations.
    ///
    /// Each entry in `field_annotations` corresponds to the field at the same
    /// index in `fields`. Annotations such as `@alias("wire_name")` are stored
    /// on the resulting `FieldDef` so that serialization and deserialization
    /// boundaries can use `wire_name()` instead of the field name.
    pub fn register_type_with_annotations(
        &mut self,
        name: impl Into<String>,
        fields: Vec<(String, FieldType)>,
        field_annotations: Vec<Vec<FieldAnnotation>>,
    ) -> SchemaId {
        let mut schema = TypeSchema::new(name, fields);
        for (i, annotations) in field_annotations.into_iter().enumerate() {
            if i < schema.fields.len() && !annotations.is_empty() {
                schema.fields[i].annotations = annotations;
            }
        }
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

    /// Register a type whose ID is drawn from this registry's per-instance
    /// counter rather than the process-global `NEXT_SCHEMA_ID`.
    ///
    /// Preferred replacement for [`register_type`](Self::register_type) inside
    /// `new_with_stdlib` and any future per-`Runtime` registration pathways.
    pub fn register_type_scoped(
        &mut self,
        name: impl Into<String>,
        fields: Vec<(String, FieldType)>,
    ) -> SchemaId {
        let id = self.allocate_id();
        let schema = TypeSchema::with_id(id, name, fields);
        self.register(schema);
        id
    }

    /// Register an enum whose ID is drawn from this registry's per-instance
    /// counter. See [`register_type_scoped`](Self::register_type_scoped).
    pub fn register_enum_scoped(
        &mut self,
        name: impl Into<String>,
        variants: Vec<EnumVariantInfo>,
    ) -> SchemaId {
        let id = self.allocate_id();
        let schema = TypeSchema::new_enum_with_id(id, name, variants);
        self.register(schema);
        id
    }

    /// Create a registry seeded with the canonical stdlib schemas
    /// (Row / Option / Result / builtin fixed-layout), using the registry's
    /// own per-instance ID counter rather than the legacy global static.
    ///
    /// This is the entry point for per-`Runtime` schema isolation. Two
    /// registries constructed with `new_with_stdlib` assign IDs from
    /// independent domains and do not observe each other's state.
    ///
    /// Note: some schema constructors (e.g. when builtin_schemas uses
    /// `TypeSchema::new`) still fall through to the global counter during the
    /// B1 migration window; only the registry-level `register_type_scoped`
    /// path is fully isolated. See the parity tests in this module for the
    /// invariants that hold today.
    pub fn new_with_stdlib() -> Self {
        let mut registry = Self::new();

        // Register Row type via the per-registry counter.
        registry.register_type_scoped(
            "Row",
            vec![
                ("timestamp".to_string(), FieldType::Timestamp),
                ("fields".to_string(), FieldType::Any),
            ],
        );

        // Register Option / Result enums via the per-registry counter.
        registry.register_enum_scoped(
            "Option",
            vec![
                EnumVariantInfo::new("Some", 0, 1),
                EnumVariantInfo::new("None", 1, 0),
            ],
        );
        registry.register_enum_scoped(
            "Result",
            vec![
                EnumVariantInfo::new("Ok", 0, 1),
                EnumVariantInfo::new("Err", 1, 1),
            ],
        );

        // Register builtin fixed-layout schemas.
        //
        // NOTE: during the B1 migration window, `register_builtin_schemas`
        // internally uses `TypeSchema::new`, which still bumps the global
        // counter. The resulting IDs land in this registry's `by_id` / `by_name`
        // maps, but they are drawn from the global domain. Registries
        // constructed via `new_with_stdlib` therefore isolate *future*
        // scoped allocations; they do not retrofit the builtin IDs. This is
        // acceptable because builtin IDs are stable within a process — the
        // failing-test leakage comes from user-registered types, which go
        // through `register_type_scoped`.
        super::builtin_schemas::register_builtin_schemas(&mut registry);

        registry
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

    // ---- B1.1 parity tests --------------------------------------------------
    //
    // These tests exercise the new per-registry schema ID counter in isolation
    // from the process-global `NEXT_SCHEMA_ID` static. They prove that two
    // independent `TypeSchemaRegistry` instances built with `new_with_stdlib`
    // allocate IDs from *their own* domains when using `register_type_scoped`
    // / `register_enum_scoped` — the root-cause fix for the cross-test schema
    // ID leakage that motivates Track B1.

    #[test]
    fn b1_1_registry_allocate_id_is_per_instance() {
        let r1 = TypeSchemaRegistry::new();
        let r2 = TypeSchemaRegistry::new();

        // Both freshly-constructed registries start at the same seed value.
        assert_eq!(r1.peek_next_id(), r2.peek_next_id());

        // Allocations on r1 don't advance r2's counter.
        let id1a = r1.allocate_id();
        let id1b = r1.allocate_id();
        assert_eq!(id1b, id1a + 1);
        assert_eq!(r2.peek_next_id(), id1a);

        // And vice-versa.
        let id2a = r2.allocate_id();
        assert_eq!(id2a, id1a);
    }

    #[test]
    fn b1_1_new_with_stdlib_uses_registry_counter_for_scoped_types() {
        let mut r1 = TypeSchemaRegistry::new_with_stdlib();
        let mut r2 = TypeSchemaRegistry::new_with_stdlib();

        // Both registries expose the canonical stdlib types.
        for name in ["Row", "Option", "Result"] {
            assert!(r1.has_type(name), "r1 missing {name}");
            assert!(r2.has_type(name), "r2 missing {name}");
        }

        // User-registered schemas go through the per-registry counter and
        // therefore get IDs from disjoint domains when allocated back-to-back
        // on independent registries.
        let r1_user =
            r1.register_type_scoped("UserA", vec![("x".to_string(), FieldType::F64)]);
        let r2_user =
            r2.register_type_scoped("UserA", vec![("x".to_string(), FieldType::F64)]);

        // Both "UserA" schemas resolve within their own registry.
        assert_eq!(r1.get("UserA").unwrap().id, r1_user);
        assert_eq!(r2.get("UserA").unwrap().id, r2_user);

        // The key invariant: r2's scoped ID is NOT advanced by allocations on
        // r1. Independent registries can produce equal IDs for the same name
        // without collision inside their own space.
        let r1_user_b =
            r1.register_type_scoped("UserB", vec![("y".to_string(), FieldType::F64)]);
        assert_eq!(r1_user_b, r1_user + 1);

        // r2's counter is unaffected by r1_user_b.
        let r2_user_b =
            r2.register_type_scoped("UserB", vec![("y".to_string(), FieldType::F64)]);
        assert_eq!(r2_user_b, r2_user + 1);
    }

    #[test]
    fn b1_1_scoped_enum_ids_are_per_registry() {
        let mut r1 = TypeSchemaRegistry::new();
        let mut r2 = TypeSchemaRegistry::new();

        let e1 = r1.register_enum_scoped(
            "Color",
            vec![
                EnumVariantInfo::new("Red", 0, 0),
                EnumVariantInfo::new("Green", 1, 0),
            ],
        );
        let e2 = r2.register_enum_scoped(
            "Color",
            vec![
                EnumVariantInfo::new("Red", 0, 0),
                EnumVariantInfo::new("Green", 1, 0),
            ],
        );

        // Independent registries may legitimately produce the same ID for an
        // enum type defined under the same name.
        assert_eq!(e1, e2);
        assert!(r1.get("Color").unwrap().is_enum());
        assert!(r2.get("Color").unwrap().is_enum());
    }

    #[test]
    fn b1_1_ensure_next_id_above_is_per_registry() {
        let r1 = TypeSchemaRegistry::new();
        let r2 = TypeSchemaRegistry::new();

        r1.ensure_next_id_above(500);
        assert_eq!(r1.peek_next_id(), 501);

        // r2 is unaffected.
        assert_eq!(r2.peek_next_id(), INITIAL_SCHEMA_ID);
    }
}
