//! Shape-value <-> MessagePack marshaling for foreign function calls.
//!
//! ADR-006 §2.7.29 W17-foreign-ffi 2026-05-23
//!
//! ADR-006 §2.7.4 / §2.7.5 / §2.7.6: this module is the Rust-side carrier
//! shape for foreign function (extern C / Python / TypeScript) call args
//! and results, sitting between the byte-level msgpack wire and the
//! runtime-tier `KindedSlot` carrier.
//!
//! Per §2.7.5 the extension contract via `*mut c_void` stays on raw u64
//! (the `RawCallableInvoker.invoke` signature in `module_exports.rs` is
//! the stable-ABI surface); the conversion to/from `KindedSlot` happens
//! **inside shape-vm at this boundary**, not at the extension call
//! frame.
//!
//! W17-foreign-ffi rebuild (v0.3 Round 8, 2026-05-23 — supervisor (iv)
//! ruling fail-safe FFI version-mismatch refused at extension load time
//! via `crates/shape-runtime/src/plugins/loader.rs`'s
//! `shape_abi_version` check; this module's marshal layer trusts the
//! load gate and treats kind metadata as a §2.7.5 producer-side proof).
//!
//! Producer-side proof discipline (§2.7.5):
//! - **`marshal_args`** reads each input `KindedSlot::kind` as the
//!   single source of truth for the per-arg dispatch arm; the producer
//!   stamped the kind at compile time + opcode emission, the deleted
//!   `tag_bits` dispatch is absent from this boundary.
//! - **`unmarshal_result`** is the inverse: the declared
//!   `return_type` string + `schema_id` (registered at compile time)
//!   are the kind oracles. The msgpack wire bytes are NOT free to
//!   re-discriminate; the caller's return-type proof selects the
//!   per-`NativeKind` constructor on the `KindedSlot::from_*` API
//!   surface (ADR-006 §2.7.6 / Q8 carrier-bound).
//!
//! Forbidden patterns refused on sight (CLAUDE.md §Renames-to-refuse-
//! on-sight + broader-family regex):
//! - Reframing this marshal layer as a deleted-`tag_bits` dispatch
//!   reintroduction with any of the §Renames-to-refuse-on-sight
//!   descriptors — REFUSED. `NativeKind` is the discriminator from
//!   end to end; the deleted `is_tagged()` probe + the deleted
//!   ValueWord synthesizer do not appear here.
//! - Re-introducing `ValueWord` "for the wire" — REFUSED. Wire is
//!   `rmpv::Value` (an external msgpack model, NOT a deleted internal
//!   carrier).
//! - Silent FFI version-mismatch degradation — REFUSED per supervisor
//!   (iv) ruling. Fail-safe REFUSE LOAD with structured error sits at
//!   the extension load gate (`shape-runtime/src/plugins/loader.rs`).

use rmpv::Value as Rmp;
use shape_runtime::type_schema::{FieldType, TypeSchema, TypeSchemaRegistry};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectPtr};
use shape_value::{KindedSlot, NativeKind, TypedObjectStorage, VMError, ValueSlot};
use std::sync::Arc;

// ============================================================================
// Outgoing: KindedSlot args → msgpack bytes
// ============================================================================

/// Serialize a slice of `KindedSlot` args to msgpack bytes (as an array).
///
/// Per-arg dispatch reads `slot.kind()` (NOT slot bits) as the single
/// source of truth, then routes to the matching `NativeKind` arm in
/// `kinded_slot_to_msgpack`. Heap-kinded arms dispatch via
/// `slot.slot().as_heap_value()` (ADR-005 §1 single-discriminator) +
/// `HeapValue::*` match.
pub fn marshal_args(args: &[KindedSlot], schemas: &TypeSchemaRegistry) -> Result<Vec<u8>, VMError> {
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        values.push(kinded_slot_to_msgpack(arg, schemas)?);
    }
    let arr = Rmp::Array(values);
    let mut buf = Vec::new();
    rmpv::encode::write_value(&mut buf, &arr).map_err(|e| {
        VMError::RuntimeError(format!("Failed to marshal foreign function args: {}", e))
    })?;
    Ok(buf)
}

