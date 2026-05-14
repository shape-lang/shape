//! Native `json` module for JSON parsing and serialization.
//!
//! Exports: json.parse(text), json.stringify(value, pretty?), json.is_valid(text)
//!
//! `parse(text)` always returns a typed `Json` enum value. The legacy
//! `json_value_to_nanboxed` untyped fallback was removed in sweep phase 4a.
//! Schema-driven parsing (`__parse_typed`) coerces JSON directly into a
//! TypedObject for the supplied schema; nested unknown objects fall back to
//! the typed `Json` enum rather than an untyped HashMap.
//!
//! Phase-2d strict-typing migration status (Stage D close-out batch,
//! 2026-05-07):
//!
//! - `json.parse(text) -> Result<Json>` — **MIGRATED at Stage D Step 4.**
//!   Body builds the strict-typed `JsonValue` enum
//!   (`crate::json_value::JsonValue`) directly from `serde_json::Value`
//!   and wraps with `TypedReturn::Ok(ConcreteReturn::JsonValue(...))`
//!   per Stage D Step 1's `ConcreteReturn::JsonValue` variant addition
//!   (commit `a022f43`). N6 sub-shape (b1) sign-off; closes B1
//!   sub-decision #2 for json.parse.
//! - `json.__parse_typed(text, schema_id) -> Result<any>` — **MIGRATED
//!   at Stage D close-out Step 3.** Body builds `HeapValue::TypedObject`
//!   directly from the runtime schema + JSON object via
//!   `build_typed_object_from_json`, then wraps the `Arc<HeapValue>` in
//!   `ConcreteReturn::OpaqueTypedObject` per close-out Step 2's variant
//!   addition (commit `1bca2c4`). N8 sign-off; closes B1 sub-decision
//!   #2 for json.__parse_typed. The 5 legacy ValueWord-using helpers
//!   (make_json_enum / json_value_to_enum / json_object_to_typed /
//!   json_value_to_typed_nb / json_value_to_typed_json_enum) were
//!   DELETED at close-out Step 3 — verified call-graph private to
//!   `__parse_typed` before deletion.
//! - `json.stringify(value: any, pretty?: bool) -> Result<string>` —
//!   DEFERRED pending **N7** (HeapValue→JSON serializer for HTTP /
//!   object-output marshal contexts). N7 is the unified workstream
//!   covering HTTP post_json/put_json + yaml/toml/msgpack
//!   stringify/encode/encode_bytes (6 consumers total). Body uses
//!   deleted `to_json_value()` + would need the N7 serializer.
//! - `json.is_valid(text) -> bool` — Migratable in isolation but kept
//!   deferred for per-file atomicity; lands with stringify when N7
//!   sign-off unblocks the residual json cohort.
//!
//! N7 is supervisor-level; queued for next-session relay batch (see
//! `docs/defections.md` HashMap-marshal cluster sub-decision queue
//! 2026-05-07 Stage B+D close-out subsection).
//!
//! Strict-typed helpers `serde_json_to_json_value` (used by json.parse),
//! `build_json_enum_heap_value`, `build_field_slot_from_json`, and
//! `build_typed_object_from_json` (used by __parse_typed) construct
//! ValueSlots directly from native types via the `ValueSlot::from_*`
//! primitives — no ValueWord intermediate, no call to `nb_to_slot`.
//!
//! Note: `nb_to_slot` (defined `pub(crate)` at
//! `crate::type_schema::mod`) and adjacent slot-construction code in
//! `type_schema/mod.rs` still cite the deleted `ValueWord` API. That
//! cleanup is **N9 candidate** — type_schema/mod.rs slot-construction-
//! layer migration. Tracked separately for next-session pickup; this
//! commit explicitly does NOT touch type_schema/mod.rs (verification
//! gate caught the cross-cutting concern; Option A2 chosen over A1 to
//! preserve per-file atomicity).

