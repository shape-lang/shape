//! Type Schema Registry for JIT Type Specialization
//!
//! This module provides compile-time type information for optimizing
//! field access in the JIT compiler. When the type of an object is known,
//! we can generate direct memory access instead of HashMap lookups.
//!
//! # Overview
//!
//! - `TypeSchema` - Describes the layout of a declared type
//! - `FieldDef` - Defines a single field with name, type, and offset
//! - `TypeSchemaRegistry` - Global registry of all known type schemas
//!
//! # Performance
//!
//! Direct field access: ~2ns vs HashMap lookup: ~25ns (12x faster)
//!
//! # Intersection Types
//!
//! Supports merging multiple schemas for intersection types (`A + B`).
//! Field collisions are detected at compile time and result in errors.

use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use shape_value::heap_value::HeapValue;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ADR-005: TypedFieldValue is the input carrier ABI for object construction.
// Single-discriminator discipline (§Decision §1): all heap types route through
// `Heap(Arc<HeapValue>)` and dispatch via `HeapValue::kind()`. The single
// explicit exception is `String(Arc<String>)` (§Decision §2), justified by
// measured allocation cost on the most common heap type — strings are an
// order of magnitude more frequent than other heap types in stdlib parser
// output, and routing them through `Arc::new(HeapValue::String(arc))` would
// cost one extra `Arc::new` allocation per string field at construction.
//
// Per ADR-005 §Forbidden, do NOT add per-HeapKind variants here
// (Array/Object/HashMap/Decimal/Timestamp/...). Adding any such variant
// requires its own ADR-level justification with measurement.
//
// See docs/adr/005-typed-slot-construction.md.
#[derive(Debug, Clone)]
pub enum TypedFieldValue {
    F64(f64),
    I64(i64),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    U64(u64),
    Bool(bool),
    /// String exception, named and bounded in ADR-005 §Decision §2.
    /// `Arc<String>` is the runtime carrier (refcounted shared ownership);
    /// not `String` (owned), not `&str` (borrowed), not `StringId` (interned).
    /// Future interning layer (ADR-005 §5 Layer 3) coexists by deduplicating
    /// the Arc-inner.
    String(Arc<String>),
    /// Single discriminator for all other heap types. Dispatch via
    /// `HeapValue::kind()`. Per ADR-005 §1, no parallel sum types whose
    /// variants project 1:1 to HeapKind.
    Heap(Arc<HeapValue>),
}

// Module declarations
pub mod builtin_schemas;
pub mod current;
pub mod enum_support;
pub mod field_types;
pub mod intersection;
pub mod physical_binding;
pub mod registry;
pub mod schema;

// Re-export public types for backward compatibility
pub use builtin_schemas::BuiltinSchemaIds;
pub use current::{
    SyncRegistryScope, current_registry, default_registry, try_current_registry, with_async_scope,
};
pub use enum_support::{EnumInfo, EnumVariantInfo};
pub use field_types::{FieldAnnotation, FieldDef, FieldType};
pub use physical_binding::PhysicalSchemaBinding;
pub use registry::{TypeSchemaBuilder, TypeSchemaRegistry};
pub use schema::{TypeBinding, TypeBindingError, TypeSchema};

/// Error type for schema operations
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    /// Field collision detected during intersection merge
    #[error("Field collision on '{field_name}': type '{type1}' vs '{type2}'")]
    FieldCollision {
        field_name: String,
        type1: String,
        type2: String,
    },
    /// Schema not found
    #[error("Schema not found: {0}")]
    NotFound(String),
}

/// Unique identifier for a type schema
pub type SchemaId = u32;

/// Ensure all future schema IDs from the current ambient registry are
/// strictly greater than `max_existing_id`.
///
/// Used when loading externally compiled/cached bytecode that may contain
/// schema IDs from previous processes. Since B1.7 the reservation lands
/// on [`current_registry`] instead of a process-global counter, so each
/// runtime narrows the reservation to its own domain.
pub fn ensure_next_schema_id_above(max_existing_id: SchemaId) {
    current_registry().ensure_next_id_above(max_existing_id);
}