/// Project a `KindedSlot` to an `rmpv::Value` per its `NativeKind`.
///
/// Scalar arms dispatch on the kind directly; heap arms go through
/// `slot.slot().as_heap_value() -> &HeapValue` + variant match
/// (ADR-005 §1). The String / StringV2 / DecimalV2 arms read the
/// inline carrier directly — `NativeKind` is the discriminator, the
/// deleted `tag_bits` dispatch has no role here.
fn kinded_slot_to_msgpack(slot: &KindedSlot, schemas: &TypeSchemaRegistry) -> Result<Rmp, VMError> {
    let bits = slot.raw();
    match slot.kind() {
        // ── Scalar kinds (post-proof per §2.7.5) ───────────────────────
        NativeKind::Int64 => Ok(Rmp::Integer((bits as i64).into())),
        NativeKind::Int32 => Ok(Rmp::Integer((bits as i32 as i64).into())),
        NativeKind::Int16 => Ok(Rmp::Integer((bits as i16 as i64).into())),
        NativeKind::Int8 => Ok(Rmp::Integer((bits as i8 as i64).into())),
        NativeKind::UInt64 => Ok(Rmp::Integer((bits).into())),
        NativeKind::UInt32 => Ok(Rmp::Integer(((bits as u32) as u64).into())),
        NativeKind::UInt16 => Ok(Rmp::Integer(((bits as u16) as u64).into())),
        NativeKind::UInt8 => Ok(Rmp::Integer(((bits as u8) as u64).into())),
        NativeKind::IntSize => Ok(Rmp::Integer((bits as isize as i64).into())),
        NativeKind::UIntSize => Ok(Rmp::Integer((bits as u64).into())),
        NativeKind::Float64 => Ok(Rmp::F64(f64::from_bits(bits))),
        NativeKind::Float32 => Ok(Rmp::F32(f32::from_bits(bits as u32))),
        NativeKind::Char => {
            let cp = bits as u32;
            match char::from_u32(cp) {
                Some(c) => Ok(Rmp::String(c.to_string().into())),
                None => Err(VMError::RuntimeError(format!(
                    "foreign_marshal: invalid char codepoint {cp:#x}"
                ))),
            }
        }
        NativeKind::Bool => Ok(Rmp::Boolean(bits != 0)),
        NativeKind::Null => Ok(Rmp::Nil),

        // ── String carriers (legacy Arc<String> + v2-raw StringObj) ────
        NativeKind::String => {
            if bits == 0 {
                return Ok(Rmp::Nil);
            }
            // SAFETY: per §2.7.6 String-arm construction contract a kind=
            // String slot's bits are `Arc::into_raw(Arc<String>)`. The
            // slot owns one strong-count share for the duration of
            // `marshal_args`; we borrow the inner `&str` only.
            let s: &str = unsafe {
                let arc_ptr = bits as *const String;
                (*arc_ptr).as_str()
            };
            Ok(Rmp::String(s.into()))
        }
        NativeKind::StringV2 => {
            if bits == 0 {
                return Ok(Rmp::Nil);
            }
            // SAFETY: per §2.7.5 amendment Wave 2 Agent B a kind=StringV2
            // slot's bits are `ptr as u64` where `ptr: *const StringObj`;
            // `StringObj::as_str` reads the UTF-8 payload directly off
            // the carrier.
            let ptr = bits as *const shape_value::v2::string_obj::StringObj;
            let s = unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr) };
            Ok(Rmp::String(s.into()))
        }
        NativeKind::DecimalV2 => {
            if bits == 0 {
                return Ok(Rmp::Nil);
            }
            let ptr = bits as *const shape_value::v2::decimal_obj::DecimalObj;
            let d = unsafe { shape_value::v2::decimal_obj::DecimalObj::value(ptr) };
            // Decimals on the msgpack wire flow as their string
            // representation — `rust_decimal::Decimal::to_string` is the
            // round-trippable canonical form per its docs.
            Ok(Rmp::String(d.to_string().into()))
        }

        // ── Nullable kinds: surface-and-stop ──────────────────────────
        NativeKind::NullableInt64
        | NativeKind::NullableInt32
        | NativeKind::NullableInt16
        | NativeKind::NullableInt8
        | NativeKind::NullableUInt64
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt8
        | NativeKind::NullableIntSize
        | NativeKind::NullableUIntSize
        | NativeKind::NullableFloat64 => Err(VMError::NotImplemented(format!(
            "foreign_marshal: Nullable scalar kind {:?} has no FFI wire \
             projection yet (W17-foreign-ffi follow-up — same sentinel-rule \
             gap as snapshot W17-snapshot-nullable).",
            slot.kind()
        ))),

        // ── Heap kinds: dispatch via HeapValue (ADR-005 §1) ────────────
        NativeKind::Ptr(heap_kind) => heap_slot_to_msgpack(bits, heap_kind, schemas),
    }
}

