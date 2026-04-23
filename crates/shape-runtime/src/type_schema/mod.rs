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

use shape_value::{ValueWord, ValueWordExt};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};

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
    SyncRegistryScope, current_registry, try_current_registry, with_async_scope,
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

/// Counter for generating unique schema IDs
static NEXT_SCHEMA_ID: AtomicU32 = AtomicU32::new(1);

/// Generate a new unique schema ID
pub(crate) fn next_schema_id() -> SchemaId {
    NEXT_SCHEMA_ID.fetch_add(1, Ordering::SeqCst)
}

/// Ensure all future schema IDs are strictly greater than `max_existing_id`.
///
/// This is used when loading externally compiled/cached bytecode that may
/// contain schema IDs from previous processes. Without reserving past the
/// maximum observed ID, newly created runtime schemas could collide.
pub fn ensure_next_schema_id_above(max_existing_id: SchemaId) {
    let required_next = max_existing_id.saturating_add(1);
    let mut current = NEXT_SCHEMA_ID.load(Ordering::SeqCst);

    while current < required_next {
        match NEXT_SCHEMA_ID.compare_exchange(
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

/// Global stdlib/builtin schema registry for `typed_object_to_hashmap` lookups.
/// Initialized once with builtin types (__AnyError, __TraceFrame, etc.).
static STDLIB_SCHEMA_REGISTRY: std::sync::LazyLock<TypeSchemaRegistry> =
    std::sync::LazyLock::new(|| {
        let (registry, _ids) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        registry
    });

/// Process-wide fallback registry that owns the predeclared-schema state
/// used when no runtime scope is active.
///
/// Prior to B1.6 the predeclared maps were standalone process-global
/// statics (`PREDECLARED_SCHEMA_CACHE` / `PREDECLARED_SCHEMA_REGISTRY`).
/// B1.6 moves the logic onto `TypeSchemaRegistry` as instance state; this
/// fallback exists so the free functions still work when invoked outside
/// any Runtime scope (old tests, static setup, ad-hoc tooling). B1.7
/// retires both the fallback and the `STDLIB_SCHEMA_REGISTRY` static.
static FALLBACK_PREDECLARED_REGISTRY: std::sync::LazyLock<TypeSchemaRegistry> =
    std::sync::LazyLock::new(TypeSchemaRegistry::new);

/// Register a predeclared schema with `FieldType::Any` for the given ordered fields.
///
/// This is intended for compile-time schema derivation paths (extensions/comptime)
/// that need runtime object construction without runtime schema synthesis.
///
/// Writes to the process-wide fallback registry (preserving B1.5
/// behaviour where predeclared schemas were globally visible across
/// Runtimes) AND mirrors into the current ambient registry when present
/// so per-Runtime lookups also succeed. The SchemaId is drawn from the
/// global `NEXT_SCHEMA_ID` counter so it remains interleaved with other
/// global-counter allocations (builtin schemas, no-scope
/// `TypeSchema::new` calls), matching pre-B1.6 test ordering. B1.7
/// retires the fallback once all call sites hold a scope.
pub fn register_predeclared_any_schema(fields: &[String]) -> SchemaId {
    let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();

    // Primary write: process-wide fallback. Check cache first to avoid
    // re-allocating global IDs on repeat calls.
    if let Some(existing) = FALLBACK_PREDECLARED_REGISTRY
        .lookup_predeclared_id_by_field_order(&field_refs)
    {
        if let Some(reg) = try_current_registry() {
            reg.mirror_predeclared_any_schema(fields, existing);
        }
        return existing;
    }

    // Allocate from the global counter (legacy behaviour) so the
    // predeclared ID domain does not collide with per-registry scoped
    // allocations (1, 2, 3 ...).
    let id = next_schema_id();
    FALLBACK_PREDECLARED_REGISTRY.mirror_predeclared_any_schema(fields, id);

    // Mirror into the ambient registry if one is active so scoped
    // lookups observe the same schema without cross-referencing the
    // fallback.
    if let Some(reg) = try_current_registry() {
        reg.mirror_predeclared_any_schema(fields, id);
    }
    id
}

fn lookup_predeclared_schema_by_id(id: SchemaId) -> Option<TypeSchema> {
    if let Some(reg) = try_current_registry() {
        if let Some(schema) = reg.lookup_predeclared_by_id(id) {
            return Some(schema);
        }
    }
    FALLBACK_PREDECLARED_REGISTRY.lookup_predeclared_by_id(id)
}

fn lookup_predeclared_schema_id(fields: &[&str]) -> Option<SchemaId> {
    // Order-sensitive fast path over the current registry's predeclared cache.
    if let Some(reg) = try_current_registry() {
        if let Some(id) = reg.lookup_predeclared_id_by_field_order(fields) {
            return Some(id);
        }

        // Ordered match against user-registered / stdlib schemas in the
        // ambient registry (previously only searched STDLIB_SCHEMA_REGISTRY).
        if let Some(id) = reg
            .type_names()
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
        {
            return Some(id);
        }
    }

    // Fallback-registry cache (no-scope callers).
    if let Some(id) = FALLBACK_PREDECLARED_REGISTRY.lookup_predeclared_id_by_field_order(fields) {
        return Some(id);
    }

    STDLIB_SCHEMA_REGISTRY
        .type_names()
        .filter_map(|name| STDLIB_SCHEMA_REGISTRY.get(name))
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
    // Prefer the current ambient registry (stdlib + predeclared); fall
    // back to the legacy process-global static and the process-wide
    // fallback predeclared registry during the B1 migration window.
    if let Some(reg) = try_current_registry() {
        if let Some(schema) = reg.get_by_id(id).cloned() {
            return Some(schema);
        }
        if let Some(schema) = reg.lookup_predeclared_by_id(id) {
            return Some(schema);
        }
    }
    if let Some(schema) = STDLIB_SCHEMA_REGISTRY.get_by_id(id).cloned() {
        return Some(schema);
    }
    FALLBACK_PREDECLARED_REGISTRY.lookup_predeclared_by_id(id)
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

    // Ambient-first: order-insensitive match over the current registry
    // (user/stdlib types + predeclared schemas).
    if let Some(reg) = try_current_registry() {
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
    }

    if let Some(schema) = STDLIB_SCHEMA_REGISTRY
        .type_names()
        .filter_map(|name| STDLIB_SCHEMA_REGISTRY.get(name))
        .find(|schema| schema_matches_field_set(schema, fields))
    {
        return Some(schema.clone());
    }

    // Fallback registry (no-scope callers).
    if let Some(schema) = FALLBACK_PREDECLARED_REGISTRY.lookup_predeclared_by_field_set(fields) {
        return Some(schema);
    }

    // Auto-register an anonymous schema for ad-hoc field sets
    let owned: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
    let id = register_predeclared_any_schema(&owned);
    lookup_predeclared_schema_by_id(id)
}

/// Create a `ValueWord::TypedObject` from a list of key-value pairs.
///
/// This is the standalone equivalent of `VirtualMachine::create_typed_object_from_pairs()`.
/// It requires a matching predeclared schema in the stdlib schema registry.
/// Runtime schema synthesis is not allowed.
///
/// Safe to call from any crate (shape-runtime, shape-vm, tests) without needing
/// a `&mut VirtualMachine` reference.
///
/// # Example
/// ```ignore
/// use shape_runtime::type_schema::typed_object_from_pairs;
///
/// let obj = typed_object_from_pairs(&[
///     ("name", ValueWord::from_string(Arc::new("hello".into()))),
///     ("count", ValueWord::from_i64(42)),
/// ]);
/// ```
pub fn typed_object_from_pairs(fields: &[(&str, ValueWord)]) -> ValueWord {
    let field_names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    let schema = lookup_schema_for_fields(&field_names).unwrap_or_else(|| {
        panic!(
            "Missing predeclared schema for fields [{}]. Runtime schema synthesis is disabled.",
            field_names.join(", ")
        )
    });
    let value_by_name: HashMap<&str, &ValueWord> =
        fields.iter().map(|(name, value)| (*name, value)).collect();

    // Build slots — inline types stored as inline ValueSlots, heap types as heap pointers
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
        let (slot, is_heap) = nb_to_slot(value);
        slots.push(slot);
        if is_heap {
            heap_mask |= 1u64 << i;
        }
    }

    ValueWord::from_heap_value(shape_value::heap_value::HeapValue::TypedObject {
        schema_id: schema.id as u64,
        slots: slots.into_boxed_slice(),
        heap_mask,
    })
}

/// Create an anonymous `TypedObject` from ValueWord field pairs.
///
/// Like `typed_object_from_pairs` but takes `ValueWord` values directly,
/// avoiding ValueWord construction.
///
/// Returns a `ValueWord` wrapping the TypedObject on the heap.
pub fn typed_object_from_nb_pairs(
    fields: &[(&str, shape_value::ValueWord)],
) -> shape_value::ValueWord {
    let field_names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    let schema = lookup_schema_for_fields(&field_names).unwrap_or_else(|| {
        panic!(
            "Missing predeclared schema for fields [{}]. Runtime schema synthesis is disabled.",
            field_names.join(", ")
        )
    });
    let value_by_name: HashMap<&str, &shape_value::ValueWord> =
        fields.iter().map(|(name, value)| (*name, value)).collect();

    // Build slots — inline types as inline ValueSlots, heap types as heap pointers
    let mut slots = Vec::with_capacity(schema.fields.len());
    let mut heap_mask: u64 = 0;
    for (i, field_def) in schema.fields.iter().enumerate() {
        let nb = value_by_name
            .get(field_def.name.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "Missing field '{}' while materializing typed object",
                    field_def.name
                )
            });
        let (slot, is_heap) = nb_to_slot(nb);
        slots.push(slot);
        if is_heap {
            heap_mask |= 1u64 << i;
        }
    }

    shape_value::ValueWord::from_heap_value(shape_value::heap_value::HeapValue::TypedObject {
        schema_id: schema.id as u64,
        slots: slots.into_boxed_slice(),
        heap_mask,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::{ValueWord, ValueWordExt};

    #[test]
    fn typed_object_from_nb_pairs_is_order_insensitive_for_builtin_schema() {
        let obj = typed_object_from_nb_pairs(&[
            (
                "function",
                ValueWord::from_string(std::sync::Arc::new("f".to_string())),
            ),
            (
                "file",
                ValueWord::from_string(std::sync::Arc::new("m".to_string())),
            ),
            ("line", ValueWord::from_i64(42)),
            ("ip", ValueWord::from_i64(7)),
        ]);

        let map = typed_object_to_hashmap_nb(&obj).expect("typed object should decode");
        assert_eq!(map.get("function").and_then(|v| v.as_str()), Some("f"));
        assert_eq!(map.get("file").and_then(|v| v.as_str()), Some("m"));
        assert_eq!(map.get("line").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(map.get("ip").and_then(|v| v.as_i64()), Some(7));
    }
}

/// Convert a TypedObject `ValueWord` back to a `HashMap<String, ValueWord>`.
///
/// This is the inverse of `typed_object_from_pairs`. It looks up the schema
/// to recover field names, then extracts each slot's heap value.
///
/// Returns `None` if the value is not a TypedObject or the schema is not found.
pub fn typed_object_to_hashmap(value: &ValueWord) -> Option<HashMap<String, ValueWord>> {
    // Delegate to the ValueWord-native version
    typed_object_to_hashmap_nb(value)
}

/// Convert a ValueWord TypedObject to a `HashMap<String, ValueWord>`.
///
/// Like `typed_object_to_hashmap` but works directly with ValueWord,
/// avoiding ValueWord materialization.
///
/// Returns `None` if the value is not a TypedObject or the schema is not found.
pub fn typed_object_to_hashmap_nb(
    value: &shape_value::ValueWord,
) -> Option<HashMap<String, shape_value::ValueWord>> {
    let (schema_id, slots, heap_mask) = value.as_typed_object()?;
    let sid = schema_id as SchemaId;
    let schema = lookup_schema_by_id(sid)?;
    let mut map = HashMap::with_capacity(schema.fields.len());
    for (i, field_def) in schema.fields.iter().enumerate() {
        if i < slots.len() {
            let val = if heap_mask & (1u64 << i) != 0 {
                slots[i].as_heap_nb()
            } else {
                // Non-heap slot: raw bits are a ValueWord representation
                // (inline f64, i48, bool, none, unit, function, module_fn).
                // Reconstruct the ValueWord from its raw bits.
                // Safety: these bits were stored by nb_to_slot from a valid
                // inline ValueWord, so they are a valid ValueWord representation.
                // For inline tags, clone_from_bits is a pure bitwise copy.
                unsafe { shape_value::ValueWord::clone_from_bits(slots[i].raw()) }
            };
            map.insert(field_def.name.clone(), val);
        }
    }
    Some(map)
}

/// Convert a ValueWord to a ValueSlot, returning `(slot, is_heap)`.
///
/// Inline ValueWord types (f64, i48, bool, none, unit, function, module_fn)
/// are stored as raw ValueWord bits in the slot. Heap types are cloned into a
/// heap-allocated ValueSlot. The `is_heap` flag indicates whether the
/// heap_mask bit should be set.
///
/// For non-heap slots, use `slot_to_nb_inline(slot)` to reconstruct the
/// ValueWord from raw bits.
pub(crate) fn nb_to_slot(nb: &shape_value::ValueWord) -> (shape_value::slot::ValueSlot, bool) {
    use shape_value::slot::ValueSlot;

    if nb.is_heap() {
        {
            // Handle unified heap values (bit-47): materialize to HeapValue.
            if shape_value::ValueBits::from_raw(nb.raw_bits()).is_unified_heap() {
                if let Some(view) = nb.as_any_array() {
                    let hv = shape_value::heap_value::HeapValue::Array(view.to_generic());
                    return (ValueSlot::from_heap(hv), true);
                }
                // For other unified types, store raw bits.
                return (ValueSlot::from_raw(nb.raw_bits()), false);
            }
            // cold-path: as_heap_ref retained — generic heap-to-slot clone
            let hv = nb.as_heap_ref().unwrap().clone(); // cold-path
            (ValueSlot::from_heap(hv), true)
        }
    } else {
        // Store raw ValueWord bits — reconstructible via ValueWord::from_raw_bits()
        (ValueSlot::from_raw(nb.raw_bits()), false)
    }
}