/// Register a predeclared schema with `FieldType::Any` for the given ordered fields.
///
/// This is intended for compile-time schema derivation paths (extensions/comptime)
/// that need runtime object construction without runtime schema synthesis.
///
/// Since B1.7 the registration targets the ambient [`current_registry`]
/// exclusively; scopeless callers land on the process-wide default
/// registry exposed by that accessor. The previous `FALLBACK_PREDECLARED_REGISTRY`
/// static has been retired.
pub fn register_predeclared_any_schema(fields: &[String]) -> SchemaId {
    current_registry().register_predeclared_any_schema(fields)
}

fn lookup_predeclared_schema_by_id(id: SchemaId) -> Option<TypeSchema> {
    current_registry().lookup_predeclared_by_id(id)
}

fn lookup_predeclared_schema_id(fields: &[&str]) -> Option<SchemaId> {
    let reg = current_registry();

    // Order-sensitive fast path over the current registry's predeclared cache.
    if let Some(id) = reg.lookup_predeclared_id_by_field_order(fields) {
        return Some(id);
    }

    // Ordered match against user-registered / stdlib schemas in the ambient
    // registry.
    reg.type_names()
        .filter_map(|name| reg.get(name))
        .find(|schema| {
            if schema.fields.len() != fields.len() {
                return false;
            }
            schema
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .eq(fields.iter().copied())
        })
        .map(|schema| schema.id)
}

fn lookup_schema_by_id(id: SchemaId) -> Option<TypeSchema> {
    let reg = current_registry();
    if let Some(schema) = reg.get_by_id(id).cloned() {
        return Some(schema);
    }
    reg.lookup_predeclared_by_id(id)
}

/// Public wrapper for looking up a schema by ID across all registries
/// (stdlib + predeclared). Used by wire_conversion when Context registry
/// doesn't have the schema (e.g. ad-hoc/const-eval objects).
pub fn lookup_schema_by_id_public(id: SchemaId) -> Option<TypeSchema> {
    lookup_schema_by_id(id)
}

fn schema_matches_field_set(schema: &TypeSchema, fields: &[&str]) -> bool {
    if schema.fields.len() != fields.len() {
        return false;
    }
    let wanted: HashSet<&str> = fields.iter().copied().collect();
    schema
        .fields
        .iter()
        .all(|field| wanted.contains(field.name.as_str()))
}

/// Resolve a schema for a field list.
///
/// Resolution is order-sensitive first (fast path), then order-insensitive
/// fallback for wire/object map roundtrips where key ordering is unstable.
/// If no existing schema matches, auto-registers an anonymous `FieldType::Any`
/// schema so that ad-hoc objects (const eval, tests, FFI) work without
/// explicit pre-registration.
fn lookup_schema_for_fields(fields: &[&str]) -> Option<TypeSchema> {
    if let Some(id) = lookup_predeclared_schema_id(fields) {
        return lookup_schema_by_id(id);
    }

    let reg = current_registry();
    // Order-insensitive match over the current registry's named schemas.
    if let Some(schema) = reg
        .type_names()
        .filter_map(|name| reg.get(name))
        .find(|schema| schema_matches_field_set(schema, fields))
    {
        return Some(schema.clone());
    }
    if let Some(schema) = reg.lookup_predeclared_by_field_set(fields) {
        return Some(schema);
    }

    // Auto-register an anonymous schema for ad-hoc field sets.
    let owned: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
    let id = register_predeclared_any_schema(&owned);
    lookup_predeclared_schema_by_id(id)
}

