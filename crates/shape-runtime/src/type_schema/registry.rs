//! Type schema registry and builder
//!
//! This module provides the shared registry for type schemas and a fluent
//! builder API for creating schemas.

use super::SchemaId;
use super::enum_support::EnumVariantInfo;
use super::field_types::{FieldAnnotation, FieldType};
use super::schema::{SchemaContentId, TypeSchema};
use std::collections::HashMap;
use std::sync::RwLock;
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
    /// Dense per-registry counter that hands out the intern INDEX for each
    /// distinct [`SchemaContentId`] (WF-3A / ADR-006 §2.7.31). It advances
    /// only inside [`Self::intern_content`] (one step per NEW structure) and
    /// via predeclared allocation — never as a blind, order-dependent
    /// identity source.
    ///
    /// Skipped during (de)serialization; a decoded registry rebuilds its
    /// derived `by_content` index and re-seats this counter at the tail via
    /// [`Self::rebuild_content_index`].
    #[serde(skip, default = "default_next_id")]
    next_id: AtomicU32,
    /// Content-derived identity index: `SchemaContentId -> SchemaId` handle.
    ///
    /// The single mint table. `intern_content` returns the existing handle
    /// for a known content id (dedup) or allocates the next dense handle for
    /// a new one. This is a DERIVED index of the canonical content id (the
    /// blessed `StringId`-family interning relationship), not a parallel
    /// discriminator — no code path assigns a handle except `intern_content`.
    /// Skipped during (de)serialization; rebuilt from `by_name` on load.
    #[serde(skip, default)]
    by_content: RwLock<HashMap<SchemaContentId, SchemaId>>,
    /// Schemas indexed by name
    by_name: HashMap<String, TypeSchema>,
    /// Schemas indexed by ID for fast runtime lookup
    by_id: HashMap<SchemaId, String>,
    /// Predeclared schemas keyed by ordered field-name signature.
    ///
    /// Populated by [`Self::register_predeclared_any_schema`] when
    /// compile-time tooling, extensions, or comptime paths derive a
    /// TypedObject layout that is not backed by a named type.
    /// Moved onto the registry in B1.6 (previously a process-global
    /// `PREDECLARED_SCHEMA_CACHE` static).
    #[serde(skip, default)]
    predeclared_cache: RwLock<HashMap<String, SchemaId>>,
    /// Predeclared schemas indexed by schema ID. B1.6 migrated this off
    /// the legacy `PREDECLARED_SCHEMA_REGISTRY` static.
    #[serde(skip, default)]
    predeclared_by_id: RwLock<HashMap<SchemaId, TypeSchema>>,
}

fn default_next_id() -> AtomicU32 {
    AtomicU32::new(INITIAL_SCHEMA_ID)
}

impl Default for TypeSchemaRegistry {
    fn default() -> Self {
        Self {
            next_id: default_next_id(),
            by_content: RwLock::new(HashMap::new()),
            by_name: HashMap::new(),
            by_id: HashMap::new(),
            predeclared_cache: RwLock::new(HashMap::new()),
            predeclared_by_id: RwLock::new(HashMap::new()),
        }
    }
}