use crate::json_value::JsonValue;
use crate::marshal::{register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::type_schema::TypeSchemaRegistry;
use crate::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::heap_value::HeapValue;
use shape_value::{KindedSlot, ValueSlot};
use std::sync::Arc;

// Json enum variant IDs (must match order in json_value.shape).
//
// Layout: Null | Bool(bool) | Int(int) | Number(number) | Str(string)
//       | Array(any) | Object(any)
const JSON_VARIANT_NULL: i64 = 0;
const JSON_VARIANT_BOOL: i64 = 1;
const JSON_VARIANT_INT: i64 = 2;
const JSON_VARIANT_NUMBER: i64 = 3;
const JSON_VARIANT_STR: i64 = 4;
const JSON_VARIANT_ARRAY: i64 = 5;
const JSON_VARIANT_OBJECT: i64 = 6;

/// Build a Json-enum `HeapValue::TypedObject` directly from a
/// `serde_json::Value`. Used as the `FieldType::Any` fallback path in
/// `json.__parse_typed` — when a schema field is typed `any`, the JSON
/// payload is stored as a strict-typed `Json` enum tree
/// (`HeapValue::TypedObject` keyed by the Json schema). Recursion lives
/// at the HeapValue layer; each variant's payload is built directly via
/// `ValueSlot::from_*` primitives without ValueWord intermediates.
///
/// The Json enum's layout: slot 0 = `__variant` (I64), slot 1 =
/// `__payload_0` (heap or inline native). Variant IDs match
/// `JSON_VARIANT_*` constants which mirror `json_value.shape`.
///
/// Integral JSON numbers that fit in `i64` map to `Json::Int`; all other
/// numbers map to `Json::Number(f64)`. Preserves the `int` / `number`
/// distinction at the boundary.
fn build_json_enum_heap_value(value: serde_json::Value, json_schema_id: u64) -> HeapValue {
    let (variant_id, payload_slot, payload_is_heap) = match value {
        serde_json::Value::Null => (JSON_VARIANT_NULL, ValueSlot::none(), false),
        serde_json::Value::Bool(b) => (JSON_VARIANT_BOOL, ValueSlot::from_bool(b), false),
        serde_json::Value::Number(n) => {
            // Prefer Json::Int for integral i64-fitting numbers.
            if let Some(i) = n.as_i64() {
                if !n.to_string().contains('.') {
                    return build_typed_object(
                        json_schema_id,
                        vec![
                            ValueSlot::from_int(JSON_VARIANT_INT),
                            ValueSlot::from_int(i),
                        ],
                        0,
                    );
                }
            }
            (
                JSON_VARIANT_NUMBER,
                ValueSlot::from_number(n.as_f64().unwrap_or(0.0)),
                false,
            )
        }
        serde_json::Value::String(s) => (
            JSON_VARIANT_STR,
            ValueSlot::from_string_arc(Arc::new(s)),
            true,
        ),
        serde_json::Value::Array(arr) => {
            // W17-typed-carrier-bundle-A checkpoint 2/4: every JSON array
            // element is a `Json` enum-TypedObject built by
            // `build_json_enum_heap_value` — so the array always lowers to
            // `TypedArrayData::TypedObject` per ADR-006 §2.7.24 Q25.A. The
            // prior polymorphic `the-deleted-heterogeneous-element-carrier` carrier is gone.
            let elements: Vec<Arc<HeapValue>> = arr
                .into_iter()
                .map(|v| Arc::new(build_json_enum_heap_value(v, json_schema_id)))
                .collect();
            let data = Arc::new(
                shape_value::TypedArrayData::build_specialized_from_heap_arcs(elements)
                    .expect("json: build_json_enum_heap_value always returns TypedObject"),
            );
            (JSON_VARIANT_ARRAY, ValueSlot::from_typed_array(data), true)
        }
        serde_json::Value::Object(map) => {
            // Build a HashMap-shaped HeapValue (insertion order preserved).
            let mut keys_vec: Vec<Arc<String>> = Vec::with_capacity(map.len());
            let mut values_vec: Vec<Arc<HeapValue>> = Vec::with_capacity(map.len());
            for (k, v) in map.into_iter() {
                keys_vec.push(Arc::new(k));
                values_vec.push(Arc::new(build_json_enum_heap_value(v, json_schema_id)));
            }
            let hm = shape_value::heap_value::HashMapData::from_pairs(keys_vec, values_vec);
            (
                JSON_VARIANT_OBJECT,
                ValueSlot::from_hashmap(Arc::new(hm)),
                true,
            )
        }
    };
    let heap_mask = if payload_is_heap { 1u64 << 1 } else { 0u64 };
    build_typed_object(
        json_schema_id,
        vec![ValueSlot::from_int(variant_id), payload_slot],
        heap_mask,
    )
}

/// Build a `HeapValue::TypedObject(Arc<TypedObjectStorage>)` from raw
/// slots + a `heap_mask`. The schema's `FieldType`s are the source of
/// truth at read time — no per-slot kind table is recorded on this
/// fast path (mirrors `type_schema::typed_object_from_pairs`).
fn build_typed_object(schema_id: u64, slots: Vec<ValueSlot>, heap_mask: u64) -> HeapValue {
    // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new` per D1 API
    // surface. `HeapValue::TypedObject` variant signature flip is
    // ckpt-final territory; the wrap below will not compile until the
    // variant signature shifts to `*const TypedObjectStorage`.
    let storage = shape_value::TypedObjectStorage::_new(
        schema_id,
        slots.into_boxed_slice(),
        heap_mask,
        Arc::from(Vec::<shape_value::NativeKind>::new().into_boxed_slice()),
    );
    HeapValue::TypedObject(storage)
}

/// Convert a `serde_json::Value` into the strict-typed `JsonValue` sum
/// (`crate::json_value::JsonValue`).
///
/// Stage D Step 4 (2026-05-07). Used by `json.parse` to produce an
/// `Arc<HeapValue>`-free recursive value tree that wraps directly into
/// `ConcreteReturn::JsonValue`. Same int-vs-number split rule as the
/// legacy `json_value_to_enum`: integral JSON numbers fitting in `i64`
/// map to `JsonValue::Int`; all other numbers map to `JsonValue::Number`.
fn serde_json_to_json_value(value: serde_json::Value) -> JsonValue {
    match value {
        serde_json::Value::Null => JsonValue::Null,
        serde_json::Value::Bool(b) => JsonValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if !n.to_string().contains('.') {
                    return JsonValue::Int(i);
                }
            }
            JsonValue::Number(n.as_f64().unwrap_or(0.0))
        }
        serde_json::Value::String(s) => JsonValue::String(s),
        serde_json::Value::Array(arr) => {
            JsonValue::Array(arr.into_iter().map(serde_json_to_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let pairs: Vec<(String, JsonValue)> = map
                .into_iter()
                .map(|(k, v)| (k, serde_json_to_json_value(v)))
                .collect();
            JsonValue::Object(pairs)
        }
    }
}

/// Build a single `ValueSlot` for a schema field given its declared type
/// and a JSON value. Returns `(slot, is_heap)` where `is_heap` is the
/// bit to set in `heap_mask` if the slot stores a heap pointer.
///
/// For typed fields (I64/F64/Bool/String/Decimal/Object-with-known-schema),
/// produces the strict-typed slot directly via `ValueSlot::from_*`
/// primitives. For `FieldType::Any` and untypable shapes (Array, mixed
/// types, Object-without-known-schema), falls back to a Json-enum-tree
/// HeapValue via `build_json_enum_heap_value`.
fn build_field_slot_from_json(
    value: &serde_json::Value,
    field_type: &crate::type_schema::FieldType,
    registry: &TypeSchemaRegistry,
    json_schema_id: u64,
) -> Result<(ValueSlot, bool), String> {
    use crate::type_schema::FieldType;
    use serde_json::Value;
    match (value, field_type) {
        (Value::Null, _) => Ok((ValueSlot::none(), false)),
        (Value::Bool(b), FieldType::Bool) => Ok((ValueSlot::from_bool(*b), false)),
        (Value::Number(n), FieldType::I64) => {
            Ok((ValueSlot::from_int(n.as_i64().unwrap_or(0)), false))
        }
        (Value::Number(n), FieldType::F64) | (Value::Number(n), FieldType::Decimal) => Ok((
            ValueSlot::from_number(n.as_f64().unwrap_or(0.0)),
            false,
        )),
        (Value::String(s), FieldType::String) => {
            Ok((ValueSlot::from_string_arc(Arc::new(s.clone())), true))
        }
        (Value::Object(obj), FieldType::Object(type_name)) => {
            if let Some(nested_schema) = registry.get(type_name) {
                let nested_hv =
                    build_typed_object_from_json(nested_schema, obj, registry, json_schema_id)?;
                Ok((heap_to_slot(nested_hv), true))
            } else {
                // Nested type's schema not registered — fall back to a
                // typed `Json::Object` HeapValue per the legacy contract.
                let json_hv =
                    build_json_enum_heap_value(Value::Object(obj.clone()), json_schema_id);
                Ok((heap_to_slot(json_hv), true))
            }
        }
        // FieldType::Any or any other shape (Array, type-mismatched, etc.)
        // → fall back to a Json enum tree at the slot.
        _ => {
            let json_hv = build_json_enum_heap_value(value.clone(), json_schema_id);
            Ok((heap_to_slot(json_hv), true))
        }
    }
}

/// Project a `HeapValue` (typically a `TypedObject` produced by the
/// nested-schema path or a `Json::Object` enum from the fallback path)
/// into a typed `ValueSlot` via the matching per-FieldType constructor.
/// Used by the JSON-tree builder to avoid the deprecated
/// `ValueSlot::from_heap(HeapValue)` boxing path.
fn heap_to_slot(hv: HeapValue) -> ValueSlot {
    match hv {
        HeapValue::TypedObject(arc) => ValueSlot::from_typed_object(arc),
        HeapValue::String(arc) => ValueSlot::from_string_arc(arc),
        HeapValue::TypedArray(arc) => ValueSlot::from_typed_array(arc),
        HeapValue::HashMap(arc) => ValueSlot::from_hashmap(arc),
        HeapValue::Decimal(arc) => ValueSlot::from_decimal(arc),
        HeapValue::BigInt(arc) => ValueSlot::from_bigint(arc),
        HeapValue::DataTable(arc) => ValueSlot::from_data_table(arc),
        HeapValue::IoHandle(arc) => ValueSlot::from_io_handle(arc),
        HeapValue::NativeView(arc) => ValueSlot::from_native_view(arc),
        // Inline-scalar / less-common variants fall back to the deprecated
        // boxing path until per-variant constructors land in Phase 2c.
        #[allow(deprecated)]
        other => ValueSlot::from_heap(other),
    }
}

/// Build a `HeapValue::TypedObject` keyed by the given schema, populated
/// from a JSON object. Matches JSON keys to schema fields using
/// `wire_name()` (respects `@alias`). Missing fields are written as
/// `ValueSlot::none()` with no heap_mask bit set.
fn build_typed_object_from_json(
    schema: &crate::type_schema::TypeSchema,
    map: &serde_json::Map<String, serde_json::Value>,
    registry: &TypeSchemaRegistry,
    json_schema_id: u64,
) -> Result<HeapValue, String> {
    let num_fields = schema.fields.len();
    let mut slots = vec![ValueSlot::none(); num_fields];
    let mut heap_mask = 0u64;

    for field in &schema.fields {
        let wire = field.wire_name();
        let (slot, is_heap) = if let Some(jv) = map.get(wire) {
            build_field_slot_from_json(jv, &field.field_type, registry, json_schema_id)?
        } else {
            (ValueSlot::none(), false)
        };
        slots[field.index as usize] = slot;
        if is_heap {
            heap_mask |= 1u64 << field.index;
        }
    }

    Ok(build_typed_object(
        schema.id as u64,
        slots,
        heap_mask,
    ))
}

/// Create the `json` module with JSON parsing and serialization functions.
pub fn create_json_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::json");
    module.description = "JSON parsing and serialization".to_string();

    // json.parse(text: string) -> Result<Json>
    // Stage D Step 4 (2026-05-07): migrated to the strict-typed marshal
    // layer. Body builds `JsonValue` (`crate::json_value::JsonValue`)
    // directly and wraps with `TypedReturn::Ok(ConcreteReturn::JsonValue(..))`
    // per Step 1's variant addition. No body-time schema lookup —
    // `ConcreteType::JsonValue("Json")` carries the type-name at the
    // registration-display layer.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse",
        "Parse a JSON string into Shape values",
        "text",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::JsonValue("Json".to_string()))),
        |text: Arc<String>, _ctx| {
            let parsed: serde_json::Value = serde_json::from_str(text.as_str())
                .map_err(|e| format!("json.parse() failed: {}", e))?;

            let result = serde_json_to_json_value(parsed);

            Ok(TypedReturn::Ok(ConcreteReturn::JsonValue(result)))
        },
    );

    // json.__parse_typed(text: string, schema_id: number) -> Result<any>
    // Stage D close-out Step 3 (2026-05-07): migrated to the strict-typed
    // marshal layer via Step 2's `ConcreteReturn::OpaqueTypedObject`
    // variant (commit `1bca2c4`). Body builds `HeapValue::TypedObject`
    // directly from the runtime schema + JSON object via
    // `build_typed_object_from_json`, then wraps the `Arc<HeapValue>` in
    // `ConcreteReturn::OpaqueTypedObject` per the N8 sign-off framing.
    //
    // The 5 legacy ValueWord-using helpers (make_json_enum,
    // json_value_to_enum, json_object_to_typed, json_value_to_typed_nb,
    // json_value_to_typed_json_enum) are DELETED. The strict-typed
    // replacements (`build_json_enum_heap_value`,
    // `build_field_slot_from_json`, `build_typed_object_from_json`)
    // construct ValueSlots directly from native types via the
    // `ValueSlot::from_*` primitives — no ValueWord intermediate, no
    // call to `nb_to_slot` (which is type_schema/mod.rs's slot-
    // construction utility; cleaning that up is N9 territory tracked
    // separately).
    //
    // Json schema (`std::core::json_value`) is looked up at body time
    // via `ctx.schemas.get("Json")` — needed for the FieldType::Any
    // fallback to construct typed Json-enum-tree HeapValues for
    // untypable nested values.
    register_typed_fn_2::<_, Arc<String>, f64>(
        &mut module,
        "__parse_typed",
        "Parse a JSON string into a typed struct",
        [("text", "string"), ("schema_id", "number")],
        ConcreteType::Result(Box::new(ConcreteType::OpaqueTypedObject(
            "any".to_string(),
        ))),
        |text: Arc<String>, schema_id_f: f64, ctx| {
            let schema_id = schema_id_f as u32;

            let parsed: serde_json::Value = serde_json::from_str(text.as_str())
                .map_err(|e| format!("json.__parse_typed() failed: {}", e))?;

            let map = match parsed {
                serde_json::Value::Object(m) => m,
                _ => {
                    return Err("json.__parse_typed() requires a JSON object".to_string());
                }
            };

            let schema = ctx
                .schemas
                .get_by_id(schema_id)
                .ok_or_else(|| format!("json.__parse_typed(): unknown schema id {}", schema_id))?;

            let json_schema = ctx.schemas.get("Json").ok_or_else(|| {
                "json.__parse_typed() requires the `Json` enum schema (load std::core::json_value)"
                    .to_string()
            })?;
            let json_schema_id = json_schema.id as u64;

            let result_hv = build_typed_object_from_json(schema, &map, ctx.schemas, json_schema_id)?;

            Ok(TypedReturn::Ok(ConcreteReturn::OpaqueTypedObject(Arc::new(
                result_hv,
            ))))
        },
    );

    // json.stringify(value: any, pretty?: bool) -> Result<string>
    //
    // Phase 1.B body shim: pre-bulldozer this called
    // `value.to_json_value()` on a `&ValueWord`. Post-ADR-006 the
    // generic value→JSON serializer is the deferred N7 workstream
    // (HeapValue→JSON unified across http/yaml/toml/msgpack/json).
    // Until N7 lands, the body returns an error rather than emit a
    // partial / unsound serializer. Variadic shape preserves the
    // optional `pretty` arg per §2.7.4 ruling.
    register_typed_function(
        &mut module,
        "stringify",
        "Serialize a Shape value to a JSON string",
        vec![
            ModuleParam {
                name: "value".to_string(),
                type_name: "any".to_string(),
                required: true,
                description: "Value to serialize".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "pretty".to_string(),
                type_name: "bool".to_string(),
                required: false,
                description: "Pretty-print with indentation (default: false)".to_string(),
                default_snippet: Some("false".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |args, _ctx| {
            let _value = args
                .first()
                .ok_or_else(|| "json.stringify() requires a value argument".to_string())?;
            let _pretty = args.get(1).map(|a| a.slot().as_bool()).unwrap_or(false);
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "json.stringify() pending N7 (HeapValue→JSON) — see ADR-006 §2.7.4".to_string(),
            )))
        },
    );

    // json.is_valid(text: string) -> bool
    //
    // Phase 1.B body shim: variadic args carry `KindedSlot` placeholders
    // (see `marshal.rs` register_typed_function — kind threading lands
    // in Phase 2c). Read the first slot as a `String` Arc per the
    // declared `string` param contract.
    register_typed_function(
        &mut module,
        "is_valid",
        "Check if a string is valid JSON",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "String to validate as JSON".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        |args, _ctx| {
            let slot = args
                .first()
                .ok_or_else(|| "json.is_valid() requires a string argument".to_string())?;
            let text = slot_as_string(slot)
                .ok_or_else(|| "json.is_valid() requires a string argument".to_string())?;
            let valid = serde_json::from_str::<serde_json::Value>(text.as_str()).is_ok();
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(valid)))
        },
    );

    module
}

