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

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use crate::executor::result_option_carrier::{build_err, build_ok, build_some};
use rmpv::Value as Rmp;
use shape_abi_v1::foreign_types::{ForeignDirection, ForeignScalar, ForeignType};
use shape_runtime::type_schema::{BuiltinSchemaIds, FieldType, TypeSchema, TypeSchemaRegistry};
use shape_value::heap_value::{HashMapKindedRef, HeapKind, HeapValue, TypedObjectPtr};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{
    ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I64, ELEM_TYPE_STRING, TypedArray, stamp_elem_type,
};
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

/// Serialize args using the declared param types as the compile-time
/// element-kind oracle for scalar-element `Array<T>` / scalar-V
/// `HashMap` args (ffi-rebuild §4.4, Q14). This is the dynamic-language
/// entry point.
///
/// The OUTER dispatch is still on `slot.kind()` (clause 1) — the declared
/// type is used ONLY to recover a container's element `NativeKind`, which
/// is not stored at runtime (the v2-raw `TypedArray<T>` header does not
/// tag the element type per `heap_header.rs`). Scalar args ignore the
/// declared type entirely and route to `kinded_slot_to_msgpack`.
pub fn marshal_args_typed(
    args: &[KindedSlot],
    param_types: &[String],
    schemas: &TypeSchemaRegistry,
) -> Result<Vec<u8>, VMError> {
    let mut values = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let declared = param_types.get(i).map(|s| s.as_str()).unwrap_or("");
        values.push(kinded_slot_to_msgpack_typed(arg, declared, schemas)?);
    }
    let arr = Rmp::Array(values);
    let mut buf = Vec::new();
    rmpv::encode::write_value(&mut buf, &arr).map_err(|e| {
        VMError::RuntimeError(format!("Failed to marshal foreign function args: {}", e))
    })?;
    Ok(buf)
}

fn kinded_slot_to_msgpack_typed(
    slot: &KindedSlot,
    declared: &str,
    schemas: &TypeSchemaRegistry,
) -> Result<Rmp, VMError> {
    match slot.kind() {
        NativeKind::Ptr(HeapKind::TypedArray) => typed_array_to_msgpack(slot.raw(), declared),
        NativeKind::Ptr(HeapKind::HashMap) => hashmap_to_msgpack(slot.raw()),
        // Scalars + already-covered heap kinds (String/TypedObject/…):
        // the declared type is redundant; dispatch on kind alone.
        _ => kinded_slot_to_msgpack(slot, schemas),
    }
}

/// Project a `Ptr(HeapKind::TypedArray)` slot to a msgpack `Array`, using
/// the declared `Array<T>` type to select the element monomorphization
/// (§4.4 — element kind is compile-time-proven, never runtime-tagged).
///
/// The element kind comes from the canonical marshaling table (ADR-019 §1 /
/// #196), so the outbound element set and the declaration-site check cannot
/// drift apart.
fn typed_array_to_msgpack(bits: u64, declared: &str) -> Result<Rmp, VMError> {
    if bits == 0 {
        return Ok(Rmp::Array(Vec::new()));
    }
    let elem = match ForeignType::classify(declared, ForeignDirection::Argument) {
        Ok(ForeignType::Array(elem)) => elem,
        Ok(other) => {
            return Err(VMError::NotImplemented(format!(
                "foreign_marshal: Array arg needs a declared `Array<T>` element \
                 type to select the buffer monomorphization; got '{}'",
                other.shape_spelling()
            )));
        }
        Err(e) => {
            return Err(VMError::NotImplemented(format!(
                "foreign_marshal: Array arg declared '{}' is not in the foreign \
                 marshaling table ('{}' {}).",
                e.declared,
                e.spelling,
                e.reason.explain()
            )));
        }
    };
    // `len` lives at offset 16 of the 24-byte struct — element-independent,
    // so any monomorphization reads it correctly.
    let n = unsafe { TypedArray::<i64>::len(bits as *const TypedArray<i64>) } as usize;
    let mut out = Vec::with_capacity(n);
    match elem {
        ForeignScalar::Int => {
            let s = unsafe { TypedArray::<i64>::as_slice(bits as *const TypedArray<i64>) };
            for v in s {
                out.push(Rmp::Integer((*v).into()));
            }
        }
        ForeignScalar::Number => {
            let s = unsafe { TypedArray::<f64>::as_slice(bits as *const TypedArray<f64>) };
            for v in s {
                out.push(Rmp::F64(*v));
            }
        }
        ForeignScalar::Bool => {
            let s = unsafe { TypedArray::<u8>::as_slice(bits as *const TypedArray<u8>) };
            for v in s {
                out.push(Rmp::Boolean(*v != 0));
            }
        }
        ForeignScalar::String => {
            let ptr = bits as *const TypedArray<*const StringObj>;
            for i in 0..n {
                let sp = unsafe { TypedArray::<*const StringObj>::get_unchecked(ptr, i as u32) };
                let s = unsafe { StringObj::as_str(sp) };
                out.push(Rmp::String(s.into()));
            }
        }
        ForeignScalar::Unit => {
            return Err(VMError::NotImplemented(
                "foreign_marshal: `Array<none>` has no element projection."
                    .to_string(),
            ));
        }
    }
    Ok(Rmp::Array(out))
}