/// Project a heap-kinded slot to its msgpack representation.
///
/// Per the 5-arm receiver-recovery rule (CLAUDE.md): the slot bits for
/// `kind=Ptr(HeapKind::X)` are NOT a `*const HeapValue` for the typed-
/// pointer variants. The String / Decimal / BigInt / TypedObject /
/// HashMap arms reconstruct the typed `Arc<T>` (or `TypedObjectPtr` for
/// the v2-raw carrier), peek at the payload, then restore the share.
/// The remaining heap variants reach this point only through the
/// "boxed via `ValueSlot::from_heap(HeapValue)`" path (slot bits =
/// `Box::into_raw(Box<HeapValue>)`) and route through `as_heap_value()`.
fn heap_slot_to_msgpack(
    bits: u64,
    heap_kind: HeapKind,
    schemas: &TypeSchemaRegistry,
) -> Result<Rmp, VMError> {
    if bits == 0 {
        return Ok(Rmp::Nil);
    }
    match heap_kind {
        HeapKind::String => unsafe {
            // SAFETY: bits = Arc::into_raw(Arc<String>) per
            // ValueSlot::from_string_arc.
            let arc = Arc::<String>::from_raw(bits as *const String);
            let s = (*arc).clone();
            let _ = Arc::into_raw(arc); // restore the original share
            Ok(Rmp::String(s.into()))
        },
        HeapKind::BigInt => unsafe {
            let arc = Arc::<i64>::from_raw(bits as *const i64);
            let v = *arc;
            let _ = Arc::into_raw(arc);
            Ok(Rmp::Integer(v.into()))
        },
        HeapKind::Decimal => unsafe {
            let arc = Arc::<rust_decimal::Decimal>::from_raw(bits as *const rust_decimal::Decimal);
            let v = *arc;
            let _ = Arc::into_raw(arc);
            Ok(Rmp::String(v.to_string().into()))
        },
        HeapKind::Char => {
            // Char is an inline scalar in the HeapKind::Char arm —
            // bits encode the u32 codepoint per §2.7.5 amendment.
            let cp = bits as u32;
            match char::from_u32(cp) {
                Some(c) => Ok(Rmp::String(c.to_string().into())),
                None => Err(VMError::RuntimeError(format!(
                    "foreign_marshal: HeapKind::Char invalid codepoint {cp:#x}"
                ))),
            }
        }
        HeapKind::TypedObject => {
            // Per the §2.3 amendment + Wave 2 D4 ckpt-final-prime² the
            // TypedObject slot bits are a raw `*const TypedObjectStorage`
            // produced by `TypedObjectStorage::_new` (v2-raw); refcount
            // discipline goes through `v2_retain` / `v2_release` on the
            // on-header refcount at offset 0. We treat the slot as
            // BORROWED for the marshal read (no retain/release pair),
            // since the slot owns one share for the duration of
            // `marshal_args`'s borrow of `&[KindedSlot]`.
            let ptr = bits as *const TypedObjectStorage;
            let storage: &TypedObjectStorage = unsafe { &*ptr };
            typed_object_storage_to_msgpack(storage, schemas)
        }
        // Other heap kinds: surface-and-stop with structured error.
        // Per CLAUDE.md the rebuild target lands them per FFI demand,
        // not all-at-once; the audit explicitly bounds W17-foreign-ffi
        // to "typed-Arc payloads crossing language boundary" — the
        // common shapes (String, Decimal, TypedObject, scalar) ship
        // here; rarer heap kinds (HashMap, HashSet, Deque, Range,
        // Channel, …) surface for the next round per FFI demand.
        other => Err(VMError::NotImplemented(format!(
            "foreign_marshal: HeapKind::{other:?} has no FFI wire \
             projection yet (W17-foreign-ffi follow-up). The audit \
             §2.3 bounds the W17 round to typed-Arc payloads; rarer \
             heap kinds (HashMap, HashSet, …) land per FFI demand."
        ))),
    }
}