impl Clone for TypeSchemaRegistry {
    fn clone(&self) -> Self {
        let by_content = self
            .by_content
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let predeclared_cache = self
            .predeclared_cache
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let predeclared_by_id = self
            .predeclared_by_id
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        Self {
            next_id: AtomicU32::new(self.next_id.load(Ordering::SeqCst)),
            by_content: RwLock::new(by_content),
            by_name: self.by_name.clone(),
            by_id: self.by_id.clone(),
            predeclared_cache: RwLock::new(predeclared_cache),
            predeclared_by_id: RwLock::new(predeclared_by_id),
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

    /// Mint the registry-local `SchemaId` handle for a canonical
    /// [`SchemaContentId`] (WF-3A / ADR-006 §2.7.31) — the SINGLE mint
    /// operation.
    ///
    /// Identical content ids return the SAME handle (structural dedup);
    /// distinct content ids get distinct dense handles. The result is a pure
    /// function of the content id within this registry, independent of
    /// registration order — the property counters could never have. This is
    /// the derived-index relationship `StringId` has to an interned string,
    /// not a second identity.
    pub fn intern_content(&self, content_id: SchemaContentId) -> SchemaId {
        if let Ok(map) = self.by_content.read() {
            if let Some(&handle) = map.get(&content_id) {
                return handle;
            }
        }
        let mut map = self
            .by_content
            .write()
            .expect("type-schema by_content lock poisoned");
        // Re-check under the write lock (another thread may have interned
        // the same content between the read and the write).
        if let Some(&handle) = map.get(&content_id) {
            return handle;
        }
        let handle = self.next_id.fetch_add(1, Ordering::SeqCst);
        map.insert(content_id, handle);
        handle
    }

    /// Keep the dense intern counter strictly ahead of `id`.
    ///
    /// Invariant: `next_id` is always greater than every handle present in
    /// `by_id`, whether that handle was minted by `intern_content` or inserted
    /// via the preserve-path (`register` of a schema carrying an
    /// externally-minted / persisted handle). This guarantees a subsequent
    /// `intern_content` can never hand out a handle that collides with an
    /// already-registered one. This is counter-invariant maintenance, not an
    /// order-dependent collision disambiguator.
    /// Reserve the intern handle space above an externally-minted id range.
    ///
    /// WF-3A: some registries MIRROR schemas from another registry, preserving
    /// those schemas' externally-minted (cross-registry) handles rather than
    /// re-interning them (extension `type_schemas` carry ids baked by the
    /// extension's own registry). Those preserved handles occupy an arbitrary
    /// range; a freshly-interned synthetic schema (`__mod_*`, inline object)
    /// must land ABOVE that range so it can never collide with a mirrored
    /// handle. This reserves the counter past `max_external_id`. This is
    /// external-handle-space reservation (the same shape as rehydrating a
    /// persisted registry's counter), NOT a two-counter collision
    /// disambiguator over one shared keyspace.
    pub fn reserve_handles_above(&self, max_external_id: SchemaId) {
        self.advance_counter_past(max_external_id);
    }

    fn advance_counter_past(&self, id: SchemaId) {
        let required_next = id.saturating_add(1);
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

    /// Rebuild the derived `by_content` index from `by_name` and re-seat the
    /// dense handle counter at the tail (WF-3A).
    ///
    /// This is INDEX REHYDRATION for a registry whose `#[serde(skip)]`
    /// `by_content` table was dropped on (de)serialization or that had
    /// schemas inserted with pre-assigned handles (loaded/persisted
    /// bytecode). It is NOT an order-dependent collision disambiguator: it
    /// re-derives each schema's handle from its persisted content id and
    /// advances the counter past the largest handle so newly-interned
    /// structures never collide with an already-registered handle.
    pub fn rebuild_content_index(&self) {
        let mut max_id = 0;
        if let Ok(mut map) = self.by_content.write() {
            for schema in self.by_name.values() {
                let cid = schema.content_id();
                map.entry(cid).or_insert(schema.id);
                max_id = max_id.max(schema.id);
            }
        }
        // Re-seat the dense counter at the tail so future interns don't reuse
        // an already-registered handle.
        self.advance_counter_past(max_id);
    }

    /// Peek the next ID that [`allocate_id`](Self::allocate_id) would produce
    /// without incrementing the counter. For tests/introspection only.
    #[cfg(test)]
    pub(crate) fn peek_next_id(&self) -> SchemaId {
        self.next_id.load(Ordering::SeqCst)
    }

    /// Insert a schema under its (already-minted) `SchemaId` handle and index
    /// its content id (WF-3A).
    ///
    /// PRESERVES `schema.id`: the fresh-mint registration paths
    /// (`register_type`, `register_type_scoped`, …) mint the handle via
    /// [`Self::intern_content`] first; the rehydration paths (loading
    /// persisted schemas with meaningful handles) preserve those handles and
    /// downstream bytecode operands stay valid. Either way the content id is
    /// indexed so a later intern of the same structure dedups to this handle.
    pub fn register(&mut self, schema: TypeSchema) {
        let name = schema.name.clone();
        let id = schema.id;
        // Index this schema's content -> handle so future interns of the same
        // structure resolve to it. `or_insert`: first registration wins.
        if let Ok(mut map) = self.by_content.write() {
            map.entry(schema.content_id()).or_insert(id);
        }
        // Keep the dense intern counter ahead of this handle (it may be an
        // externally-minted / persisted id inserted via this preserve-path);
        // otherwise a later `intern_content` could mint a colliding handle.
        self.advance_counter_past(id);
        // If this NAME previously mapped to a DIFFERENT handle (e.g. an upsert
        // changed the field set -> new structural identity), drop the stale
        // by_id entry so lookups can't resolve the old handle to the new shape.
        if let Some(prev) = self.by_name.get(&name) {
            if prev.id != id {
                self.by_id.remove(&prev.id);
            }
        }
        self.by_id.insert(id, name.clone());
        self.by_name.insert(name, schema);
    }

    /// Register a type with field definitions
    pub fn register_type(
        &mut self,
        name: impl Into<String>,
        fields: Vec<(String, FieldType)>,
    ) -> SchemaId {
        // WF-3A: mint the handle from THIS registry's content-intern table so
        // inline-object and object-merge schemas that land in one `by_id`
        // keyspace can never collide (the object-spread root cause).
        self.register_type_scoped(name, fields)
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
        let mut schema = TypeSchema::with_id(0, name, fields);
        for (i, annotations) in field_annotations.into_iter().enumerate() {
            if i < schema.fields.len() && !annotations.is_empty() {
                schema.fields[i].annotations = annotations;
            }
        }
        // WF-3A: content-intern from this registry (annotations are not part
        // of structural identity — field name + type are).
        let id = self.intern_content(schema.content_id());
        schema.id = id;
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

    /// Resolve a bare capitalized identifier in pattern position to the enum
    /// that declares it as a **unit** variant.
    ///
    /// A capitalized identifier like `Red` in a `match` arm is syntactically
    /// ambiguous: the grammar parses it as `Pattern::Identifier` (a variable
    /// binder / catch-all). If a registered enum declares `Red` as a unit
    /// variant, the identifier must instead resolve to a refutable variant
    /// pattern. This returns the declaring enum's name when exactly one such
    /// enum exists. Only **unit** variants participate — tuple/struct variants
    /// require a payload at the syntax level and are never bare identifiers.
    ///
    /// Returns `None` when no enum declares `name` as a unit variant, or when
    /// the name is ambiguous across two or more enums (the caller leaves it a
    /// binder rather than guessing).
    pub fn enum_for_unit_variant(&self, name: &str) -> Option<String> {
        let mut found: Option<String> = None;
        for schema in self.by_name.values() {
            let Some(enum_info) = schema.get_enum_info() else {
                continue;
            };
            if let Some(variant) = enum_info.variant_by_name(name) {
                if matches!(variant.kind, crate::type_schema::EnumVariantKind::Unit) {
                    if found.is_some() && found.as_deref() != Some(schema.name.as_str()) {
                        // Ambiguous across multiple enums — do not guess.
                        return None;
                    }
                    found = Some(schema.name.clone());
                }
            }
        }
        found
    }

    /// Collect the names of every enum **unit** variant registered. Used by
    /// the bytecode compiler to seed the MIR lowering layer (which has no
    /// schema-registry access) so a bare `match l { Red => … }` arm resolves
    /// `Red` as a refutable variant pattern rather than a catch-all binder.
    /// Tuple/struct variants are excluded — they require a payload at the
    /// syntax level and are never bare identifiers.
    pub fn unit_variant_names(&self) -> std::collections::HashSet<String> {
        let mut names = std::collections::HashSet::new();
        for schema in self.by_name.values() {
            let Some(enum_info) = schema.get_enum_info() else {
                continue;
            };
            for variant in &enum_info.variants {
                if matches!(variant.kind, crate::type_schema::EnumVariantKind::Unit) {
                    names.insert(variant.name.clone());
                }
            }
        }
        names
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

    /// Create a registry with common stdlib types pre-registered.
    ///
    /// Since B1.7 all registrations draw their IDs from the registry'''s
    /// per-instance counter — no process-global or ambient counter is
    /// consulted. This keeps two independently constructed registries
    /// isolated.
    pub fn with_stdlib_types() -> Self {
        let mut registry = Self::new();

        // Register Row type (generic data row).
        registry.register_type_scoped(
            "Row",
            vec![
                ("timestamp".to_string(), FieldType::Timestamp),
                (
                    "fields".to_string(),
                    crate::type_schema::any_migration::heterogeneous_stdlib_carrier(),
                ), // Dynamic fields
            ],
        );

        // Register Option enum type.
        registry.register_enum_scoped(
            "Option",
            vec![
                EnumVariantInfo::new("Some", 0, 1), // Some(T) has 1 payload field
                EnumVariantInfo::new("None", 1, 0), // None has no payload
            ],
        );

        // Register Result enum type.
        registry.register_enum_scoped(
            "Result",
            vec![
                EnumVariantInfo::new("Ok", 0, 1),  // Ok(T) has 1 payload field
                EnumVariantInfo::new("Err", 1, 1), // Err(E) has 1 payload field
            ],
        );

        // Register builtin fixed-layout schemas (AnyError, TraceFrame, etc.).
        super::builtin_schemas::register_builtin_schemas(&mut registry);

        // Note: Domain-specific types (Candle, Trade, etc.) should be
        // registered by the domain-specific stdlib, not here in core.

        registry
    }

    /// Create a registry with stdlib types and return both registry and builtin IDs.
    ///
    /// Since B1.7 all registrations draw their IDs from the registry'''s
    /// per-instance counter — no process-global or ambient counter is
    /// consulted.
    pub fn with_stdlib_types_and_builtin_ids() -> (Self, super::builtin_schemas::BuiltinSchemaIds) {
        let mut registry = Self::new();

        // Register Row type.
        registry.register_type_scoped(
            "Row",
            vec![
                ("timestamp".to_string(), FieldType::Timestamp),
                (
                    "fields".to_string(),
                    crate::type_schema::any_migration::heterogeneous_stdlib_carrier(),
                ),
            ],
        );

        // Register Option/Result enum types.
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

        // Register builtin schemas and capture IDs.
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
        let mut schema = TypeSchema::with_id(0, name, fields);
        // WF-3A: single mint point — the content-intern table of THIS registry.
        let id = self.intern_content(schema.content_id());
        schema.id = id;
        self.register(schema);
        id
    }

    /// Register or refresh a named type using this registry's local ID domain.
    ///
    /// If the schema already exists and contains every requested field, this is
    /// a no-op. If requested fields are missing, the schema is rebuilt with the
    /// same schema ID and the union of existing + requested fields. This is
    /// used for synthetic module-object schemas whose export set may be
    /// observed in more than one compiler phase.
    pub fn upsert_type_scoped_union_fields(
        &mut self,
        name: impl Into<String>,
        fields: Vec<(String, FieldType)>,
    ) -> SchemaId {
        let name = name.into();
        if let Some(existing) = self.by_name.get(&name) {
            let id = existing.id;
            let mut merged: Vec<(String, FieldType)> = existing
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.field_type.clone()))
                .collect();
            let mut changed = false;
            for (field_name, field_type) in fields {
                if merged.iter().any(|(name, _)| name == &field_name) {
                    continue;
                }
                merged.push((field_name, field_type));
                changed = true;
            }
            if changed {
                // Grow the schema IN PLACE under its STABLE handle (WF-3A).
                // Module-object (`__mod_*`) schemas are progressively
                // discovered across compiler phases; a bound module-namespace
                // type caches this handle, so re-minting on each field-set
                // growth would strand those bindings ("module has no export
                // 'X'"). `register` also indexes the grown structure's content
                // id -> this handle, so a later intern of the full shape dedups
                // here rather than minting a rival handle.
                let schema = TypeSchema::with_id(id, name, merged);
                self.register(schema);
            }
            return id;
        }

        self.register_type_scoped(name, fields)
    }

    /// Register an enum whose ID is drawn from this registry's per-instance
    /// counter. See [`register_type_scoped`](Self::register_type_scoped).
    pub fn register_enum_scoped(
        &mut self,
        name: impl Into<String>,
        variants: Vec<EnumVariantInfo>,
    ) -> SchemaId {
        let mut schema = TypeSchema::new_enum_with_id(0, name, variants);
        // WF-3A: single mint point — the content-intern table of THIS registry.
        let id = self.intern_content(schema.content_id());
        schema.id = id;
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
                (
                    "fields".to_string(),
                    crate::type_schema::any_migration::heterogeneous_stdlib_carrier(),
                ),
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

    /// Merge another registry into this one.
    ///
    /// Schemas from `other` are added to this registry. If a schema with the
    /// same name already exists, it is NOT overwritten (first registration
    /// wins), and the returned remap points the incoming ID at the existing
    /// ID. Otherwise the incoming schema's handle is re-derived by interning
    /// its content id into THIS registry (WF-3A): identical structure dedups
    /// to an existing handle, distinct structure gets a fresh dense handle.
    ///
    /// The returned map is old incoming ID -> final merged ID. Callers that
    /// carry schema IDs outside the registry, such as bytecode operands, must
    /// apply it to those carriers after merging (legitimate cross-registry
    /// handle translation — `other`'s registry-local handles become `self`'s;
    /// this is NOT a collision disambiguator, and there is no counter-bump /
    /// reallocation loop).
    pub fn merge(&mut self, other: TypeSchemaRegistry) -> HashMap<SchemaId, SchemaId> {
        let mut id_remap = HashMap::new();

        for (name, mut schema) in other.by_name {
            if let Some(existing) = self.by_name.get(&name) {
                if existing.id != schema.id {
                    id_remap.insert(schema.id, existing.id);
                }
                continue;
            }
            let original_id = schema.id;
            let new_id = self.intern_content(schema.content_id());
            if new_id != original_id {
                id_remap.insert(original_id, new_id);
            }
            schema.id = new_id;
            self.by_id.insert(new_id, name.clone());
            self.by_name.insert(name, schema);
        }
        // Also merge predeclared schemas, first-registration-wins on ID collision.
        if let (Ok(other_by_id), Ok(mut self_by_id)) = (
            other.predeclared_by_id.read(),
            self.predeclared_by_id.write(),
        ) {
            for (id, schema) in other_by_id.iter() {
                self_by_id.entry(*id).or_insert_with(|| schema.clone());
            }
        }
        if let (Ok(other_cache), Ok(mut self_cache)) = (
            other.predeclared_cache.read(),
            self.predeclared_cache.write(),
        ) {
            for (key, id) in other_cache.iter() {
                self_cache.entry(key.clone()).or_insert(*id);
            }
        }
        // Re-seat the dense counter past any predeclared handle merged in
        // above (predeclared allocation shares this counter). Index
        // rehydration, not a collision disambiguator.
        self.rebuild_content_index();
        id_remap
    }

    // -- Predeclared schema support (moved off process-global statics in B1.6) ---

    /// Build the canonical field-signature key used by the predeclared
    /// schema cache.
    fn predeclared_cache_key(fields: &[&str]) -> String {
        fields.join("\u{1f}")
    }

    /// Cache key for a typed predeclared schema.
    ///
    /// #235 stage 1: the untyped key above is field NAMES only, which was
    /// sound while every predeclared schema was all-`Any` (same names implied
    /// same schema). Once the columns carry real types, two schemas can share
    /// a field-name list and differ in type, so the key has to include the
    /// types or the first registration would be handed back to the second.
    fn predeclared_typed_cache_key(fields: &[(String, FieldType)]) -> String {
        fields
            .iter()
            .map(|(name, ft)| format!("{}\u{1e}{}", name, ft))
            .collect::<Vec<_>>()
            .join("\u{1f}")
    }

    /// Register (or retrieve) a predeclared schema whose columns carry real
    /// types.
    ///
    /// #235 stage 1 (ADR-020 grill R-G4). The `Any`-column sibling below
    /// exists only for callers that have not been converted yet; a caller that
    /// knows its column types — and the data-driven ones do, they build a
    /// `NativeKind` track for the very same fields — registers here instead.
    pub fn register_predeclared_typed_schema(&self, fields: &[(String, FieldType)]) -> SchemaId {
        let key = Self::predeclared_typed_cache_key(fields);
        if let Ok(cache) = self.predeclared_cache.read() {
            if let Some(id) = cache.get(&key) {
                return *id;
            }
        }
        let id = self.allocate_id();
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        let schema = TypeSchema::with_id(
            id,
            format!("__predecl_{}", names.join("_")),
            fields.to_vec(),
        );
        if let Ok(mut reg) = self.predeclared_by_id.write() {
            reg.insert(id, schema);
        }
        if let Ok(mut cache) = self.predeclared_cache.write() {
            cache.insert(key, id);
        }
        id
    }

    /// Register (or retrieve) a predeclared schema with `FieldType::Any`
    /// columns for the given ordered field set.
    ///
    /// Intended for compile-time schema derivation paths (extensions,
    /// comptime, printing helpers) that need runtime object construction
    /// without a user-declared type. Repeated calls with identical field
    /// names return the same cached ID.
    pub fn register_predeclared_any_schema(&self, fields: &[String]) -> SchemaId {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        let key = Self::predeclared_cache_key(&field_refs);

        if let Ok(cache) = self.predeclared_cache.read() {
            if let Some(id) = cache.get(&key) {
                return *id;
            }
        }

        let typed_fields: Vec<(String, FieldType)> = fields
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    crate::type_schema::any_migration::class_f_runtime_synthesis(),
                )
            })
            .collect();

        let id = self.allocate_id();
        let schema =
            TypeSchema::with_id(id, format!("__predecl_{}", fields.join("_")), typed_fields);

        if let Ok(mut reg) = self.predeclared_by_id.write() {
            reg.insert(id, schema);
        }
        if let Ok(mut cache) = self.predeclared_cache.write() {
            cache.insert(key, id);
        }
        id
    }

    /// Look up a predeclared schema by ID.
    pub fn lookup_predeclared_by_id(&self, id: SchemaId) -> Option<TypeSchema> {
        self.predeclared_by_id
            .read()
            .ok()
            .and_then(|reg| reg.get(&id).cloned())
    }

    /// Mirror a predeclared schema with a caller-supplied ID.
    ///
    /// Used during the B1 migration window by
    /// [`super::register_predeclared_any_schema`] so a single SchemaId
    /// owned by the process-wide fallback registry is also visible
    /// through the per-Runtime ambient registry. Idempotent: a second
    /// call with the same ID is a no-op.
    pub fn mirror_predeclared_any_schema(&self, fields: &[String], id: SchemaId) {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        let key = Self::predeclared_cache_key(&field_refs);

        if let Ok(cache) = self.predeclared_cache.read() {
            if cache.get(&key).copied() == Some(id) {
                return;
            }
        }

        let typed_fields: Vec<(String, FieldType)> = fields
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    crate::type_schema::any_migration::class_f_runtime_synthesis(),
                )
            })
            .collect();

        let schema =
            TypeSchema::with_id(id, format!("__predecl_{}", fields.join("_")), typed_fields);

        if let Ok(mut reg) = self.predeclared_by_id.write() {
            reg.entry(id).or_insert(schema);
        }
        if let Ok(mut cache) = self.predeclared_cache.write() {
            cache.entry(key).or_insert(id);
        }
    }

    /// Look up a predeclared schema ID by an ordered field signature (fast
    /// path).
    pub fn lookup_predeclared_id_by_field_order(&self, fields: &[&str]) -> Option<SchemaId> {
        let key = Self::predeclared_cache_key(fields);
        self.predeclared_cache
            .read()
            .ok()
            .and_then(|cache| cache.get(&key).copied())
    }

    /// Order-insensitive predeclared schema lookup by field set.
    pub fn lookup_predeclared_by_field_set(&self, fields: &[&str]) -> Option<TypeSchema> {
        let Ok(reg) = self.predeclared_by_id.read() else {
            return None;
        };
        reg.values()
            .find(|schema| {
                if schema.fields.len() != fields.len() {
                    return false;
                }
                let wanted: std::collections::HashSet<&str> = fields.iter().copied().collect();
                schema
                    .fields
                    .iter()
                    .all(|f| wanted.contains(f.name.as_str()))
            })
            .cloned()
    }
}