/// Project a `Ptr(HeapKind::HashMap)` slot to a msgpack `Map`. Keys are
/// always `*const StringObj`; the value kind is recovered from the
/// `HashMapKindedRef` variant (monomorphized at construction), so no
/// declared type is needed here. Non-scalar value variants surface-and-stop.
fn hashmap_to_msgpack(bits: u64) -> Result<Rmp, VMError> {
    if bits == 0 {
        return Ok(Rmp::Map(Vec::new()));
    }
    // SAFETY: kind=Ptr(HeapKind::HashMap) slot bits are
    // `Arc::into_raw(Arc<HashMapKindedRef>)`; borrow the ref for the
    // duration of the marshal (the slot owns the share).
    let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };

    // Read the insertion-ordered string keys (element-independent buffer).
    let read_keys = |keys: *const TypedArray<*const StringObj>| -> Vec<&'static str> {
        let n = unsafe { TypedArray::<*const StringObj>::len(keys) } as usize;
        let mut ks = Vec::with_capacity(n);
        for i in 0..n {
            let ptr = unsafe { TypedArray::<*const StringObj>::get_unchecked(keys, i as u32) };
            ks.push(unsafe { StringObj::as_str(ptr) });
        }
        ks
    };

    let mut entries: Vec<(Rmp, Rmp)> = Vec::with_capacity(kref.len());
    unsafe {
        match kref {
            HashMapKindedRef::I64(arc) => {
                let keys = read_keys(arc.keys);
                for (i, k) in keys.iter().enumerate() {
                    let v = *(*arc.values).data.add(i);
                    entries.push((Rmp::String((*k).into()), Rmp::Integer(v.into())));
                }
            }
            HashMapKindedRef::F64(arc) => {
                let keys = read_keys(arc.keys);
                for (i, k) in keys.iter().enumerate() {
                    let v: f64 = *(*arc.values).data.add(i);
                    entries.push((Rmp::String((*k).into()), Rmp::F64(v)));
                }
            }
            HashMapKindedRef::Bool(arc) => {
                let keys = read_keys(arc.keys);
                for (i, k) in keys.iter().enumerate() {
                    let v: u8 = *(*arc.values).data.add(i);
                    entries.push((Rmp::String((*k).into()), Rmp::Boolean(v != 0)));
                }
            }
            HashMapKindedRef::String(arc) => {
                let keys = read_keys(arc.keys);
                for (i, k) in keys.iter().enumerate() {
                    let v_ptr: *const StringObj = *(*arc.values).data.add(i);
                    let s = StringObj::as_str(v_ptr);
                    entries.push((Rmp::String((*k).into()), Rmp::String(s.into())));
                }
            }
            other => {
                return Err(VMError::NotImplemented(format!(
                    "foreign_marshal: HashMap with value kind {:?} has no \
                     scalar-V FFI projection yet (ffi-rebuild §4.4 non-scalar-V \
                     arms are stage-7 territory).",
                    other.values_kind()
                )));
            }
        }
    }
    Ok(Rmp::Map(entries))
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
        unsupported @ (HeapKind::TypedArray
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::HashMap
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice
        | HeapKind::Closure) => Err(VMError::NotImplemented(format!(
            "foreign_marshal: HeapKind::{other:?} has no FFI wire \
             projection yet (W17-foreign-ffi follow-up). The audit \
             §2.3 bounds the W17 round to typed-Arc payloads; rarer \
             heap kinds (HashMap, HashSet, …) land per FFI demand.",
            other = unsupported
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
        let slot_bits = storage.slots()[i].raw();
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
/// forbidden). For a dynamic-language foreign RETURN (the `fn python`
/// / `fn typescript` path), this structured error is not the final
/// user-visible surface: `wrap_dynamic_result` translates it into a
/// class-1 `TypeConformanceError:` `Err` on the caller's `Result<T>`
/// per the Q13/OQ10 ratification (2026-07-05), which amended ADR-006
/// §2.7.29 clause 2 for exactly this sub-case (argument-path / extern-C
/// mismatches keep the `VMError` channel).
pub fn unmarshal_result(
    bytes: &[u8],
    return_type: &str,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
    builtin: &BuiltinSchemaIds,
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
    // ADR-019 §1 / #196: the declared return type is classified against the
    // canonical marshaling table (`shape_abi_v1::foreign_types`) — the same
    // table the compiler rejects unmapped declarations with and the extensions
    // render stubs from. A classification failure here is a host-side gap, not
    // a user error: `compile_foreign_function` refuses the declaration first,
    // so reaching this arm means the two disagreed.
    let declared = ForeignType::classify(return_type, ForeignDirection::Return).map_err(|e| {
        VMError::NotImplemented(format!(
            "foreign_marshal::unmarshal_result: declared return type '{}' is not in \
             the foreign marshaling table ('{}' {}). The declaration-site check \
             ([C0933]) should have refused this signature — reaching the marshal \
             layer means the compiler and the table disagree.",
            e.declared,
            e.spelling,
            e.reason.explain(),
        ))
    })?;
    foreign_type_to_kinded_slot(&value, &declared, schema_id, schemas, builtin)
}

/// Convert an `rmpv::Value` into a `KindedSlot` whose `NativeKind` matches the
/// classified declared type.
///
/// One arm per [`ForeignType`] constructor, matched exhaustively: a new table
/// constructor cannot be added without landing its inbound projection here.
fn foreign_type_to_kinded_slot(
    val: &Rmp,
    declared: &ForeignType,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
    builtin: &BuiltinSchemaIds,
) -> Result<KindedSlot, VMError> {
    let target = declared.shape_spelling();
    let target = target.as_str();
    match declared {
        // `Option<T>`: the declared type is the oracle. Nil → None; any other
        // wire value → unmarshal per T → Some. Selected BEFORE the nil-guard so
        // `Option<int>` returning null builds `None` rather than a conformance
        // error.
        //
        // Representation note: the pattern-matcher's `None` discriminator emits
        // `IsNull` (null-coding), so `None` MUST be the null sentinel
        // (`KindedSlot::none()`) — a non-null `build_none` carrier would read as
        // `Some` under `IsNull` and then fault in `UnwrapOption`. `Some(v)` uses
        // the canonical `build_some` carrier: it is always non-null (so `IsNull`
        // is `false`) and `op_unwrap_option`'s `read_option` path unwraps it
        // correctly even for `Some(0)`/`Some(false)`.
        ForeignType::Optional(inner) => {
            if matches!(val, Rmp::Nil) {
                return Ok(KindedSlot::none());
            }
            let payload = foreign_type_to_kinded_slot(val, inner, schema_id, schemas, builtin)?;
            Ok(build_some(builtin, payload))
        }

        ForeignType::Scalar(ForeignScalar::Unit) => match val {
            Rmp::Nil => Ok(KindedSlot::none()),
            _ => Err(marshal_error(format!(
                "expected {}, got {}",
                target,
                msgpack_type_name(val)
            ))),
        },

        _ if matches!(val, Rmp::Nil) => Err(marshal_error(format!("expected {}, got None", target))),

        ForeignType::Scalar(ForeignScalar::Int) => match val {
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

        ForeignType::Scalar(ForeignScalar::Number) => match val {
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

        ForeignType::Scalar(ForeignScalar::String) => match val {
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

        ForeignType::Scalar(ForeignScalar::Bool) => match val {
            Rmp::Boolean(b) => Ok(KindedSlot::from_bool(*b)),
            _ => Err(marshal_error(format!(
                "expected bool, got {}",
                msgpack_type_name(val)
            ))),
        },

        ForeignType::Array(elem) => match val {
            Rmp::Array(items) => build_scalar_typed_array(items, elem.shape_spelling()),
            _ => Err(marshal_error(format!(
                "expected {}, got {}",
                target,
                msgpack_type_name(val)
            ))),
        },

        // `HashMap<string, V>` is outbound-only; `ForeignType::classify` refuses
        // it in return position, so this arm is unreachable through
        // `unmarshal_result`. It exists because the match is exhaustive — and it
        // states the asymmetry rather than defaulting.
        ForeignType::Map(_) => Err(VMError::NotImplemented(format!(
            "foreign_marshal: '{target}' has no inbound projection — a foreign \
             map can be passed in but not returned."
        ))),

        ForeignType::Object { .. } => match val {
            Rmp::Map(entries) => match schema_id {
                Some(sid) => marshal_typed_object_from_msgpack(entries, sid, schemas),
                None => Err(VMError::NotImplemented(format!(
                    "foreign_marshal: object-typed return ({target}) lacks a \
                     registered schema_id; ad-hoc field-set inference \
                     is a follow-up."
                ))),
            },
            _ => Err(marshal_error(format!(
                "expected object for type '{}', got {}",
                target,
                msgpack_type_name(val)
            ))),
        },
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

// The `Result<...>` / `Option<T>` / `Array<T>` spelling strippers that used to
// live here are gone: `ForeignType::classify` (shape-abi-v1) is the single
// parser for declared foreign type spellings, and every consumer — the
// declaration check, this marshal layer, and the extensions' stub renderers —
// goes through it (ADR-019 §1 / #196).

/// Build a `Ptr(HeapKind::TypedArray)` slot from a msgpack array with a
/// scalar declared element type (ffi-rebuild §4.4, stage 3). Every element
/// is checked against the declared `elem`; the first mismatch surfaces a
/// conformance `RuntimeError` (→ class-1 `TypeConformanceError` Err at the
/// caller). The produced array carries a stamped element-type byte so the
/// generic `release_v2_typed_array` drop frees it correctly.
fn build_scalar_typed_array(items: &[Rmp], elem: &str) -> Result<KindedSlot, VMError> {
    fn array_elem_err(i: usize, want: &str, got: &Rmp) -> VMError {
        marshal_error(format!(
            "Array element [{}]: expected {}, got {}",
            i,
            want,
            msgpack_type_name(got)
        ))
    }
    let ptr_bits: u64 = match elem {
        "int" | "Int" => {
            let mut vals = Vec::with_capacity(items.len());
            for (i, it) in items.iter().enumerate() {
                match it {
                    Rmp::Integer(n) => vals.push(
                        n.as_i64()
                            .or_else(|| n.as_u64().map(|u| u as i64))
                            .ok_or_else(|| array_elem_err(i, "int", it))?,
                    ),
                    _ => return Err(array_elem_err(i, "int", it)),
                }
            }
            let ptr = TypedArray::<i64>::from_slice(&vals);
            unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I64) };
            ptr as u64
        }
        "number" | "Number" | "float" | "Float" => {
            let mut vals = Vec::with_capacity(items.len());
            for (i, it) in items.iter().enumerate() {
                match it {
                    Rmp::F64(f) => vals.push(*f),
                    Rmp::F32(f) => vals.push(*f as f64),
                    Rmp::Integer(n) => vals.push(
                        n.as_i64()
                            .map(|v| v as f64)
                            .or_else(|| n.as_u64().map(|v| v as f64))
                            .ok_or_else(|| array_elem_err(i, "number", it))?,
                    ),
                    _ => return Err(array_elem_err(i, "number", it)),
                }
            }
            let ptr = TypedArray::<f64>::from_slice(&vals);
            unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_F64) };
            ptr as u64
        }
        "bool" | "Bool" => {
            let mut vals = Vec::with_capacity(items.len());
            for (i, it) in items.iter().enumerate() {
                match it {
                    Rmp::Boolean(b) => vals.push(*b as u8),
                    _ => return Err(array_elem_err(i, "bool", it)),
                }
            }
            let ptr = TypedArray::<u8>::from_slice(&vals);
            unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_BOOL) };
            ptr as u64
        }
        "string" | "String" => {
            let mut ptrs: Vec<*const StringObj> = Vec::with_capacity(items.len());
            for (i, it) in items.iter().enumerate() {
                match it {
                    Rmp::String(s) => {
                        let s = s
                            .as_str()
                            .ok_or_else(|| array_elem_err(i, "string", it))?;
                        ptrs.push(StringObj::new(s) as *const StringObj);
                    }
                    _ => return Err(array_elem_err(i, "string", it)),
                }
            }
            let ptr = TypedArray::<*const StringObj>::from_slice(&ptrs);
            unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_STRING) };
            ptr as u64
        }
        other => {
            return Err(VMError::NotImplemented(format!(
                "foreign_marshal::unmarshal_result: Array<{other}> return has no \
                 scalar-element construction arm yet (ffi-rebuild §4.4 non-scalar \
                 element arms are stage-7 territory)."
            )));
        }
    };
    Ok(KindedSlot::new(
        ValueSlot::from_raw(ptr_bits),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// Wrap a dynamic-language foreign invoke outcome into the caller's
/// `Result<T, string>` carrier per the Q13/OQ10 ratified error channel
/// (ffi-rebuild §4.5). Three disjoint outcomes, three visible surfaces:
///
/// * **foreign exception** (`invoke` rc != 0) → class-1 `Err(msg)` on the
///   user's `Result`, carrying the extension-rendered message verbatim
///   (no `TypeConformanceError:` prefix — genuine foreign failures stay
///   distinguishable from contract violations).
/// * **nonconforming return** (wire value ≠ declared type; the host is the
///   single oracle, §4.5 (1b)) → class-1 `Err` whose payload begins with the
///   stable `TypeConformanceError: ` discriminator prefix.
/// * **host marshal-arm gap / wire corruption** (`NotImplemented` or a
///   non-conformance error) → class-2 `VMError`, NEVER dressed as a foreign
///   failure (§4.5 R7) and NEVER folded into the user's `Err`.
pub fn wrap_dynamic_result(
    invoke_outcome: Result<Vec<u8>, String>,
    fn_name: &str,
    language: &str,
    return_type: &str,
    schema_id: Option<u32>,
    schemas: &TypeSchemaRegistry,
    builtin: &BuiltinSchemaIds,
) -> Result<KindedSlot, VMError> {
    match invoke_outcome {
        // (1a) foreign exception → Err, verbatim extension message.
        Err(msg) => Ok(build_err(builtin, KindedSlot::from_string(&msg))),
        Ok(bytes) => match unmarshal_result(&bytes, return_type, schema_id, schemas, builtin) {
            Ok(payload) => Ok(build_ok(builtin, payload)),
            // (1b) conformance violation → class-1 Err with the stable prefix.
            Err(VMError::RuntimeError(detail)) => {
                let msg = format!(
                    "TypeConformanceError: {} (foreign function '{}' ({}), declared \
                     {}); value: {}",
                    detail,
                    fn_name,
                    language,
                    return_type,
                    wire_value_preview(&bytes),
                );
                Ok(build_err(builtin, KindedSlot::from_string(&msg)))
            }
            // Host-side gap / corruption → class-2 VMError (surface-and-stop).
            Err(other) => Err(other),
        },
    }
}

/// Render a short, user-facing preview of the raw wire value for the
/// `TypeConformanceError` message. Never rmpv variant names — Shape-shaped
/// scalars, truncated.
fn wire_value_preview(bytes: &[u8]) -> String {
    let mut cursor = std::io::Cursor::new(bytes);
    let v = match rmpv::decode::read_value(&mut cursor) {
        Ok(v) => v,
        Err(_) => return "<unreadable>".to_string(),
    };
    let s = match &v {
        Rmp::String(x) => format!("{:?}", x.as_str().unwrap_or("<binary>")),
        Rmp::Integer(i) => format!("{}", i),
        Rmp::F32(f) => format!("{}", f),
        Rmp::F64(f) => format!("{}", f),
        Rmp::Boolean(b) => format!("{}", b),
        Rmp::Nil => "null".to_string(),
        Rmp::Array(_) => "[...]".to_string(),
        Rmp::Map(_) => "{...}".to_string(),
        Rmp::Binary(_) => "<binary>".to_string(),
        Rmp::Ext(_, _) => "<ext>".to_string(),
    };
    if s.chars().count() > 60 {
        format!("{}...", s.chars().take(60).collect::<String>())
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
fn _unused_imports_keepalive(_hv: &HeapValue, _tp: &TypedObjectPtr) {}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::builtin_schemas::register_builtin_schemas;
    use shape_runtime::type_schema::registry::TypeSchemaRegistry;

    fn schemas_with_builtins() -> (TypeSchemaRegistry, BuiltinSchemaIds) {
        let mut registry = TypeSchemaRegistry::new();
        let ids = register_builtin_schemas(&mut registry);
        (registry, ids)
    }

    fn encode(v: &Rmp) -> Vec<u8> {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, v).unwrap();
        bytes
    }

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
        let (schemas, builtin) = schemas_with_builtins();
        let bytes = encode(&Rmp::Integer(42i64.into()));
        let slot = unmarshal_result(&bytes, "int", None, &schemas, &builtin).expect("unmarshal");
        assert_eq!(slot.kind(), NativeKind::Int64);
        assert_eq!(slot.as_i64(), Some(42));
    }

    #[test]
    fn unmarshal_string_result_produces_string_kind() {
        let (schemas, builtin) = schemas_with_builtins();
        let bytes = encode(&Rmp::String("hi".into()));
        let slot = unmarshal_result(&bytes, "string", None, &schemas, &builtin).expect("unmarshal");
        assert_eq!(slot.kind(), NativeKind::String);
        assert_eq!(slot.as_str(), Some("hi"));
    }

    #[test]
    fn unmarshal_result_strips_result_wrapper() {
        let (schemas, builtin) = schemas_with_builtins();
        let bytes = encode(&Rmp::Boolean(true));
        let slot =
            unmarshal_result(&bytes, "Result<bool>", None, &schemas, &builtin).expect("unmarshal");
        assert_eq!(slot.kind(), NativeKind::Bool);
        assert_eq!(slot.as_bool(), Some(true));
    }

    #[test]
    fn unmarshal_empty_bytes_returns_none_slot() {
        let (schemas, builtin) = schemas_with_builtins();
        let slot =
            unmarshal_result(&[], "int", None, &schemas, &builtin).expect("empty-bytes branch");
        assert_eq!(slot.kind(), NativeKind::Null);
    }

    #[test]
    fn unmarshal_wire_vs_declared_mismatch_surfaces_structured_error() {
        let (schemas, builtin) = schemas_with_builtins();
        let bytes = encode(&Rmp::Boolean(true));
        let err = unmarshal_result(&bytes, "int", None, &schemas, &builtin).unwrap_err();
        // Structured RuntimeError, not a Bool-default fallback into Int64.
        assert!(matches!(err, VMError::RuntimeError(_)));
    }

    #[test]
    fn unmarshal_option_some_and_none() {
        let (schemas, builtin) = schemas_with_builtins();
        // Some(7) → canonical non-null `build_some` carrier (TypedObject).
        let bytes = encode(&Rmp::Integer(7i64.into()));
        let some = unmarshal_result(&bytes, "Option<int>", None, &schemas, &builtin).unwrap();
        assert_eq!(some.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        // None → null sentinel (null-coding, matches the pattern-matcher's
        // `IsNull` discriminator).
        let bytes = encode(&Rmp::Nil);
        let none = unmarshal_result(&bytes, "Option<int>", None, &schemas, &builtin).unwrap();
        assert_eq!(none.kind(), NativeKind::Null);
        drop(some);
        drop(none);
    }

    #[test]
    fn marshal_and_unmarshal_scalar_array_roundtrip() {
        let (schemas, builtin) = schemas_with_builtins();
        // OUTGOING: build an Array<int> slot, marshal it, expect a wire array.
        let arr = TypedArray::<i64>::from_slice(&[1i64, 2, 3]);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
        let slot = KindedSlot::new(
            ValueSlot::from_raw(arr as u64),
            NativeKind::Ptr(HeapKind::TypedArray),
        );
        let bytes = marshal_args_typed(
            std::slice::from_ref(&slot),
            std::slice::from_ref(&"Array<int>".to_string()),
            &schemas,
        )
        .expect("outgoing array marshal");
        drop(slot); // releases the array

        let mut cursor = std::io::Cursor::new(bytes.as_slice());
        let decoded = rmpv::decode::read_value(&mut cursor).unwrap();
        let outer = decoded.as_array().unwrap();
        let inner = outer[0].as_array().unwrap();
        assert_eq!(inner.len(), 3);
        assert_eq!(inner[0].as_i64(), Some(1));

        // INCOMING: unmarshal a wire array into a TypedArray<i64> slot.
        let wire = encode(&Rmp::Array(vec![
            Rmp::Integer(10i64.into()),
            Rmp::Integer(20i64.into()),
        ]));
        let rt = unmarshal_result(&wire, "Array<int>", None, &schemas, &builtin).unwrap();
        assert_eq!(rt.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let n = unsafe { TypedArray::<i64>::len(rt.raw() as *const TypedArray<i64>) };
        assert_eq!(n, 2);
        drop(rt);
    }

    #[test]
    fn wrap_dynamic_result_exception_is_plain_err() {
        let (schemas, builtin) = schemas_with_builtins();
        let outcome = Err("ValueError: boom".to_string());
        let slot =
            wrap_dynamic_result(outcome, "f", "python", "Result<int>", None, &schemas, &builtin)
                .unwrap();
        // Result::Err carrier — a TypedObject.
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        drop(slot);
    }

    #[test]
    fn wrap_dynamic_result_conformance_violation_has_prefix() {
        let (schemas, builtin) = schemas_with_builtins();
        // Python returned a string for a declared Result<int>.
        let bytes = encode(&Rmp::String("str".into()));
        let slot = wrap_dynamic_result(
            Ok(bytes),
            "f",
            "python",
            "Result<int>",
            None,
            &schemas,
            &builtin,
        )
        .unwrap();
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        // Read the Err payload string and assert the discriminator prefix.
        let carrier = read_result(&builtin, &slot).unwrap().unwrap();
        assert!(!carrier.is_ok());
        let payload = carrier.clone_payload().unwrap();
        let msg = payload.as_str().unwrap();
        assert!(
            msg.starts_with("TypeConformanceError: "),
            "got: {msg}"
        );
        assert!(msg.contains("int"));
        drop(payload);
        drop(slot);
    }

    #[test]
    fn wrap_dynamic_result_missing_arm_stays_class2() {
        let (schemas, builtin) = schemas_with_builtins();
        // Declared return type has no marshal arm → NotImplemented, NOT Err.
        let bytes = encode(&Rmp::Integer(1i64.into()));
        let err = wrap_dynamic_result(
            Ok(bytes),
            "f",
            "python",
            "Result<Set<int>>",
            None,
            &schemas,
            &builtin,
        )
        .unwrap_err();
        assert!(matches!(err, VMError::NotImplemented(_)));
    }

    use crate::executor::result_option_carrier::read_result;

    /// A representative wire value for a table entry, plus the `NativeKind` the
    /// inbound projection must produce.
    fn wire_witness(t: &ForeignType) -> (Rmp, NativeKind) {
        match t {
            ForeignType::Scalar(ForeignScalar::Int) => {
                (Rmp::Integer(7i64.into()), NativeKind::Int64)
            }
            ForeignType::Scalar(ForeignScalar::Number) => (Rmp::F64(1.5), NativeKind::Float64),
            ForeignType::Scalar(ForeignScalar::Bool) => (Rmp::Boolean(true), NativeKind::Bool),
            ForeignType::Scalar(ForeignScalar::String) => {
                (Rmp::String("s".into()), NativeKind::String)
            }
            ForeignType::Scalar(ForeignScalar::Unit) => (Rmp::Nil, NativeKind::Null),
            ForeignType::Array(elem) => {
                let item = match elem {
                    ForeignScalar::Int => Rmp::Integer(1i64.into()),
                    ForeignScalar::Number => Rmp::F64(1.0),
                    ForeignScalar::Bool => Rmp::Boolean(false),
                    ForeignScalar::String => Rmp::String("a".into()),
                    ForeignScalar::Unit => Rmp::Nil,
                };
                (
                    Rmp::Array(vec![item]),
                    NativeKind::Ptr(HeapKind::TypedArray),
                )
            }
            ForeignType::Optional(inner) => {
                let (v, _) = wire_witness(inner);
                // `Some(v)` rides the canonical non-null carrier.
                (v, NativeKind::Ptr(HeapKind::TypedObject))
            }
            ForeignType::Object { .. } => (
                Rmp::Map(vec![(Rmp::String("open".into()), Rmp::F64(1.0))]),
                NativeKind::Ptr(HeapKind::TypedObject),
            ),
            // Outbound-only; excluded from this inbound sweep by `supports`.
            ForeignType::Map(_) => (Rmp::Nil, NativeKind::Null),
        }
    }

    /// ADR-019 §1 / #196 — the inbound half of the per-type marshaling-table
    /// assertion. Every table entry that crosses in return position must have a
    /// working projection here; a new entry with no arm fails this test rather
    /// than surfacing `NotImplemented` at a user's first call.
    #[test]
    fn every_return_capable_table_entry_unmarshals() {
        use shape_abi_v1::foreign_types::marshal_table;
        let (mut schemas, builtin) = schemas_with_builtins();
        let object_schema = shape_runtime::type_schema::TypeSchemaBuilder::new("Candle")
            .f64_field("open")
            .register(&mut schemas) as u32;

        for entry in marshal_table() {
            if !entry.supports(ForeignDirection::Return) {
                continue;
            }
            let spelling = entry.shape_spelling();
            let (wire, expected_kind) = wire_witness(&entry);
            let schema_id = match entry {
                ForeignType::Object { .. } => Some(object_schema),
                _ => None,
            };
            let bytes = encode(&wire);
            let slot = unmarshal_result(&bytes, &spelling, schema_id, &schemas, &builtin)
                .unwrap_or_else(|e| {
                    panic!(
                        "table entry `{}` has no inbound marshal projection: {:?}",
                        spelling, e
                    )
                });
            assert_eq!(
                slot.kind(),
                expected_kind,
                "table entry `{}` produced the wrong slot kind",
                spelling
            );
            drop(slot);
        }
    }

    /// The declared-type parser is shared, not duplicated: a spelling the table
    /// refuses reaches the marshal layer only as a host-side gap, and says so.
    #[test]
    fn unmapped_return_type_names_the_table_and_the_declaration_check() {
        let (schemas, builtin) = schemas_with_builtins();
        let bytes = encode(&Rmp::Integer(1i64.into()));
        let err = unmarshal_result(&bytes, "Result<DataTable>", None, &schemas, &builtin)
            .expect_err("DataTable is not in the marshaling table");
        match err {
            VMError::NotImplemented(msg) => {
                assert!(msg.contains("DataTable"), "message names the type: {msg}");
                assert!(msg.contains("C0933"), "message cites the declaration check: {msg}");
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }
}