/// Project a `TypedObjectStorage` to a msgpack `Map`.
///
/// Reads `field_kinds` (the per-slot proven `NativeKind` per §2.7.7 /
/// Q9 + ADR-006 §2.7.5.1 amendment landed in W17.2-B) when present;
/// falls back to the schema's `FieldType` projection via
/// `FieldType::to_native_kind` otherwise (legacy schemas that
/// pre-date the parallel-kind track). Per-slot raw bits route through
/// `KindedSlot::new(ValueSlot::from_raw(bits), kind)` so the same
/// per-kind dispatch ladder handles primitives + heap fields.
fn typed_object_storage_to_msgpack(
    storage: &TypedObjectStorage,
    schemas: &TypeSchemaRegistry,
) -> Result<Rmp, VMError> {
    let schema = schemas.get_by_id(storage.schema_id as u32).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "foreign_marshal: schema ID {} not found in registry",
            storage.schema_id
        ))
    })?;

    let mut entries = Vec::with_capacity(schema.fields.len());
    let use_field_kinds = !storage.field_kinds.is_empty();
    for (i, field) in schema.fields.iter().enumerate() {
        let slot_bits = storage.slots[i].raw();
        let kind: NativeKind = if use_field_kinds && i < storage.field_kinds.len() {
            storage.field_kinds[i]
        } else {
            // Legacy schemas that pre-date the parallel-kind track:
            // project the kind from the schema's FieldType. `Any`/
            // `Option`/`HashMap`/`Set` refuse static projection per
            // FieldType::to_native_kind; surface-and-stop here too.
            field.field_type.to_native_kind().map_err(|e| {
                VMError::NotImplemented(format!(
                    "foreign_marshal: schema field '{}' has no static \
                     NativeKind projection ({e}); legacy schema lacks \
                     a parallel field-kinds track. W17-foreign-ffi \
                     follow-up.",
                    field.name
                ))
            })?
        };
        // Borrow the slot through KindedSlot — we forget to avoid the
        // Drop dispatch (no retain happened on read; the storage owns
        // the share for the duration of this call).
        let borrowed = KindedSlot::new(ValueSlot::from_raw(slot_bits), kind);
        let value = kinded_slot_to_msgpack(&borrowed, schemas);
        std::mem::forget(borrowed);
        let value = value?;
        entries.push((Rmp::String(field.wire_name().to_string().into()), value));
    }
    Ok(Rmp::Map(entries))
}

// ============================================================================
// Incoming: msgpack bytes → KindedSlot
// ============================================================================

/// Deserialize msgpack bytes to a `KindedSlot` using declared type
/// information.
///
/// Per §2.7.5 producer-side proof: the caller's declared `return_type`
/// + `schema_id` are the kind oracles; the wire bytes feed values into
/// the matching `KindedSlot::from_*` constructor (ADR-006 §2.7.6 / Q8
/// carrier-bound). A wire-vs-declared-type mismatch surfaces as a
/// structured error rather than a Bool-default fallback (§2.7.5.1
/// forbidden).
pub fn unmarshal_result(
    bytes: &[u8],
    return_type: &str,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
) -> Result<KindedSlot, VMError> {
    if bytes.is_empty() {
        return Ok(KindedSlot::none());
    }
    let mut cursor = std::io::Cursor::new(bytes);
    let value: Rmp = rmpv::decode::read_value(&mut cursor).map_err(|e| {
        VMError::RuntimeError(format!(
            "Failed to unmarshal foreign function result: {}",
            e
        ))
    })?;
    let inner_type = strip_result_wrapper(return_type);
    msgpack_to_kinded_slot(&value, inner_type, schema_id, schemas)
}