/// Create a `KindedSlot` carrying a `HeapValue::TypedObject` from a list
/// of `(name, KindedSlot)` field pairs.
///
/// Per ADR-006 §2.7.4 audit-accuracy ruling + §2.7.3 N9 cleanup
/// pre-flag, the previous `nb_to_slot` body relied on tag-bit dispatch
/// via `value.is_heap()` / `value.raw_bits()` / `value.as_heap_ref()`
/// / `value.as_any_array().to_generic()` (the forbidden N9
/// tag-decoding pattern). The kind-threaded rebuild reads each pair's
/// `NativeKind` from the `KindedSlot::kind` field (single source of
/// truth) and dispatches per-kind to the matching per-FieldType
/// `ValueSlot::from_*` constructor — no heap materialization, no
/// `is_heap()` consultation. The slot's strong-count share is moved
/// into the typed-object's slot list (the caller's `KindedSlot::clone`
/// bumped it on construction).
pub fn typed_object_from_pairs(fields: &[(&str, KindedSlot)]) -> KindedSlot {
    let field_names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    let schema = lookup_schema_for_fields(&field_names).unwrap_or_else(|| {
        panic!(
            "Missing predeclared schema for fields [{}]. Runtime schema synthesis is disabled.",
            field_names.join(", ")
        )
    });
    let value_by_name: HashMap<&str, &KindedSlot> =
        fields.iter().map(|(name, value)| (*name, value)).collect();

    // Build slots — `NativeKind` selects the per-FieldType constructor.
    // Heap arms set the heap_mask bit; inline-scalar arms do not.
    let mut slots = Vec::with_capacity(schema.fields.len());
    let mut heap_mask: u64 = 0;
    for (i, field_def) in schema.fields.iter().enumerate() {
        let value = value_by_name
            .get(field_def.name.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "Missing field '{}' while materializing typed object",
                    field_def.name
                )
            });
        // `KindedSlot::clone` bumps the heap refcount; the resulting
        // `ValueSlot` owns one strong-count share independent of the
        // input pair's share. The bits transfer is a memcpy of the raw
        // u64; the explicit `clone()` does the per-kind retain.
        let cloned = (*value).clone();
        let bits = cloned.slot().raw();
        let is_heap = match cloned.kind() {
            NativeKind::String | NativeKind::Ptr(_) => true,
            _ => false,
        };
        // Forget the cloned `KindedSlot` so its `Drop` does not
        // decrement the share we just transferred into the slot list.
        std::mem::forget(cloned);
        let slot = ValueSlot::from_raw(bits);
        slots.push(slot);
        if is_heap {
            heap_mask |= 1u64 << i;
        }
    }

    let storage = Arc::new(shape_value::TypedObjectStorage::new(
        schema.id as u64,
        slots.into_boxed_slice(),
        heap_mask,
        // No per-slot kind table is recorded on this fast path — the
        // schema's `FieldType`s are the source of truth at read time.
        Arc::from(Vec::<NativeKind>::new().into_boxed_slice()),
    ));
    let _: &HeapValue = &HeapValue::TypedObject(storage.clone());
    KindedSlot::new(
        ValueSlot::from_typed_object(storage),
        NativeKind::Ptr(HeapKind::TypedObject),
    )
}

#[cfg(test)]
mod tests {
    // Pre-bulldozer tests of `typed_object_from_pairs` /
    // `typed_object_to_hashmap_nb` decoded slots via `ValueWord`'s
    // `.as_str()` / `.as_i64()` methods. Phase 1.B retires those
    // accessors with the rest of `ValueWord`. Behavioural coverage of
    // typed-object construction returns when shape-vm Cluster #4 lands
    // its kind-threaded slot tests.
}

/// Convert a TypedObject `KindedSlot` back to a `HashMap<String, KindedSlot>`.
///
/// Inverse of [`typed_object_from_pairs`]. Reads the `TypedObject` heap
/// value and rebuilds a per-field map keyed by the schema's field
/// names. Phase 1.B (ADR-006 §2.7.4 audit-accuracy ruling): the per-
/// slot `NativeKind` is derived from the schema's `FieldType` — the
/// stored slots carry no per-position kind metadata in the current
/// fast path. Phase 2c lands schema → `NativeKind` lowering as a
/// shared utility; until then this helper returns `None` when the
/// schema is not registered or the value is not a TypedObject.
pub fn typed_object_to_hashmap_nb(
    _value: &KindedSlot,
) -> Option<HashMap<String, KindedSlot>> {
    // Phase 1.B: schema → NativeKind lowering is the deferred Phase 2c
    // utility. This helper's pre-bulldozer body decoded slots via
    // `slots[i].as_heap_nb()` / `ValueWord::clone_from_bits` (now
    // deleted). Returning `None` keeps callers honest until the kind-
    // threaded rebuild lands; the only current consumer is the deleted
    // unit test above.
    None
}