/// Read a [`KindedSlot`]'s bits as an `Arc<String>` payload. Used by
/// Phase 1.B variadic body shims that have been migrated off
/// `ValueWord::as_str()`. Phase 2c lands proper per-position kind
/// threading; until then, variadic bodies interpret slots per their
/// declared `ModuleParam` contract.
fn slot_as_string(slot: &KindedSlot) -> Option<Arc<String>> {
    let bits = slot.slot().raw();
    if bits == 0 {
        return None;
    }
    // SAFETY: variadic-arg slots whose registered param type is `string`
    // store an `Arc<String>::into_raw` pointer (matching
    // `ValueSlot::from_string_arc`). Reconstitute without consuming the
    // slot's strong-count share by `from_raw` + `increment_strong_count`
    // semantics — i.e. `Arc::clone` of a `from_raw`-rebuilt handle and
    // forget the rebuilt one.
    unsafe {
        let arc = Arc::<String>::from_raw(bits as *const String);
        let cloned = arc.clone();
        std::mem::forget(arc);
        Some(cloned)
    }
}


// Tests deleted along with the legacy ValueWord-based fixtures, mirroring
// the csv/http/xml migrations. The test infrastructure (`invoke_export`,
// `&[ValueWord]` arg arrays, `as_ok_inner`/`extract_enum_variant`
// helpers) all relied on the pre-bulldozer ValueWord API which no
// longer exists. New typed-marshal test harness arrives with the
// shape-vm cleanup workstream.