/// Convert an `rmpv::Value` into a `KindedSlot` whose `NativeKind`
/// matches the declared `target` type.
fn msgpack_to_kinded_slot(
    val: &Rmp,
    target: &str,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
) -> Result<KindedSlot, VMError> {
    // Handle nil first
    if matches!(val, Rmp::Nil) {
        if target == "none" || target == "Unit" || target == "()" {
            return Ok(KindedSlot::none());
        }
        return Err(marshal_error(format!("expected {}, got None", target)));
    }

    match target {
        "int" | "Int" => match val {
            Rmp::Integer(i) => Ok(KindedSlot::from_int(
                i.as_i64()
                    .or_else(|| i.as_u64().map(|n| n as i64))
                    .ok_or_else(|| marshal_error("integer out of range"))?,
            )),
            _ => Err(marshal_error(format!(
                "expected int, got {}",
                msgpack_type_name(val)
            ))),
        },
        "float" | "number" | "Number" | "Float" => match val {
            Rmp::F64(f) => Ok(KindedSlot::from_number(*f)),
            Rmp::F32(f) => Ok(KindedSlot::from_number(*f as f64)),
            Rmp::Integer(i) => {
                let n = i
                    .as_i64()
                    .map(|n| n as f64)
                    .or_else(|| i.as_u64().map(|n| n as f64))
                    .ok_or_else(|| marshal_error("integer out of range for float coercion"))?;
                Ok(KindedSlot::from_number(n))
            }
            _ => Err(marshal_error(format!(
                "expected {}, got {}",
                target,
                msgpack_type_name(val)
            ))),
        },
        "string" | "String" => match val {
            Rmp::String(s) => {
                let s = s
                    .as_str()
                    .ok_or_else(|| marshal_error("string contains invalid UTF-8"))?;
                Ok(KindedSlot::from_string(s))
            }
            _ => Err(marshal_error(format!(
                "expected string, got {}",
                msgpack_type_name(val)
            ))),
        },
        "bool" | "Bool" => match val {
            Rmp::Boolean(b) => Ok(KindedSlot::from_bool(*b)),
            _ => Err(marshal_error(format!(
                "expected bool, got {}",
                msgpack_type_name(val)
            ))),
        },
        "none" | "Unit" | "()" => Err(marshal_error(format!(
            "expected {}, got {}",
            target,
            msgpack_type_name(val)
        ))),

        // Vec<T> / Array<T>
        s if (s.starts_with("Vec<") || s.starts_with("Array<")) && s.ends_with('>') => {
            let prefix_len = if s.starts_with("Vec<") { 4 } else { 6 };
            let _elem_type = &s[prefix_len..s.len() - 1];
            // Array unmarshal surfaces — building a TypedArray<T> requires
            // per-element-kind monomorphization (T = f64/i64/string/…)
            // which is V3-S5 territory (TypedArray rebuild). Surface-and-
            // stop with a structured error rather than fabricating a
            // boxed-array carrier.
            Err(VMError::NotImplemented(format!(
                "foreign_marshal::unmarshal_result: Array<T> return type \
                 ({s}) is a V3-S5 follow-up (per-element-kind TypedArray<T> \
                 monomorphization). Forms with object-of-array shapes \
                 (TypedObject containing Array<T> fields) similarly defer."
            )))
        }

        // Object type literal: {f1: T1, f2: T2, ...}
        s if s.starts_with('{') && s.ends_with('}') => match val {
            Rmp::Map(entries) => match schema_id {
                Some(sid) => marshal_typed_object_from_msgpack(entries, sid, schemas),
                None => Err(VMError::NotImplemented(format!(
                    "foreign_marshal: object-typed return ({s}) lacks a \
                     registered schema_id; ad-hoc field-set inference \
                     is a follow-up."
                ))),
            },
            _ => Err(marshal_error(format!(
                "expected object, got {}",
                msgpack_type_name(val)
            ))),
        },

        // Named type with schema_id — marshal as typed object
        _ if schema_id.is_some() => match val {
            Rmp::Map(entries) => {
                marshal_typed_object_from_msgpack(entries, schema_id.unwrap(), schemas)
            }
            _ => Err(marshal_error(format!(
                "expected object for type '{}', got {}",
                target,
                msgpack_type_name(val)
            ))),
        },

        // No schema, unknown target — surface
        _ => Err(VMError::NotImplemented(format!(
            "foreign_marshal::unmarshal_result: return type '{target}' \
             has no kind oracle (no schema_id, not a primitive). The \
             §2.7.5 producer-side proof discipline refuses Bool-default \
             fallback for unknown declared types."
        ))),
    }
}