// `shape_value::external_value::SchemaLookup` was deleted alongside the
// rest of the external-value adapter layer (Phase 2b — see
// `docs/defections.md` 2026-05-06). The trait's role was to let
// `shape_value` look up schema metadata without depending on
// `shape_runtime`; with `external_value` removed, callers route
// through the runtime's `current_registry()` directly. This `impl`
// block becomes a no-op and is omitted entirely.

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

    /// Add an `Option<string>` field (`string?`).
    ///
    /// The slot's per-value kind (`Null` for `None`, `String` for
    /// `Some(s)`) lives in the parallel `field_kinds` track at storage
    /// construction time per ADR-006 §2.7.7 / Q9 + §2.7.26; the schema
    /// FieldType is the compile-time stamp (`FIELD_TAG_OPTION`) that
    /// routes the field read through the carrier-authoritative
    /// `field_kinds` lookup. Backs the comptime introspection contract's
    /// nullable descriptor fields (`__ComptimeTarget.return_type` /
    /// `.doc`, comptime-excellence §4.3).
    pub fn option_string_field(mut self, name: impl Into<String>) -> Self {
        self.fields
            .push((name.into(), FieldType::Option(Box::new(FieldType::String))));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a 64-bit integer field
    pub fn int_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((name.into(), FieldType::I64));
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

    /// Add a HashMap<K, V> field. W17.3-4.1 — per audit §4.B builder
    /// parity with `array_field`. Slot storage points to
    /// `HeapKind::HashMap`; the schema-side variant carries the static
    /// K/V FieldTypes for compile-time checking (ADR-006 §2.7.5).
    pub fn hashmap_field(
        mut self,
        name: impl Into<String>,
        key_type: FieldType,
        value_type: FieldType,
    ) -> Self {
        self.fields.push((
            name.into(),
            FieldType::HashMap {
                key: Box::new(key_type),
                value: Box::new(value_type),
            },
        ));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a Set<T> field. W17.3-4.1 — per audit §4.B builder parity
    /// with `array_field`. Slot storage points to `HeapKind::HashSet`;
    /// the schema-side variant carries the static element FieldType
    /// for compile-time checking (ADR-006 §2.7.5).
    pub fn set_field(mut self, name: impl Into<String>, element_type: FieldType) -> Self {
        self.fields
            .push((name.into(), FieldType::Set(Box::new(element_type))));
        self.field_meta.push(vec![]);
        self
    }

    /// Add a dynamic/any field
    pub fn any_field(mut self, name: impl Into<String>) -> Self {
        self.fields.push((
            name.into(),
            crate::type_schema::any_migration::heterogeneous_stdlib_carrier(),
        ));
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

    /// Build and register in a registry, minting the handle from the target
    /// registry's content-intern table (WF-3A).
    ///
    /// This path must not consult `current_registry`, because
    /// `DEFAULT_SCHEMA_REGISTRY` is itself initialized via this builder and
    /// that would cause a recursive `LazyLock` init. Interning directly from
    /// the target registry keeps bootstrap deterministic and per-registry
    /// isolated.
    pub fn register(self, registry: &mut TypeSchemaRegistry) -> SchemaId {
        let mut schema = TypeSchema::with_id(0, self.name, self.fields);
        let id = registry.intern_content(schema.content_id());
        schema.id = id;
        for (i, annotations) in self.field_meta.into_iter().enumerate() {
            if i < schema.fields.len() {
                schema.fields[i].annotations = annotations;
            }
        }
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
        let r1_user = r1.register_type_scoped("UserA", vec![("x".to_string(), FieldType::F64)]);
        let r2_user = r2.register_type_scoped("UserA", vec![("x".to_string(), FieldType::F64)]);

        // Both "UserA" schemas resolve within their own registry.
        assert_eq!(r1.get("UserA").unwrap().id, r1_user);
        assert_eq!(r2.get("UserA").unwrap().id, r2_user);

        // The key invariant: r2's scoped ID is NOT advanced by allocations on
        // r1. Independent registries can produce equal IDs for the same name
        // without collision inside their own space.
        let r1_user_b = r1.register_type_scoped("UserB", vec![("y".to_string(), FieldType::F64)]);
        assert_eq!(r1_user_b, r1_user + 1);

        // r2's counter is unaffected by r1_user_b.
        let r2_user_b = r2.register_type_scoped("UserB", vec![("y".to_string(), FieldType::F64)]);
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

    /// WF-3A: content-interning is the single mint. Identical structure ->
    /// identical handle (dedup); distinct structure -> distinct handle;
    /// independent of registration order.
    #[test]
    fn wf3a_intern_content_dedups_by_structure() {
        let mut r = TypeSchemaRegistry::new();

        // Two anonymous inline objects with identical structure share a handle.
        let a = r.register_type_scoped(
            "__inline_obj_1",
            vec![
                ("x".to_string(), FieldType::I64),
                ("y".to_string(), FieldType::I64),
            ],
        );
        let b = r.register_type_scoped(
            "__inline_obj_2",
            vec![
                ("x".to_string(), FieldType::I64),
                ("y".to_string(), FieldType::I64),
            ],
        );
        assert_eq!(a, b, "structurally identical anonymous schemas dedup");

        // A distinct structure gets a distinct handle.
        let c = r.register_type_scoped("__inline_obj_3", vec![("z".to_string(), FieldType::I64)]);
        assert_ne!(a, c);

        // Named/branded types are nominal: same fields, different name ->
        // distinct handles.
        let named_a = r.register_type_scoped(
            "A",
            vec![
                ("x".to_string(), FieldType::I64),
                ("y".to_string(), FieldType::I64),
            ],
        );
        let named_b = r.register_type_scoped(
            "B",
            vec![
                ("x".to_string(), FieldType::I64),
                ("y".to_string(), FieldType::I64),
            ],
        );
        assert_ne!(named_a, named_b, "named types are nominally distinct");
        // ... and both distinct from the structurally-identical anonymous one.
        assert_ne!(named_a, a);
    }

    /// WF-3A ratified identity model: NAMED/branded types are NOMINAL (name in
    /// the hash) while ANONYMOUS object literals/merges are STRUCTURAL (name
    /// excluded). Field order is layout-significant in both.
    #[test]
    fn wf3a_named_nominal_vs_anonymous_structural_identity() {
        let mut r = TypeSchemaRegistry::new();
        let xy = vec![
            ("x".to_string(), FieldType::I64),
            ("y".to_string(), FieldType::I64),
        ];
        let yx = vec![
            ("y".to_string(), FieldType::I64),
            ("x".to_string(), FieldType::I64),
        ];

        // NAMED nominal: same fields, different user name -> DISTINCT handles.
        let named_a = r.register_type_scoped("A", xy.clone());
        let named_b = r.register_type_scoped("B", xy.clone());
        assert_ne!(
            named_a, named_b,
            "type A {{x,y}} != type B {{x,y}} (nominal)"
        );

        // ANONYMOUS structural: two distinct anon names, same fields -> SAME handle.
        let anon_1 = r.register_type_scoped("__inline_obj_10", xy.clone());
        let anon_2 = r.register_type_scoped("__inline_obj_11", xy.clone());
        assert_eq!(
            anon_1, anon_2,
            "two anonymous {{x,y}} intern equal (structural)"
        );
        // ...and distinct from either named handle (name in the branded hash).
        assert_ne!(anon_1, named_a);
        assert_ne!(anon_1, named_b);

        // Field ORDER is layout-significant: {x,y} != {y,x} for anonymous too.
        let anon_yx = r.register_type_scoped("__inline_obj_12", yx);
        assert_ne!(
            anon_1, anon_yx,
            "{{x,y}} != {{y,x}} (declaration-order layout)"
        );

        // The u32 handle resolves back to the correct arity via get_by_id.
        assert_eq!(r.get_by_id(anon_1).unwrap().field_count(), 2);
        assert_eq!(r.get_by_id(anon_yx).unwrap().field_count(), 2);
    }

    /// WF-3A registration-order-shuffle regression: the exact Wave-2 trigger.
    /// Registering an unrelated `Snapshot` enum first (advancing the counter)
    /// used to shift the merged/inline object-spread ids onto a colliding
    /// 1-field handle. Under content-derived identity the inline `[z]` and the
    /// merged `[x,y,z]` schemas stay DISTINCT handles in BOTH orders, arity is
    /// correct in both, and the anonymous `[x,y,z]` content id is identical
    /// across registries despite differing handle values.
    #[test]
    fn wf3a_registration_order_shuffle_no_collision() {
        let xy = vec![
            ("x".to_string(), FieldType::I64),
            ("y".to_string(), FieldType::I64),
        ];
        let z = vec![("z".to_string(), FieldType::I64)];
        let xyz = vec![
            ("x".to_string(), FieldType::I64),
            ("y".to_string(), FieldType::I64),
            ("z".to_string(), FieldType::I64),
        ];

        // Register the object-spread schema set (mirrors compile_dynamic_object:
        // base [x,y], empty [], merged [x,y], inline [z], merged [x,y,z]).
        fn register_spread_set(
            r: &mut TypeSchemaRegistry,
            xy: &[(String, FieldType)],
            z: &[(String, FieldType)],
            xyz: &[(String, FieldType)],
        ) -> (SchemaId, SchemaId, SchemaId) {
            let base = r.register_type_scoped("__inline_obj_1", xy.to_vec());
            let _empty = r.register_type_scoped("__inline_obj_2", vec![]);
            let _merged_xy = r.register_type_scoped("__merged_1_2", xy.to_vec());
            let inline_z = r.register_type_scoped("__inline_obj_3", z.to_vec());
            let merged_xyz = r.register_type_scoped("__merged_3_4", xyz.to_vec());
            (base, inline_z, merged_xyz)
        }

        // Order 1: object-spread set alone.
        let mut r1 = TypeSchemaRegistry::new();
        let (_b1, inline_z_1, merged_xyz_1) = register_spread_set(&mut r1, &xy, &z, &xyz);

        // Order 2: register an unrelated Snapshot enum FIRST (the Wave-2
        // regression trigger — advances the shared counter), then the same set.
        let mut r2 = TypeSchemaRegistry::new();
        r2.register_enum_scoped(
            "Snapshot",
            vec![
                EnumVariantInfo::new("Pending", 0, 0),
                EnumVariantInfo::new("Ready", 1, 1),
                EnumVariantInfo::new("Failed", 2, 1),
            ],
        );
        let (_b2, inline_z_2, merged_xyz_2) = register_spread_set(&mut r2, &xy, &z, &xyz);

        // Core: inline [z] and merged [x,y,z] are DISTINCT handles in both orders.
        assert_ne!(inline_z_1, merged_xyz_1, "order 1: [z] != [x,y,z]");
        assert_ne!(
            inline_z_2, merged_xyz_2,
            "order 2 (Snapshot-seeded): [z] != [x,y,z]"
        );

        // Arity is correct in both — get_by_id resolves the right structure.
        assert_eq!(r1.get_by_id(inline_z_1).unwrap().field_count(), 1);
        assert_eq!(r1.get_by_id(merged_xyz_1).unwrap().field_count(), 3);
        assert_eq!(r2.get_by_id(inline_z_2).unwrap().field_count(), 1);
        assert_eq!(r2.get_by_id(merged_xyz_2).unwrap().field_count(), 3);

        // Handle VALUES differ across registries (Snapshot shifted the counter)...
        // ...but the anonymous [x,y,z] CONTENT ID is identical regardless of order.
        let cid_1 = r1.get_by_id(merged_xyz_1).unwrap().content_id();
        let cid_2 = r2.get_by_id(merged_xyz_2).unwrap().content_id();
        assert_eq!(cid_1, cid_2, "anon [x,y,z] content id is order-independent");
    }

    #[test]
    fn wf3a_rebuild_content_index_reseats_counter() {
        let r = TypeSchemaRegistry::new();
        // Simulate a rehydrated registry: insert a schema with a high,
        // pre-assigned handle, then rebuild the derived index.
        let schema = TypeSchema::with_id(500, "Loaded", vec![("f".to_string(), FieldType::I64)]);
        // Direct by_name/by_id seeding to model a deserialized registry.
        {
            let mut r_mut = r;
            r_mut.by_id.insert(500, "Loaded".to_string());
            r_mut.by_name.insert("Loaded".to_string(), schema);
            r_mut.rebuild_content_index();
            // A fresh intern must not reuse handle 500 or below.
            let fresh = r_mut
                .register_type_scoped("__inline_obj_1", vec![("g".to_string(), FieldType::I64)]);
            assert!(
                fresh > 500,
                "fresh handle {fresh} must be past the loaded tail"
            );
        }
    }
}