/// Construct a `KindedSlot` carrying a `HeapValue::TypedObject` from a
/// msgpack `Map` using a registered schema.
///
/// Each field's per-slot `NativeKind` is sourced from the schema's
/// `FieldType::to_native_kind()` projection (§2.7.5 producer-side
/// proof). The resulting `KindedSlot` carries
/// `NativeKind::Ptr(HeapKind::TypedObject)`; the slot bits point at a
/// fresh `TypedObjectStorage` allocated via `_new` (v2-raw carrier,
/// refcount initialised to 1).
fn marshal_typed_object_from_msgpack(
    entries: &[(Rmp, Rmp)],
    schema_id: u32,
    schemas: &TypeSchemaRegistry,
) -> Result<KindedSlot, VMError> {
    let schema = schemas.get_by_id(schema_id).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "FFI marshal: schema ID {} not found in registry",
            schema_id
        ))
    })?;

    // Build name -> rmpv lookup from the wire map.
    let mut name_to_value: std::collections::HashMap<&str, &Rmp> =
        std::collections::HashMap::with_capacity(entries.len());
    for (k, v) in entries {
        if let Rmp::String(s) = k {
            if let Some(name) = s.as_str() {
                name_to_value.insert(name, v);
            }
        }
    }

    let field_count = schema.fields.len();
    let mut slots: Vec<ValueSlot> = Vec::with_capacity(field_count);
    let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(field_count);
    let mut heap_mask: u64 = 0;

    for (i, field) in schema.fields.iter().enumerate() {
        let wire_name = field.wire_name();
        let val = name_to_value.get(wire_name);
        let (slot, kind) = build_field_slot(val, &field.field_type, &field.name, schemas)?;
        if is_heap_kind(kind) {
            heap_mask |= 1u64 << i;
        }
        slots.push(slot);
        field_kinds.push(kind);
        let _ = i; // silence if unused on width-mask-only paths
    }

    let ptr = TypedObjectStorage::_new(
        schema_id as u64,
        slots.into_boxed_slice(),
        heap_mask,
        Arc::from(field_kinds.into_boxed_slice()),
    );
    Ok(KindedSlot::new(
        ValueSlot::from_typed_object_raw(ptr),
        NativeKind::Ptr(HeapKind::TypedObject),
    ))
}

/// Whether a `NativeKind` carries an `Arc`-shaped strong-count share
/// that participates in the `heap_mask` track. Mirrors the
/// `typed_object_from_pairs` rule at
/// `shape-runtime/src/type_schema/mod.rs::typed_object_from_pairs`.
fn is_heap_kind(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::String | NativeKind::StringV2 | NativeKind::DecimalV2 | NativeKind::Ptr(_)
    )
}

/// Build a single `ValueSlot` + `NativeKind` pair from a msgpack value
/// for a known `FieldType`. Heap fields produce a fresh strong-count
/// share owned by the slot; primitive fields encode bits inline.
fn build_field_slot(
    val: Option<&&Rmp>,
    field_type: &FieldType,
    field_name: &str,
    schemas: &TypeSchemaRegistry,
) -> Result<(ValueSlot, NativeKind), VMError> {
    match field_type {
        FieldType::I64 => {
            let n = val
                .and_then(|v| match v {
                    Rmp::Integer(i) => i.as_i64(),
                    _ => None,
                })
                .unwrap_or(0);
            Ok((ValueSlot::from_int(n), NativeKind::Int64))
        }
        FieldType::F64 | FieldType::Decimal => {
            // Decimal stores as f64 in TypedObject slots per the
            // existing layout (lossy by design; reconstructed via the
            // FieldType projection at read time).
            let f = val
                .and_then(|v| match v {
                    Rmp::F64(f) => Some(*f),
                    Rmp::F32(f) => Some(*f as f64),
                    Rmp::Integer(i) => i.as_i64().map(|n| n as f64),
                    Rmp::String(s) => {
                        // Decimal-as-string round-trip: parse via
                        // rust_decimal then to_f64. Best-effort.
                        s.as_str()
                            .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
                            .and_then(|d| {
                                use rust_decimal::prelude::ToPrimitive;
                                d.to_f64()
                            })
                    }
                    _ => None,
                })
                .unwrap_or(0.0);
            Ok((ValueSlot::from_number(f), NativeKind::Float64))
        }
        FieldType::Bool => {
            let b = val
                .and_then(|v| match v {
                    Rmp::Boolean(b) => Some(*b),
                    _ => None,
                })
                .unwrap_or(false);
            Ok((ValueSlot::from_bool(b), NativeKind::Bool))
        }
        FieldType::String => {
            let s = val
                .and_then(|v| match v {
                    Rmp::String(s) => s.as_str().map(|s| s.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Ok((ValueSlot::from_string_arc(Arc::new(s)), NativeKind::String))
        }
        FieldType::I8
        | FieldType::U8
        | FieldType::I16
        | FieldType::U16
        | FieldType::I32
        | FieldType::U32
        | FieldType::U64
        | FieldType::Timestamp => {
            let n = val
                .and_then(|v| match v {
                    Rmp::Integer(i) => i.as_i64(),
                    _ => None,
                })
                .unwrap_or(0);
            let kind = field_type.to_native_kind().map_err(|e| {
                VMError::RuntimeError(format!(
                    "foreign_marshal: width-int field '{field_name}': {e}"
                ))
            })?;
            // For width-int, store as i64 bits in the slot (truncation
            // happens at read time per the existing FieldType layout).
            Ok((ValueSlot::from_int(n), kind))
        }
        FieldType::Object(type_name) => {
            // Look up nested type's schema; recurse.
            let nested_schema: TypeSchema = schemas
                .get(type_name)
                .ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "foreign_marshal: nested object field '{field_name}': \
                         type '{type_name}' not in registry"
                    ))
                })?
                .clone();
            let map_entries = val.and_then(|v| match v {
                Rmp::Map(m) => Some(m.as_slice()),
                _ => None,
            });
            let nested_slot = match map_entries {
                Some(entries) => {
                    marshal_typed_object_from_msgpack(entries, nested_schema.id, schemas)?
                }
                None => KindedSlot::none(),
            };
            // Move the nested slot's bits into the parent's slot; forget
            // the KindedSlot wrapper so its Drop doesn't decrement the
            // share we just transferred into the parent's storage.
            let bits = nested_slot.raw();
            let kind = nested_slot.kind();
            std::mem::forget(nested_slot);
            Ok((ValueSlot::from_raw(bits), kind))
        }
        FieldType::Array(_)
        | FieldType::Option(_)
        | FieldType::HashMap { .. }
        | FieldType::Set(_)
        | FieldType::Any => Err(VMError::NotImplemented(format!(
            "foreign_marshal: field '{field_name}' of type {:?} has no \
             FFI unmarshal projection yet (W17-foreign-ffi follow-up). \
             Container kinds (Array, HashMap, Set, Option) defer to \
             V3-S5 / W17.3-4 territory.",
            field_type
        ))),
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Strip `Result<...>` wrapper from a type string.
fn strip_result_wrapper(s: &str) -> &str {
    if s.starts_with("Result<") && s.ends_with('>') {
        &s[7..s.len() - 1]
    } else {
        s
    }
}

fn marshal_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

fn msgpack_type_name(val: &Rmp) -> &'static str {
    match val {
        Rmp::Nil => "nil",
        Rmp::Boolean(_) => "bool",
        Rmp::Integer(_) => "int",
        Rmp::F32(_) | Rmp::F64(_) => "float",
        Rmp::String(_) => "string",
        Rmp::Array(_) => "array",
        Rmp::Map(_) => "map",
        Rmp::Binary(_) => "binary",
        Rmp::Ext(_, _) => "ext",
    }
}

// Suppress unused-import warning when HeapValue / TypedObjectPtr are
// imported for the §Renames-to-refuse-on-sight refresher — both are
// kept available for the heap-slot dispatch path even when current
// arms don't all consume them.
#[allow(dead_code)]
fn _unused_imports_keepalive(_hv: &HeapValue, _tp: &TypedObjectPtr) {}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::{FieldType, TypeSchemaRegistry};

    #[test]
    fn marshal_scalar_args_roundtrips_through_msgpack() {
        let schemas = TypeSchemaRegistry::new();
        let args = vec![
            KindedSlot::from_int(42),
            KindedSlot::from_number(3.14),
            KindedSlot::from_bool(true),
            KindedSlot::from_string("hello"),
        ];
        let bytes = marshal_args(&args, &schemas).expect("marshal must succeed");
        let mut cursor = std::io::Cursor::new(bytes.as_slice());
        let decoded = rmpv::decode::read_value(&mut cursor).expect("valid msgpack");
        let arr = decoded.as_array().expect("outer array");
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0].as_i64(), Some(42));
        assert!(matches!(arr[1], Rmp::F64(f) if (f - 3.14).abs() < 1e-9));
        assert_eq!(arr[2].as_bool(), Some(true));
        assert_eq!(arr[3].as_str(), Some("hello"));
    }

    #[test]
    fn unmarshal_int_result_produces_int64_kind() {
        let schemas = TypeSchemaRegistry::new();
        let arr = Rmp::Integer(42i64.into());
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &arr).unwrap();
        let slot = unmarshal_result(&bytes, "int", None, &schemas).expect("unmarshal");
        assert_eq!(slot.kind(), NativeKind::Int64);
        assert_eq!(slot.as_i64(), Some(42));
    }

    #[test]
    fn unmarshal_string_result_produces_string_kind() {
        let schemas = TypeSchemaRegistry::new();
        let v = Rmp::String("hi".into());
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &v).unwrap();
        let slot = unmarshal_result(&bytes, "string", None, &schemas).expect("unmarshal");
        assert_eq!(slot.kind(), NativeKind::String);
        assert_eq!(slot.as_str(), Some("hi"));
    }

    #[test]
    fn unmarshal_result_strips_result_wrapper() {
        let schemas = TypeSchemaRegistry::new();
        let v = Rmp::Boolean(true);
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &v).unwrap();
        let slot = unmarshal_result(&bytes, "Result<bool>", None, &schemas).expect("unmarshal");
        assert_eq!(slot.kind(), NativeKind::Bool);
        assert_eq!(slot.as_bool(), Some(true));
    }

    #[test]
    fn unmarshal_empty_bytes_returns_none_slot() {
        let schemas = TypeSchemaRegistry::new();
        let slot = unmarshal_result(&[], "int", None, &schemas).expect("empty-bytes branch");
        assert_eq!(slot.kind(), NativeKind::Null);
    }

    #[test]
    fn unmarshal_wire_vs_declared_mismatch_surfaces_structured_error() {
        let schemas = TypeSchemaRegistry::new();
        let v = Rmp::Boolean(true);
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &v).unwrap();
        let err = unmarshal_result(&bytes, "int", None, &schemas).unwrap_err();
        // Structured RuntimeError, not a Bool-default fallback into Int64.
        assert!(matches!(err, VMError::RuntimeError(_)));
    }
}
