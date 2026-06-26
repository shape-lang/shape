//! Conversion between runtime values and wire format.
//!
//! Phase 2b kind-threaded rewrite. Public functions take `(bits: u64,
//! kind: NativeKind)` pairs threaded from the FunctionBlob's compile-
//! time slot-kind metadata; internal dispatch is a `match kind { ... }`
//! with no tag-bit probing. Heap slots use `NativeKind::Ptr(HeapKind)` —
//! the kind tells the dispatcher which `HeapValue` arm decodes the
//! bits without probing the heap object's self-reported discriminant
//! in production (debug-only consistency check).
//!
//! See `docs/defections.md` 2026-05-06 (Phase 2b unified marshal +
//! wire/snapshot kind threading) for the architectural rationale.
//!
//! ## API
//!
//! - [`slot_to_wire`] — project (bits, kind) into a `WireValue`.
//! - [`wire_to_slot`] — project a `WireValue` into typed slot bits,
//!   given the `expected_kind` the caller wants. Returns
//!   `Result<u64, MarshalError>`.
//! - [`slot_to_envelope`] — wrap a typed slot in a `ValueEnvelope` with
//!   metadata.
//! - [`slot_extract_content`] — extract Content node renderings from a
//!   slot whose kind says it carries Content / DataTable / TableView.
//! - [`datatable_to_wire`] / [`datatable_to_ipc_bytes`] /
//!   [`datatable_from_ipc_bytes`] — typed `DataTable` ↔ wire/IPC.

use crate::marshal::MarshalError;
use crate::Context;
use arrow_ipc::{reader::FileReader, writer::FileWriter};
use shape_value::heap_value::HeapValue;
use shape_value::{DataTable, HeapKind, KindedSlot, NativeKind, TypedObjectStorage, ValueSlot};
use shape_wire::{DurationUnit as WireDurationUnit, ValueEnvelope, WireTable, WireValue};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Project a typed slot's `(bits, kind)` to a `WireValue`.
///
/// The `kind` fully determines the projection — no tag-bit probing.
/// For `NativeKind::Ptr(hk)`, the function casts `bits` to
/// `*const HeapValue`, debug-asserts the kind matches, and dispatches
/// per `HeapValue` arm.
pub fn slot_to_wire(bits: u64, kind: NativeKind, ctx: &Context) -> WireValue {
    match kind {
        NativeKind::Float64 => WireValue::Number(f64::from_bits(bits)),
        NativeKind::NullableFloat64 => {
            let v = f64::from_bits(bits);
            if v.is_nan() {
                WireValue::Null
            } else {
                WireValue::Number(v)
            }
        }
        NativeKind::Int64 => WireValue::Integer(bits as i64),
        NativeKind::NullableInt64 => WireValue::Integer(bits as i64),
        NativeKind::Int8 => WireValue::I8(bits as i8),
        NativeKind::Int16 => WireValue::I16(bits as i16),
        NativeKind::Int32 => WireValue::I32(bits as i32),
        NativeKind::UInt8 => WireValue::U8(bits as u8),
        NativeKind::UInt16 => WireValue::U16(bits as u16),
        NativeKind::UInt32 => WireValue::U32(bits as u32),
        NativeKind::UInt64 => WireValue::U64(bits),
        NativeKind::IntSize => WireValue::Isize(bits as i64),
        NativeKind::UIntSize => WireValue::Usize(bits),
        NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUIntSize => WireValue::Integer(bits as i64),
        NativeKind::Bool => WireValue::Bool(bits != 0),
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.5 +
        // §2.7.7/Q9, 2026-05-19): `NativeKind::Null` is the canonical
        // absence-of-value discriminator. Pre-disposition `(0u64,
        // NativeKind::Bool)` was the null sentinel which collided with
        // legitimate `false` bool slots (both encoded as bits=0); the
        // SURFACE-G6-NONE-OUTPUT-ADAPTER reproducer (`fn bar() ->
        // Option<int> { None }; bar()` at top level) materialized
        // `None` as `{"Bool": false}`. Post-disposition: kind IS the
        // discriminator per §2.7.7/Q9 — `NativeKind::Null` slots
        // project to `WireValue::Null` directly, restoring soundness.
        NativeKind::Null => WireValue::Null,
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment adds F32 + Char as 4-byte scalar
        // variants. Wire projection: F32 widens to `WireValue::Number`
        // (`f64::from(f32)` is lossless); Char projects to a single-
        // codepoint string (mirror of the `HeapValue::Char` arm below)
        // because `WireValue` has no dedicated Char variant.
        NativeKind::Float32 => WireValue::Number(f64::from(f32::from_bits(bits as u32))),
        NativeKind::Char => match char::from_u32(bits as u32) {
            Some(c) => WireValue::String(c.to_string()),
            None => WireValue::Null,
        },
        NativeKind::String => {
            // bits is an Arc<String> raw pointer
            let ptr = bits as *const String;
            // SAFETY: kind contract pins this slot to an Arc<String> raw ptr.
            let s = unsafe { &*ptr };
            WireValue::String(s.clone())
        }
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (ADR-006 §2.7.5 amendment, 2026-05-14): the v2-raw `*const StringObj`
        // carrier projects to the same `WireValue::String` wire shape as
        // `NativeKind::String` (Arc-wrapped sibling), via the carrier's
        // `as_str` accessor reading the UTF-8 payload at offset 8 (data ptr)
        // / 16 (len) of the `repr(C)` struct. The slot bits are NOT an
        // `Arc<T>` pointer — `StringObj` is a manually-allocated `repr(C)`
        // 24-byte carrier per `v2/string_obj.rs`.
        NativeKind::StringV2 => {
            if bits == 0 {
                return WireValue::Null;
            }
            // SAFETY: per the §2.7.5 amendment construction contract,
            // kind=StringV2 means bits = `ptr as u64` pointing to a live
            // `StringObj` with bumped refcount — the slot owns one
            // v2-retain share for the duration of this call.
            let ptr = bits as *const shape_value::v2::string_obj::StringObj;
            let s: &str = unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr) };
            WireValue::String(s.to_string())
        }
        // Wave 2 Agent B: the v2-raw `*const DecimalObj` carrier projects
        // to `WireValue::Number` (the same wire shape as
        // `HeapValue::Decimal` per `heap_value_to_wire` below) via the
        // carrier's `value` accessor reading the inline `rust_decimal::Decimal`
        // at offset 8 of the `repr(C)` struct.
        NativeKind::DecimalV2 => {
            if bits == 0 {
                return WireValue::Null;
            }
            // SAFETY: per the §2.7.5 amendment construction contract,
            // kind=DecimalV2 means bits = `ptr as u64` pointing to a live
            // `DecimalObj` with bumped refcount.
            let ptr = bits as *const shape_value::v2::decimal_obj::DecimalObj;
            let value = unsafe { shape_value::v2::decimal_obj::DecimalObj::value(ptr) };
            WireValue::Number(value.to_string().parse().unwrap_or(0.0))
        }
        NativeKind::Ptr(hk) => heap_to_wire(bits, hk, ctx),
    }
}

/// Project an `Arc<HeapValue>` raw pointer slot to `WireValue`,
/// dispatching on the pre-known `HeapKind` rather than probing the
/// heap object's self-reported `kind()`.
fn heap_to_wire(bits: u64, hk: HeapKind, ctx: &Context) -> WireValue {
    if bits == 0 {
        return WireValue::Null;
    }
    // Defensive: `HeapKind::Char` is an inline-codepoint label, NOT an
    // `Arc<HeapValue>` pointer — its `bits` are a raw UTF-32 codepoint.
    // The canonical post-amendment carrier is the scalar `NativeKind::Char`
    // (handled in `slot_to_wire`), but any producer that still stamps the
    // pre-amendment `Ptr(HeapKind::Char)` label must not reach the
    // `*const HeapValue` cast below — casting a codepoint (e.g. 0x63) to a
    // HeapValue pointer and dereferencing it is a misaligned-pointer abort.
    // This arm projects the codepoint directly, mirroring the
    // `NativeKind::Char` arm and `HeapValue::Char` arm.
    if hk == HeapKind::Char {
        return match char::from_u32(bits as u32) {
            Some(c) => WireValue::String(c.to_string()),
            None => WireValue::Null,
        };
    }
    // WS-3 F2b: `HeapKind::Result` / `HeapKind::Option` are typed-Arc
    // dispatch labels — their bits are `Arc::into_raw(Arc<ResultData>)` /
    // `Arc::into_raw(Arc<OptionData>)`, NOT an `Arc<HeapValue>`. Casting
    // those bits to `*const HeapValue` (the path below) reads a
    // `ResultData`/`OptionData` as a `HeapValue` enum — type confusion +
    // UB. This crashed (SIGSEGV) whenever a `Result`/`Option` was the
    // program's terminal value (e.g. `fn main() -> Result<int,string>`
    // returning `Ok(0)` as the trailing expression). Project the typed
    // payload directly, mirroring `printing.rs`'s `HeapKind::Result` /
    // `HeapKind::Option` formatter arms.
    if hk == HeapKind::Result {
        // SAFETY: `KindedSlot::from_result` construction contract —
        // Result-kind bits are `Arc::into_raw(Arc<ResultData>)`.
        let r: &shape_value::heap_value::ResultData =
            unsafe { &*(bits as *const shape_value::heap_value::ResultData) };
        let inner = slot_to_wire(r.payload.raw(), r.payload.kind(), ctx);
        return WireValue::Result {
            ok: r.is_ok,
            value: Box::new(inner),
        };
    }
    if hk == HeapKind::Option {
        // SAFETY: `KindedSlot::from_option` construction contract —
        // Option-kind bits are `Arc::into_raw(Arc<OptionData>)`.
        let o: &shape_value::heap_value::OptionData =
            unsafe { &*(bits as *const shape_value::heap_value::OptionData) };
        if o.is_some {
            return slot_to_wire(o.payload.raw(), o.payload.kind(), ctx);
        }
        return WireValue::Null;
    }
    // JOINT-FIX-1a (2026-05-28): `HeapKind::TypedObject` is a typed-Arc
    // dispatch label per ADR-006 §2.3 + Wave 2 D4 ckpt-2 v2-raw `_new`
    // allocator — its bits are `*const TypedObjectStorage` (or
    // `Arc::into_raw(Arc<TypedObjectStorage>)` for the Arc-allocated
    // sibling at `from_typed_object`), NOT an `Arc<HeapValue>`. Casting
    // those bits to `*const HeapValue` (the path below) reads a
    // `TypedObjectStorage` as a `HeapValue` enum — type confusion + UB
    // identical in shape to the WS-3 F2b Result/Option crash precedent
    // above. Repro: `type P { x: int, y: int }; P { x: 1, y: 2 }` (raw
    // TypedObject as script terminal value) — pre-fix 10/10 SEGV; same
    // pattern via `!!`-produced AnyError TypedObject inside a Result
    // carrier flowing through the recursive `slot_to_wire(r.payload, …)`
    // at L182. Mirror of the `HeapValue::TypedObject(storage)` arm
    // body at L270-301 below — schema-driven field projection into
    // `WireValue::Object(map)`.
    if hk == HeapKind::TypedObject {
        if bits == 0 {
            return WireValue::Null;
        }
        // SAFETY: per the `KindedSlot::from_typed_object_raw` /
        // `from_typed_object` construction contract,
        // `Ptr(HeapKind::TypedObject)` bits are `*const TypedObjectStorage`.
        // Borrow transiently to read schema_id + slots — no share
        // retire (the caller still owns the carrier).
        let storage: &shape_value::TypedObjectStorage =
            unsafe { &*(bits as *const shape_value::TypedObjectStorage) };
        let schema_id = storage.schema_id;
        let slots = storage.slots();
        let heap_mask = storage.heap_mask;
        let schema = ctx
            .type_schema_registry()
            .get_by_id(schema_id as u32)
            .cloned()
            .or_else(|| crate::type_schema::lookup_schema_by_id_public(schema_id as u32));
        if let Some(schema) = schema {
            if let Some(wire) =
                typed_object_result_option_to_wire(schema.name.as_str(), storage, ctx)
            {
                return wire;
            }
            let mut map = BTreeMap::new();
            for field_def in &schema.fields {
                let idx = field_def.index as usize;
                if idx >= slots.len() {
                    continue;
                }
                let Some(field_kind) = storage.field_kinds.get(idx).copied() else {
                    continue;
                };
                let field_bits = slots[idx].raw();
                // JOINT-FIX-1a guard: for heap-kind fields (String /
                // Ptr(_)), the `heap_mask` bit being clear means the
                // slot is uninitialized (zero bits represent "field
                // absent" per the construction-side contract in
                // `build_any_error` at `exceptions/mod.rs:421-440`
                // which initializes `slots` to `ValueSlot::none()` and
                // only sets bits for fields with values). Reading a
                // null `*const String` and dereferencing it is UB —
                // project as `WireValue::Null` per the
                // `NullableFloat64::is_nan` / nullable-int-bits=0
                // sibling discipline at L47-74. Non-heap-kind fields
                // (scalar Bool / Int / etc.) are bit-pattern-valid
                // regardless of mask state.
                let is_heap_kind = field_is_heap_like(field_kind);
                if is_heap_kind && ((heap_mask >> idx) & 1 == 0) {
                    map.insert(field_def.name.clone(), WireValue::Null);
                    continue;
                }
                let field_wire = slot_to_wire(field_bits, field_kind, ctx);
                map.insert(field_def.name.clone(), field_wire);
            }
            return WireValue::Object(map);
        }
        return WireValue::String(format!("<typed_object:schema#{}>", schema_id));
    }
    if hk == HeapKind::TypedArray {
        return v2_typed_array_to_wire(bits, ctx);
    }
    let ptr = bits as *const HeapValue;
    // SAFETY: NativeKind::Ptr(hk) contract — bits is a valid Arc<HeapValue> ptr.
    let hv = unsafe { &*ptr };
    debug_assert_eq!(
        hv.kind(),
        hk,
        "slot kind {:?} does not match HeapValue::{:?}",
        hk,
        hv.kind()
    );
    heap_value_to_wire(hv, ctx)
}

fn v2_typed_array_to_wire(bits: u64, ctx: &Context) -> WireValue {
    use shape_value::v2::heap_header::{HeapHeader, HEAP_KIND_V2_TYPED_ARRAY};
    use shape_value::v2::typed_array::{
        read_elem_type, TypedArray, TypedArrayElem, ELEM_TYPE_BOOL, ELEM_TYPE_CHAR,
        ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I16, ELEM_TYPE_I32,
        ELEM_TYPE_I64, ELEM_TYPE_I8, ELEM_TYPE_STRING, ELEM_TYPE_TRAIT_OBJECT,
        ELEM_TYPE_TYPED_ARRAY, ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U16, ELEM_TYPE_U32, ELEM_TYPE_U8,
    };
    use shape_value::v2::{decimal_obj::DecimalObj, string_obj::StringObj};

    if bits == 0 {
        return WireValue::Null;
    }

    let ptr = bits as *const u8;
    // SAFETY: `HeapKind::TypedArray` is the v2-raw typed-array carrier
    // contract: bits point at a `TypedArray<T>` whose first field is
    // `HeapHeader`.
    let header = unsafe { &*(ptr as *const HeapHeader) };
    assert_eq!(
        header.kind, HEAP_KIND_V2_TYPED_ARRAY,
        "slot kind TypedArray does not point at a v2 TypedArray header"
    );

    // SAFETY: the header check above proves the common 24-byte
    // `TypedArray<T>` layout. Reading `len` through `TypedArray<u8>` only
    // touches the monomorphization-independent header/data/len/cap fields.
    let len = unsafe { (*(ptr as *const TypedArray<u8>)).len };
    // SAFETY: the producer-side stamp lives in the HeapHeader pad byte.
    let elem_type = unsafe { read_elem_type(ptr) };
    let mut values = Vec::with_capacity(len as usize);

    for index in 0..len {
        let value = match elem_type {
            ELEM_TYPE_F64 => unsafe {
                let arr = ptr as *const TypedArray<f64>;
                WireValue::Number(TypedArray::<f64>::get_unchecked(arr, index))
            },
            ELEM_TYPE_I64 => unsafe {
                let arr = ptr as *const TypedArray<i64>;
                WireValue::Integer(TypedArray::<i64>::get_unchecked(arr, index))
            },
            ELEM_TYPE_I32 => unsafe {
                let arr = ptr as *const TypedArray<i32>;
                WireValue::I32(TypedArray::<i32>::get_unchecked(arr, index))
            },
            ELEM_TYPE_BOOL => unsafe {
                let arr = ptr as *const TypedArray<u8>;
                WireValue::Bool(TypedArray::<u8>::get_unchecked(arr, index) != 0)
            },
            ELEM_TYPE_I8 => unsafe {
                let arr = ptr as *const TypedArray<i8>;
                WireValue::I8(TypedArray::<i8>::get_unchecked(arr, index))
            },
            ELEM_TYPE_U8 => unsafe {
                let arr = ptr as *const TypedArray<u8>;
                WireValue::U8(TypedArray::<u8>::get_unchecked(arr, index))
            },
            ELEM_TYPE_I16 => unsafe {
                let arr = ptr as *const TypedArray<i16>;
                WireValue::I16(TypedArray::<i16>::get_unchecked(arr, index))
            },
            ELEM_TYPE_U16 => unsafe {
                let arr = ptr as *const TypedArray<u16>;
                WireValue::U16(TypedArray::<u16>::get_unchecked(arr, index))
            },
            ELEM_TYPE_U32 => unsafe {
                let arr = ptr as *const TypedArray<u32>;
                WireValue::U32(TypedArray::<u32>::get_unchecked(arr, index))
            },
            ELEM_TYPE_F32 => unsafe {
                let arr = ptr as *const TypedArray<f32>;
                WireValue::F32(TypedArray::<f32>::get_unchecked(arr, index))
            },
            ELEM_TYPE_CHAR => unsafe {
                let arr = ptr as *const TypedArray<char>;
                WireValue::String(TypedArray::<char>::get_unchecked(arr, index).to_string())
            },
            ELEM_TYPE_STRING => unsafe {
                let arr = ptr as *const TypedArray<*const StringObj>;
                let elem = TypedArray::<*const StringObj>::get_unchecked(arr, index);
                slot_to_wire(elem as u64, NativeKind::StringV2, ctx)
            },
            ELEM_TYPE_DECIMAL => unsafe {
                let arr = ptr as *const TypedArray<*const DecimalObj>;
                let elem = TypedArray::<*const DecimalObj>::get_unchecked(arr, index);
                slot_to_wire(elem as u64, NativeKind::DecimalV2, ctx)
            },
            ELEM_TYPE_TYPED_OBJECT => unsafe {
                let arr = ptr as *const TypedArray<*const TypedObjectStorage>;
                let elem = TypedArray::<*const TypedObjectStorage>::get_unchecked(arr, index);
                slot_to_wire(elem as u64, NativeKind::Ptr(HeapKind::TypedObject), ctx)
            },
            ELEM_TYPE_TRAIT_OBJECT => {
                panic!(
                    "TypedArray wire conversion cannot serialize Array<dyn Trait>: \
                     TraitObjectStorage has no wire-stable representation. \
                     Refusing to fabricate a placeholder value."
                );
            }
            ELEM_TYPE_TYPED_ARRAY => unsafe {
                let arr = ptr as *const TypedArray<*const TypedArrayElem>;
                let elem = TypedArray::<*const TypedArrayElem>::get_unchecked(arr, index);
                v2_typed_array_to_wire(elem as u64, ctx)
            },
            other => {
                panic!(
                    "TypedArray wire conversion requires a known producer-side \
                     element stamp, got discriminant {other}"
                );
            }
        };
        values.push(value);
    }

    WireValue::Array(values)
}

fn typed_object_result_option_to_wire(
    schema_name: &str,
    storage: &TypedObjectStorage,
    ctx: &Context,
) -> Option<WireValue> {
    use crate::type_schema::builtin_schemas::{
        OPTION_PAYLOAD, OPTION_VARIANT, OPTION_VARIANT_NONE, OPTION_VARIANT_SOME, RESULT_PAYLOAD,
        RESULT_VARIANT, RESULT_VARIANT_ERR, RESULT_VARIANT_OK,
    };

    let slots = storage.slots();
    match schema_name {
        "__Result"
            if slots.len() > RESULT_PAYLOAD && storage.field_kinds.len() > RESULT_PAYLOAD =>
        {
            let ok = match slots[RESULT_VARIANT].as_i64() {
                RESULT_VARIANT_OK => true,
                RESULT_VARIANT_ERR => false,
                _ => return None,
            };
            Some(WireValue::Result {
                ok,
                value: Box::new(typed_object_field_to_wire(storage, RESULT_PAYLOAD, ctx)),
            })
        }
        "__Option"
            if slots.len() > OPTION_PAYLOAD && storage.field_kinds.len() > OPTION_PAYLOAD =>
        {
            match slots[OPTION_VARIANT].as_i64() {
                OPTION_VARIANT_SOME => {
                    Some(typed_object_field_to_wire(storage, OPTION_PAYLOAD, ctx))
                }
                OPTION_VARIANT_NONE => Some(WireValue::Null),
                _ => None,
            }
        }
        _ => None,
    }
}

fn typed_object_field_to_wire(
    storage: &TypedObjectStorage,
    idx: usize,
    ctx: &Context,
) -> WireValue {
    let Some(field_kind) = storage.field_kinds.get(idx).copied() else {
        return WireValue::Null;
    };
    let Some(slot) = storage.slots().get(idx) else {
        return WireValue::Null;
    };
    let field_bits = slot.raw();
    if field_is_heap_like(field_kind) && ((storage.heap_mask >> idx) & 1 == 0 || field_bits == 0) {
        return WireValue::Null;
    }
    slot_to_wire(field_bits, field_kind, ctx)
}

fn field_is_heap_like(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::String | NativeKind::StringV2 | NativeKind::DecimalV2 | NativeKind::Ptr(_)
    )
}

/// Project a `&HeapValue` to `WireValue` by dispatching on its
/// surviving variants. Reused by the snapshot path (Phase 2b
/// snapshot.rs commit) which has the same heap projection needs.
pub fn heap_value_to_wire(hv: &HeapValue, ctx: &Context) -> WireValue {
    match hv {
        HeapValue::String(s) => WireValue::String((**s).clone()),
        HeapValue::Decimal(d) => WireValue::Number(d.to_string().parse().unwrap_or(0.0)),
        HeapValue::BigInt(i) => WireValue::Integer(**i),
        HeapValue::Char(c) => WireValue::String(c.to_string()),
        HeapValue::Future(id) => WireValue::String(format!("<future:{}>", id)),
        HeapValue::DataTable(dt) => datatable_to_wire(dt.as_ref()),
        HeapValue::Content(_node) => {
            // Phase 1.B: the JSON-renderer integration for Content trees
            // is the deferred Phase 2c content-marshalling rebuild — see
            // ADR-006 §2.7.4. Until then, surface a placeholder
            // WireValue rather than emit a partial / wrong-shape
            // serialization.
            WireValue::String("<content:phase-2c-rebuild>".to_string())
        }
        HeapValue::Instant(t) => WireValue::String(format!("{:?}", **t)),
        HeapValue::IoHandle(_h) => {
            // Phase 1.B: IoHandleData no longer exposes a stable `id()`
            // accessor; the handle's identity is structural (the inner
            // OS resource) rather than a numeric tag. Phase 2c surfaces
            // a kind-threaded handle-printer.
            WireValue::String("<io_handle>".to_string())
        }
        HeapValue::NativeScalar(v) => match v {
            shape_value::heap_value::NativeScalar::I8(n) => WireValue::I8(*n),
            shape_value::heap_value::NativeScalar::U8(n) => WireValue::U8(*n),
            shape_value::heap_value::NativeScalar::I16(n) => WireValue::I16(*n),
            shape_value::heap_value::NativeScalar::U16(n) => WireValue::U16(*n),
            shape_value::heap_value::NativeScalar::I32(n) => WireValue::I32(*n),
            shape_value::heap_value::NativeScalar::I64(n) => WireValue::I64(*n),
            shape_value::heap_value::NativeScalar::U32(n) => WireValue::U32(*n),
            shape_value::heap_value::NativeScalar::U64(n) => WireValue::U64(*n),
            shape_value::heap_value::NativeScalar::Isize(n) => WireValue::Isize(*n as i64),
            shape_value::heap_value::NativeScalar::Usize(n) => WireValue::Usize(*n as u64),
            shape_value::heap_value::NativeScalar::Ptr(n) => WireValue::Ptr(*n as u64),
            shape_value::heap_value::NativeScalar::F32(n) => WireValue::F32(*n),
        },
        HeapValue::NativeView(v) => WireValue::Object(
            [
                (
                    "__type".to_string(),
                    WireValue::String(if v.mutable { "cmut" } else { "cview" }.to_string()),
                ),
                (
                    "layout".to_string(),
                    WireValue::String(v.layout.name.clone()),
                ),
                (
                    "ptr".to_string(),
                    WireValue::String(format!("0x{:x}", v.ptr)),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        HeapValue::TypedObject(storage) => {
            // ADR-005 §Forbidden / Q10 forward pointer: wire serialization
            // must NOT re-introduce Box<HeapValue> slot wrapping. The
            // schema-driven kind threading below is ADR-005-aligned (typed
            // slot bits + schema; no intermediate HeapValue materialization
            // on deserialization).
            let schema_id = storage.schema_id;
            let slots = storage.slots();
            let schema = ctx
                .type_schema_registry()
                .get_by_id(schema_id as u32)
                .cloned()
                .or_else(|| crate::type_schema::lookup_schema_by_id_public(schema_id as u32));
            if let Some(schema) = schema {
                let mut map = BTreeMap::new();
                for field_def in &schema.fields {
                    let idx = field_def.index as usize;
                    if idx >= slots.len() {
                        continue;
                    }
                    let Some(field_kind) = schema.field_kind(idx) else {
                        continue;
                    };
                    let field_bits = slots[idx].raw();
                    let field_wire = slot_to_wire(field_bits, field_kind, ctx);
                    map.insert(field_def.name.clone(), field_wire);
                }
                WireValue::Object(map)
            } else {
                WireValue::String(format!("<typed_object:schema#{}>", schema_id))
            }
        }
        HeapValue::ClosureRaw(_handle) => {
            // Phase 1.B: OwnedClosureBlock no longer exposes a public
            // `function_id()` accessor on the runtime side (the typed-
            // closure slot ABI carries the function-id via the
            // `TypedClosureHeader` itself). Phase 2c lands a
            // schema-aware closure printer.
            WireValue::String("<closure>".to_string())
        }
        HeapValue::TaskGroup(_data) => WireValue::String("<task_group>".to_string()),
        // V3-S5 ckpt-5-prime (2026-05-15): `HeapValue::TypedArray(arc)` arm
        // RETIRED in lockstep with the deleted `HeapValue::TypedArray` variant
        // (ckpt-4) + deleted `TypedArrayData` inner enum (ckpt-1). Wire
        // serialisation of v2-raw `*mut TypedArray<T>` pointers lands at the
        // ckpt-5-prime² + ckpt-6 producer/consumer storage-shape migration
        // (per-element-type marshal-layer projection before the value becomes
        // a `HeapValue`). The `typed_array_to_wire` helper below is RETIRED
        // in the same lockstep. Refusal #1 binding.
        HeapValue::Temporal(td) => temporal_to_wire(&**td),
        HeapValue::TableView(tv) => match &**tv {
            shape_value::heap_value::TableViewData::TypedTable { table, schema_id } => {
                datatable_to_wire_with_schema(table.as_ref(), Some(*schema_id as u32))
            }
            shape_value::heap_value::TableViewData::IndexedTable { table, .. } => {
                datatable_to_wire(table.as_ref())
            }
            shape_value::heap_value::TableViewData::RowView { .. }
            | shape_value::heap_value::TableViewData::ColumnRef { .. } => {
                WireValue::String("<table_view:phase-2c>".to_string())
            }
        },
        HeapValue::HashMap(_) => {
            // Phase 1.B (ADR-006 §2.7.4): kind-threaded HashMap-to-wire
            // serialization is the deferred Phase 2c marshal rebuild.
            WireValue::String("<hashmap:phase-2c>".to_string())
        }
        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
        // 2026-05-10): Set wire serialization follows the same
        // phase-2c deferral shape as HashMap; surface as an opaque
        // tag until the marshal rebuild lands.
        HeapValue::HashSet(_) => WireValue::String("<hashset:phase-2c>".to_string()),
        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10):
        // Deque wire serialization follows the same phase-2c deferral
        // shape as HashMap / HashSet — opaque tag until the marshal
        // rebuild lands.
        HeapValue::Deque(_) => WireValue::String("<deque:phase-2c>".to_string()),
        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment):
        // FilterExpr trees are transient query-DSL values; they don't
        // cross the wire boundary today. Surface as an opaque tag.
        HeapValue::FilterExpr(_) => WireValue::String("<filter_expr>".to_string()),
        // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10): Reference
        // values are within-program data and never cross the wire
        // boundary. Surface as an opaque tag, same as FilterExpr.
        HeapValue::Reference(_) => WireValue::String("<ref>".to_string()),
        // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
        // Iterator pipelines are lazy within-program values and never
        // cross the wire boundary (callers materialise via collect /
        // forEach / etc. before serialisation). Surface as an opaque
        // tag, same as FilterExpr / Reference.
        HeapValue::Iterator(_) => WireValue::String("<iterator>".to_string()),
        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21, 2026-05-10):
        // channels are concurrency primitives with interior
        // `Mutex<ChannelInner>` state; no wire serialization at landing —
        // same phase-2c deferral shape as HashMap / HashSet. Surface as
        // an opaque tag for diagnostics.
        HeapValue::Channel(_) => WireValue::String("<channel:phase-2c>".to_string()),
        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
        // 2026-05-10): PriorityQueue wire serialisation projects to a
        // `WireValue::Array` of i64 priorities in heap-array order
        // (mirror of the JSON shape — i64-priority-only at landing).
        HeapValue::PriorityQueue(d) => {
            WireValue::Array(d.heap.iter().map(|v| WireValue::Integer(*v)).collect())
        }
        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10): Range
        // serializes as a JSON-ish `{"start", "end", "step",
        // "inclusive"}` payload via the `as_array_for_wire` shape
        // (range bounds + step are tiny scalars; lossless round-trip).
        // Wire serialization here just stamps the literal-form string
        // — full structured wire is the deferred Phase 2c marshal
        // rebuild same as HashMap / HashSet (which surface as opaque
        // tags above). Matches the playbook's "wire/JSON conversion
        // arms (rejection or proper)" guidance.
        HeapValue::Range(r) => {
            let s = if r.inclusive {
                format!("{}..={}", r.start, r.end)
            } else {
                format!("{}..{}", r.start, r.end)
            };
            WireValue::String(s)
        }
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10):
        // Result/Option carriers are within-program control-flow values;
        // wire serialisation goes through the AnyError schema for thrown
        // errors and the unwrapped inner value for `Ok(_)` / `Some(_)`.
        // Until those marshal paths land, surface as an opaque tag —
        // same Phase-2c deferral shape as HashMap / HashSet / Iterator.
        HeapValue::Result(_) => WireValue::String("<result:phase-2c>".to_string()),
        HeapValue::Option(_) => WireValue::String("<option:phase-2c>".to_string()),
        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11): concurrency
        // primitives are runtime-tier handles with no wire shape.
        // Surface as opaque tags — same Phase-2c deferral shape as
        // Channel / HashMap / HashSet.
        HeapValue::Mutex(_) => WireValue::String("<mutex:phase-2c>".to_string()),
        HeapValue::Atomic(_) => WireValue::String("<atomic:phase-2c>".to_string()),
        HeapValue::Lazy(_) => WireValue::String("<lazy:phase-2c>".to_string()),
        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11):
        // `dyn Trait` carriers have no wire shape — same Phase-2c
        // deferral as concurrency primitives. A future `Serializable`
        // trait could route through the vtable, but that's emission-tier
        // work outside this sub-cluster.
        HeapValue::TraitObject(_) => WireValue::String("<trait_object:phase-2c>".to_string()),
        // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
        // ModuleFn references are VM-internal callable handles
        // — same opaque-tag shape as the concurrency primitives.
        HeapValue::ModuleFn(id) => WireValue::String(format!("<module_fn:{}>", id)),
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix /
        // MatrixSlice wire serialisation inherits the N7-architectural-
        // choice deferral from the pre-amendment
        // `TypedArrayData::Matrix` / `FloatSlice` shape (the 2D-layout
        // encoding policy is undecided). Surface as opaque tags —
        // same Phase-2c deferral pattern as the concurrency primitives.
        HeapValue::Matrix(m) => {
            WireValue::String(format!("<matrix:{}x{}:phase-2c>", m.rows, m.cols))
        }
        HeapValue::MatrixSlice(s) => {
            WireValue::String(format!("<matrix_slice:{}:phase-2c>", s.len))
        }
    }
}

// V3-S5 ckpt-5-prime (2026-05-15): `typed_array_to_wire` helper RETIRED per W12
// audit §3.6 + handover §0 wholesale-deletion cascade. The helper
// pattern-matched on the deleted `TypedArrayData` enum (retired at ckpt-1) and
// was called by the deleted `HeapValue::TypedArray` outer arm (retired at
// ckpt-4) above. The v2-raw `*mut TypedArray<T>` wire-serialisation path lands
// at the ckpt-5-prime² + ckpt-6 producer/consumer storage-shape migration
// (per-element-type marshal-layer projection before the value becomes a
// `HeapValue`). Refusal #1 binding.

fn temporal_to_wire(td: &shape_value::heap_value::TemporalData) -> WireValue {
    use shape_value::heap_value::TemporalData;
    match td {
        TemporalData::DateTime(dt) => WireValue::Timestamp(dt.timestamp_millis()),
        TemporalData::TimeSpan(d) => WireValue::Duration {
            value: d.num_milliseconds() as f64,
            unit: WireDurationUnit::Milliseconds,
        },
        TemporalData::Duration(d) => WireValue::Duration {
            value: d.value,
            unit: WireDurationUnit::Milliseconds,
        },
        TemporalData::Timeframe(_)
        | TemporalData::TimeReference(_)
        | TemporalData::DateTimeExpr(_)
        | TemporalData::DataDateTimeRef(_) => WireValue::String(format!("<{}>", td.type_name())),
    }
}

/// Project a `WireValue` to typed slot bits, given the kind the caller
/// wants. Returns [`MarshalError::KindMismatch`] when wire shape doesn't
/// match the expected kind.
///
/// For heap kinds, this allocates a new `Arc<HeapValue>` and returns
/// the raw pointer as bits — caller takes ownership of the heap
/// reference (one strong count).
pub fn wire_to_slot(wire: &WireValue, expected_kind: NativeKind) -> Result<u64, MarshalError> {
    match (wire, expected_kind) {
        (WireValue::Number(n), NativeKind::Float64) => Ok(f64::to_bits(*n)),
        (WireValue::Integer(i), NativeKind::Int64) => Ok(*i as u64),
        (WireValue::Bool(b), NativeKind::Bool) => Ok(*b as u64),
        (WireValue::Null, NativeKind::NullableFloat64) => Ok(f64::to_bits(f64::NAN)),
        (WireValue::Null, NativeKind::Null) => Ok(0),
        (WireValue::String(s), NativeKind::String) => {
            let arc = Arc::new(s.clone());
            Ok(Arc::into_raw(arc) as u64)
        }
        (WireValue::I8(n), NativeKind::Int8) => Ok((*n as i64) as u64),
        (WireValue::I16(n), NativeKind::Int16) => Ok((*n as i64) as u64),
        (WireValue::I32(n), NativeKind::Int32) => Ok((*n as i64) as u64),
        (WireValue::U8(n), NativeKind::UInt8) => Ok(*n as u64),
        (WireValue::U16(n), NativeKind::UInt16) => Ok(*n as u64),
        (WireValue::U32(n), NativeKind::UInt32) => Ok(*n as u64),
        (WireValue::U64(n), NativeKind::UInt64) => Ok(*n),
        // Heap kinds are constructed by allocating Arc<HeapValue> with the
        // matching variant. Each surviving HeapKind variant is handled here
        // as stdlib mass migration (Phase 2c) and the snapshot replay path
        // discover concrete consumers.
        (WireValue::String(s), NativeKind::Ptr(HeapKind::String)) => {
            let arc = Arc::new(HeapValue::String(Arc::new(s.clone())));
            Ok(Arc::into_raw(arc) as u64)
        }
        (WireValue::Table(table), NativeKind::Ptr(HeapKind::DataTable)) => {
            let dt = datatable_from_ipc_bytes(&table.ipc_bytes, None, None)
                .map_err(MarshalError::Body)?;
            let arc = Arc::new(HeapValue::DataTable(Arc::new(dt)));
            Ok(Arc::into_raw(arc) as u64)
        }
        (WireValue::Result { ok, value }, NativeKind::Ptr(HeapKind::TypedObject)) => {
            let payload = wire_payload_to_kinded_slot(value)?;
            Ok(build_builtin_result_typed_object(*ok, payload))
        }
        (WireValue::Null, NativeKind::Ptr(HeapKind::TypedObject)) => {
            Ok(build_builtin_option_typed_object(false, KindedSlot::none()))
        }
        // Calling site passed a wire/kind pair we don't currently handle.
        // The strict-typed answer is to extend this match, not fall back —
        // each new case represents a concrete stdlib/wire shape, and
        // pattern-match exhaustiveness is the discipline.
        _ => Err(MarshalError::Body(format!(
            "wire_to_slot: no projection for wire variant into kind {:?}",
            expected_kind
        ))),
    }
}

fn wire_payload_to_kinded_slot(wire: &WireValue) -> Result<KindedSlot, MarshalError> {
    let slot = match wire {
        WireValue::Null => KindedSlot::none(),
        WireValue::Bool(b) => KindedSlot::new(
            ValueSlot::from_raw(if *b { 1 } else { 0 }),
            NativeKind::Bool,
        ),
        WireValue::Number(n) => {
            KindedSlot::new(ValueSlot::from_raw(n.to_bits()), NativeKind::Float64)
        }
        WireValue::Integer(i) => KindedSlot::new(ValueSlot::from_raw(*i as u64), NativeKind::Int64),
        WireValue::I8(n) => {
            KindedSlot::new(ValueSlot::from_raw((*n as i64) as u64), NativeKind::Int8)
        }
        WireValue::U8(n) => KindedSlot::new(ValueSlot::from_raw(*n as u64), NativeKind::UInt8),
        WireValue::I16(n) => {
            KindedSlot::new(ValueSlot::from_raw((*n as i64) as u64), NativeKind::Int16)
        }
        WireValue::U16(n) => KindedSlot::new(ValueSlot::from_raw(*n as u64), NativeKind::UInt16),
        WireValue::I32(n) => {
            KindedSlot::new(ValueSlot::from_raw((*n as i64) as u64), NativeKind::Int32)
        }
        WireValue::U32(n) => KindedSlot::new(ValueSlot::from_raw(*n as u64), NativeKind::UInt32),
        WireValue::I64(n) => KindedSlot::new(ValueSlot::from_raw(*n as u64), NativeKind::Int64),
        WireValue::U64(n) => KindedSlot::new(ValueSlot::from_raw(*n), NativeKind::UInt64),
        WireValue::Isize(n) => KindedSlot::new(ValueSlot::from_raw(*n as u64), NativeKind::IntSize),
        WireValue::Usize(n) => KindedSlot::new(ValueSlot::from_raw(*n), NativeKind::UIntSize),
        WireValue::F32(n) => {
            KindedSlot::new(ValueSlot::from_raw(n.to_bits() as u64), NativeKind::Float32)
        }
        WireValue::String(s) => KindedSlot::from_string_arc(Arc::new(s.clone())),
        WireValue::Result { ok, value } => {
            let payload = wire_payload_to_kinded_slot(value)?;
            KindedSlot::new(
                ValueSlot::from_raw(build_builtin_result_typed_object(*ok, payload)),
                NativeKind::Ptr(HeapKind::TypedObject),
            )
        }
        other => {
            return Err(MarshalError::Body(format!(
                "wire_to_slot: no Result/Option payload projection for wire variant {}",
                other.type_name()
            )));
        }
    };
    Ok(slot)
}

fn build_builtin_result_typed_object(ok: bool, payload: KindedSlot) -> u64 {
    use crate::type_schema::builtin_schemas::{RESULT_VARIANT_ERR, RESULT_VARIANT_OK};
    let (_registry, schemas) =
        crate::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
    build_builtin_variant_typed_object(
        schemas.result as u64,
        if ok {
            RESULT_VARIANT_OK
        } else {
            RESULT_VARIANT_ERR
        },
        payload,
    )
}

fn build_builtin_option_typed_object(is_some: bool, payload: KindedSlot) -> u64 {
    use crate::type_schema::builtin_schemas::{OPTION_VARIANT_NONE, OPTION_VARIANT_SOME};
    let (_registry, schemas) =
        crate::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
    build_builtin_variant_typed_object(
        schemas.option as u64,
        if is_some {
            OPTION_VARIANT_SOME
        } else {
            OPTION_VARIANT_NONE
        },
        payload,
    )
}

fn build_builtin_variant_typed_object(schema_id: u64, variant: i64, payload: KindedSlot) -> u64 {
    use crate::type_schema::builtin_schemas::OPTION_PAYLOAD;
    let payload_slot = payload.slot();
    let payload_kind = payload.kind();
    let payload_bits = payload_slot.raw();
    let heap_mask = if payload_bits != 0 && field_is_heap_like(payload_kind) {
        1u64 << OPTION_PAYLOAD
    } else {
        0
    };
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());
    let ptr = TypedObjectStorage::_new(
        schema_id,
        vec![ValueSlot::from_int(variant), payload_slot].into_boxed_slice(),
        heap_mask,
        field_kinds,
    );
    std::mem::forget(payload);
    ptr as u64
}

/// Wrap a typed slot in a `ValueEnvelope` with optional metadata.
///
/// `type_name` is the user-facing Shape type name (e.g. `"int"`,
/// `"DataTable"`, `"MyType"`). The envelope's `type_info` is populated
/// from the type registry when available.
pub fn slot_to_envelope(
    bits: u64,
    kind: NativeKind,
    type_name: &str,
    ctx: &Context,
) -> ValueEnvelope {
    let value = slot_to_wire(bits, kind, ctx);
    let _ = type_name;
    let _ = ctx;
    // Phase 1.B (ADR-006 §2.7.4): the type-info / type-registry lookup
    // path that resolved a `TypeRegistry` from `TypeRegistry::default()`
    // is gone; the rebuilt path queries `TypeRegistry::for_number` /
    // primitives + the runtime's per-schema cache. Until the kind-
    // threaded envelope lookup lands in Phase 2c, fall back to the
    // wire-side inference helper.
    ValueEnvelope::from_value(value)
}

/// If the slot carries a renderable Content shape (Content node, DataTable,
/// or TableView), return `(content_json, content_html, content_terminal)`.
/// Otherwise all three are `None`.
///
/// W18.2 (R8 — output-adapter integration): the kind-threaded content
/// dispatch is rebuilt on top of the surviving 6-renderer infrastructure
/// (TERMINAL / HTML / MARKDOWN / JSON / PLAIN). Slot bits are dispatched
/// per `HeapKind`, the underlying typed payload is materialised as a
/// `ContentNode`, and each renderer projects the node into its own
/// output shape:
///
/// - JSON via [`crate::renderers::json::JsonRenderer`] parsed into
///   [`serde_json::Value`] (the renderer guarantees valid JSON per the
///   `renderers::cross_renderer_tests::json_is_valid_json` regression).
/// - HTML via [`crate::renderers::html::HtmlRenderer`].
/// - Terminal via [`crate::renderers::terminal::TerminalRenderer`] (ANSI
///   escapes; consumers route to PLAIN when the sink is non-TTY).
///
/// Pre-rebuild this function returned `(None, None, None)` for every
/// Content-bearing slot per the Phase 1.B placeholder — that scar is
/// resolved here.
pub fn slot_extract_content(
    bits: u64,
    kind: NativeKind,
) -> (Option<serde_json::Value>, Option<String>, Option<String>) {
    let NativeKind::Ptr(hk) = kind else {
        return (None, None, None);
    };
    if bits == 0 {
        return (None, None, None);
    }
    // SAFETY: `Ptr(HeapKind::*)` contract — bits is a valid typed pointer
    // for the labelled heap kind. The `(hk, hv)` cross-check below is
    // belt-and-braces; an inconsistent label is a producer-side bug.
    let node = match hk {
        HeapKind::Content => {
            let hv = unsafe { &*(bits as *const HeapValue) };
            match hv {
                HeapValue::Content(node) => Some((**node).clone()),
                _ => None,
            }
        }
        HeapKind::DataTable => {
            let dt: &shape_value::DataTable = unsafe { &*(bits as *const shape_value::DataTable) };
            Some(crate::content_dispatch::datatable_to_content_node(dt, None))
        }
        HeapKind::TableView => {
            let tv: &shape_value::heap_value::TableViewData =
                unsafe { &*(bits as *const shape_value::heap_value::TableViewData) };
            match tv {
                shape_value::heap_value::TableViewData::TypedTable { table, .. }
                | shape_value::heap_value::TableViewData::IndexedTable { table, .. } => Some(
                    crate::content_dispatch::datatable_to_content_node(table.as_ref(), None),
                ),
                // RowView / ColumnRef are deferred Phase 2c content
                // adapters — no current renderer.
                _ => None,
            }
        }
        _ => None,
    };
    let Some(node) = node else {
        return (None, None, None);
    };

    // W18.2: route the materialised `ContentNode` through each of the
    // three renderer adapters wired into the host boundary. The JSON
    // renderer's output is guaranteed-valid JSON (see
    // `renderers::cross_renderer_tests::json_is_valid_json`); the
    // `serde_json::from_str` fallback to `None` is defensive only.
    use crate::content_renderer::ContentRenderer;
    let json_renderer = crate::renderers::json::JsonRenderer;
    let html_renderer = crate::renderers::html::HtmlRenderer::new();
    let terminal_renderer = crate::renderers::terminal::TerminalRenderer::new();
    let content_json: Option<serde_json::Value> =
        serde_json::from_str(&json_renderer.render(&node)).ok();
    let content_html = Some(html_renderer.render(&node));
    let content_terminal = Some(terminal_renderer.render(&node));
    (content_json, content_html, content_terminal)
}

// ───────────────────────── DataTable ↔ wire/IPC ─────────────────────────
//
// Typed-handle conversions. These don't go through `(bits, kind)` —
// the caller passes a `&DataTable` directly, which is the typed-Rust
// equivalent of NativeKind::Ptr(HeapKind::DataTable). The marshal layer
// uses these internally when projecting a DataTable slot.

pub fn datatable_to_wire(dt: &DataTable) -> WireValue {
    datatable_to_wire_with_schema(dt, dt.schema_id())
}

fn datatable_to_wire_with_schema(dt: &DataTable, schema_id: Option<u32>) -> WireValue {
    match datatable_to_ipc_bytes(dt) {
        Ok(ipc_bytes) => WireValue::Table(WireTable {
            ipc_bytes,
            type_name: None,
            schema_id,
            row_count: dt.row_count(),
            column_count: dt.column_count(),
        }),
        Err(e) => WireValue::String(format!("<datatable_serialize_error: {}>", e)),
    }
}

pub fn datatable_to_ipc_bytes(dt: &DataTable) -> std::result::Result<Vec<u8>, String> {
    // The DataTable now wraps a `RecordBatch` directly (`inner()`); the
    // pre-bulldozer `to_arrow_batch` accessor is gone since the wrapper
    // is the batch.
    let arrow_batch = dt.inner();
    let schema = arrow_batch.schema();
    let mut buf = Vec::new();
    {
        let mut writer = FileWriter::try_new(&mut buf, &schema)
            .map_err(|e| format!("Arrow IPC writer init failed: {}", e))?;
        writer
            .write(arrow_batch)
            .map_err(|e| format!("Arrow IPC write failed: {}", e))?;
        writer
            .finish()
            .map_err(|e| format!("Arrow IPC finish failed: {}", e))?;
    }
    Ok(buf)
}

pub fn datatable_from_ipc_bytes(
    bytes: &[u8],
    column_overrides: Option<&[shape_value::datatable::ColumnPtrs]>,
    schema_id_override: Option<u32>,
) -> std::result::Result<DataTable, String> {
    let cursor = std::io::Cursor::new(bytes);
    let reader = FileReader::try_new(cursor, None)
        .map_err(|e| format!("Arrow IPC reader init failed: {}", e))?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|e| format!("Arrow IPC batch read failed: {}", e))?);
    }
    if batches.is_empty() {
        return Err(
            "datatable_from_ipc_bytes: empty IPC stream — no Arrow RecordBatch to wrap".to_string(),
        );
    }
    // The first batch is the canonical wrapper; concatenation is a
    // Phase 2c rebuild item alongside the broader DataTable IPC layer.
    let first = batches.into_iter().next().unwrap();
    let _ = column_overrides;
    let dt = DataTable::new(first);
    let dt = if let Some(sid) = schema_id_override {
        dt.with_schema_id(sid)
    } else {
        dt
    };
    Ok(dt)
}

#[cfg(test)]
mod u64_wire_tests {
    //! R5c-2-β-γ checkpoint (b) u64-carrier — wire/snapshot round-trip.
    //!
    //! A `NativeKind::UInt64` slot must round-trip through MessagePack
    //! wire serialization with its full `0..2^64` range intact: a value
    //! above `i64::MAX` must NOT be lossy-projected to a signed
    //! `WireValue::Integer`. The carrier projects to `WireValue::U64`
    //! (full-range), and `wire_to_slot` recovers the exact bits. The
    //! snapshot path serializes the parallel `Vec<u64>` data + the slot's
    //! `NativeKind` verbatim (per ADR-006 §2.7.7), so the same bit/kind
    //! pair is preserved there by construction.

    use super::{slot_to_wire, wire_to_slot};
    use crate::context::ExecutionContext;
    use shape_value::NativeKind;
    use shape_wire::WireValue;

    fn roundtrip(bits: u64) -> u64 {
        let ctx = ExecutionContext::new_empty();
        let wire = slot_to_wire(bits, NativeKind::UInt64, &ctx);
        // Full-range u64 must project to the dedicated U64 wire variant,
        // not a lossy signed Integer.
        assert!(
            matches!(wire, WireValue::U64(_)),
            "u64 slot must project to WireValue::U64, got {:?}",
            wire
        );
        wire_to_slot(&wire, NativeKind::UInt64).expect("u64 wire should decode")
    }

    #[test]
    fn u64_max_round_trips() {
        assert_eq!(roundtrip(u64::MAX), u64::MAX);
    }

    #[test]
    fn u64_above_i64_max_round_trips_lossless() {
        // i64::MAX + 1 — the first value a signed projection would corrupt.
        let v = (i64::MAX as u64) + 1;
        assert_eq!(roundtrip(v), v);
    }

    #[test]
    fn u64_small_round_trips() {
        assert_eq!(roundtrip(0), 0);
        assert_eq!(roundtrip(42), 42);
    }

    #[test]
    fn u64_wire_variant_preserves_full_range() {
        // The wire value itself must carry the full unsigned magnitude.
        let ctx = ExecutionContext::new_empty();
        match slot_to_wire(u64::MAX, NativeKind::UInt64, &ctx) {
            WireValue::U64(n) => assert_eq!(n, u64::MAX),
            other => panic!("expected WireValue::U64, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod typed_array_wire_tests {
    use super::slot_to_wire;
    use crate::context::ExecutionContext;
    use shape_value::v2::typed_array::{
        release_v2_typed_array, stamp_elem_type, TypedArray, ELEM_TYPE_I64,
    };
    use shape_value::{HeapKind, NativeKind};
    use shape_wire::WireValue;

    #[test]
    fn v2_i64_typed_array_projects_to_wire_array() {
        let ctx = ExecutionContext::new_empty();
        let arr = TypedArray::<i64>::from_slice(&[1, 2, 3, 4]);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };

        let wire = slot_to_wire(arr as u64, NativeKind::Ptr(HeapKind::TypedArray), &ctx);
        assert_eq!(
            wire,
            WireValue::Array(vec![
                WireValue::Integer(1),
                WireValue::Integer(2),
                WireValue::Integer(3),
                WireValue::Integer(4),
            ])
        );

        unsafe { release_v2_typed_array(arr as *mut u8) };
    }
}

#[cfg(test)]
mod char_wire_tests {
    //! β-fix CKPT-A char-carrier — `charAt` return-value wire projection.
    //!
    //! `op_string_char_at` (and its char-producing siblings) push a
    //! scalar Unicode codepoint. Pre-fix the slot was stamped
    //! `NativeKind::Ptr(HeapKind::Char)` — a scalar codepoint mislabeled
    //! as an `Arc<HeapValue>` pointer. When that slot was the program
    //! return value, `heap_to_wire` cast the codepoint bits (e.g. 0x63 =
    //! 'c') to `*const HeapValue` and dereferenced it, triggering a
    //! misaligned-pointer non-unwinding abort (SIGABRT).
    //!
    //! The producer-side fix stamps the canonical scalar
    //! `NativeKind::Char` (ADR-006 §2.7.5). The defensive `heap_to_wire`
    //! `HeapKind::Char` early-arm guarantees that even a mislabeled
    //! `Ptr(HeapKind::Char)` slot projects the codepoint directly
    //! instead of dereferencing it as a heap object.

    use super::{heap_to_wire, slot_to_wire};
    use crate::context::ExecutionContext;
    use shape_value::{HeapKind, NativeKind};
    use shape_wire::WireValue;

    /// The canonical post-fix carrier: a scalar `NativeKind::Char` slot
    /// (codepoint inline) projects to a single-codepoint string with no
    /// pointer dereference.
    #[test]
    fn native_kind_char_slot_projects_to_string() {
        let ctx = ExecutionContext::new_empty();
        // 'c' = U+0063 — the `"abc".reverse().charAt(0)` reproducer result.
        let wire = slot_to_wire('c' as u64, NativeKind::Char, &ctx);
        assert_eq!(wire, WireValue::String("c".to_string()));
    }

    /// `charAt(0)` of `"abc"` returns 'a' — direct (non-reversed) path.
    #[test]
    fn native_kind_char_slot_first_codepoint() {
        let ctx = ExecutionContext::new_empty();
        let wire = slot_to_wire('a' as u64, NativeKind::Char, &ctx);
        assert_eq!(wire, WireValue::String("a".to_string()));
    }

    /// Defensive: a `Ptr(HeapKind::Char)`-labeled slot (a mislabeled
    /// scalar codepoint, e.g. emitted by any un-migrated producer) must
    /// NOT be dereferenced as an `Arc<HeapValue>`. The `heap_to_wire`
    /// early-arm projects the codepoint directly. Pre-fix this input
    /// aborted the process with a misaligned-pointer panic.
    #[test]
    fn heap_kind_char_label_does_not_deref_codepoint() {
        let ctx = ExecutionContext::new_empty();
        // 0x63 ('c') is NOT 8-byte aligned and is not a valid HeapValue
        // pointer — the pre-fix catch-all would have aborted here.
        let wire = heap_to_wire('c' as u64, HeapKind::Char, &ctx);
        assert_eq!(wire, WireValue::String("c".to_string()));
    }

    /// Defensive arm also covers a non-ASCII multi-byte codepoint.
    #[test]
    fn heap_kind_char_label_handles_unicode_codepoint() {
        let ctx = ExecutionContext::new_empty();
        let wire = heap_to_wire('λ' as u64, HeapKind::Char, &ctx);
        assert_eq!(wire, WireValue::String("λ".to_string()));
    }

    /// A `Ptr(HeapKind::Char)` slot routed through the public
    /// `slot_to_wire` entry point (the program-return-value path) also
    /// projects safely — this is the exact path the SIGABRT reproducer
    /// exercised.
    #[test]
    fn slot_to_wire_char_label_return_value_path_is_safe() {
        let ctx = ExecutionContext::new_empty();
        let wire = slot_to_wire('c' as u64, NativeKind::Ptr(HeapKind::Char), &ctx);
        assert_eq!(wire, WireValue::String("c".to_string()));
    }
}

#[cfg(test)]
mod ws3_f2b_result_option_wire_tests {
    //! WS-3 F2b — `Result` / `Option` program-return-value wire projection.
    //!
    //! `HeapKind::Result` / `HeapKind::Option` are typed-Arc dispatch
    //! labels — their slot bits are `Arc::into_raw(Arc<ResultData>)` /
    //! `Arc::into_raw(Arc<OptionData>)`, NOT an `Arc<HeapValue>`. Pre-fix,
    //! `heap_to_wire`'s catch-all cast those bits to `*const HeapValue`
    //! and dereferenced — reading a `ResultData`/`OptionData` as a
    //! `HeapValue` enum (type confusion + UB). This crashed (SIGSEGV)
    //! whenever a `Result`/`Option` was the program's terminal value
    //! (e.g. `fn main() -> Result<int,string>` returning `Ok(0)`), which
    //! made every `?`-using program crash once it compiled.
    //!
    //! The fix adds dedicated `HeapKind::Result` / `HeapKind::Option`
    //! arms that read the typed payload directly.

    use super::slot_to_wire;
    use crate::context::ExecutionContext;
    use shape_value::heap_value::{OptionData, ResultData};
    use shape_value::kinded_slot::KindedSlot;
    use shape_wire::WireValue;
    use std::sync::Arc;

    #[test]
    fn ok_result_projects_to_wire_result_ok() {
        let ctx = ExecutionContext::new_empty();
        let payload = KindedSlot::from_int(42);
        let slot = KindedSlot::from_result(Arc::new(ResultData::ok(payload)));
        let wire = slot_to_wire(slot.raw(), slot.kind(), &ctx);
        match wire {
            WireValue::Result { ok, value } => {
                assert!(ok, "Ok(42) must wire with ok=true");
                assert_eq!(*value, WireValue::Integer(42));
            }
            other => panic!("expected WireValue::Result, got {:?}", other),
        }
    }

    #[test]
    fn err_result_projects_to_wire_result_err() {
        let ctx = ExecutionContext::new_empty();
        let payload = KindedSlot::from_int(7);
        let slot = KindedSlot::from_result(Arc::new(ResultData::err(payload)));
        let wire = slot_to_wire(slot.raw(), slot.kind(), &ctx);
        match wire {
            WireValue::Result { ok, value } => {
                assert!(!ok, "Err(7) must wire with ok=false");
                assert_eq!(*value, WireValue::Integer(7));
            }
            other => panic!("expected WireValue::Result, got {:?}", other),
        }
    }

    #[test]
    fn some_option_projects_to_inner_value() {
        let ctx = ExecutionContext::new_empty();
        let payload = KindedSlot::from_int(5);
        let slot = KindedSlot::from_option(Arc::new(OptionData::some(payload)));
        let wire = slot_to_wire(slot.raw(), slot.kind(), &ctx);
        // Null-coding semantics: `Some(x) ≡ x`.
        assert_eq!(wire, WireValue::Integer(5));
    }

    #[test]
    fn none_option_projects_to_null() {
        let ctx = ExecutionContext::new_empty();
        let slot = KindedSlot::from_option(Arc::new(OptionData::none()));
        let wire = slot_to_wire(slot.raw(), slot.kind(), &ctx);
        assert_eq!(wire, WireValue::Null);
    }
}

#[cfg(test)]
mod l5_typed_object_result_option_wire_tests {
    use super::{slot_to_wire, wire_to_slot};
    use crate::context::ExecutionContext;
    use crate::type_schema::builtin_schemas::{
        resolve_builtin_schema_ids, OPTION_PAYLOAD, OPTION_VARIANT_NONE, OPTION_VARIANT_SOME,
        RESULT_VARIANT_ERR, RESULT_VARIANT_OK,
    };
    use shape_value::{HeapKind, KindedSlot, NativeKind, TypedObjectStorage, ValueSlot};
    use shape_wire::WireValue;
    use std::sync::Arc;

    fn variant_object(schema_id: u64, variant: i64, payload: KindedSlot) -> KindedSlot {
        let payload_slot = payload.slot();
        let payload_kind = payload.kind();
        let payload_bits = payload_slot.raw();
        let heap_mask = if payload_bits != 0
            && matches!(
                payload_kind,
                NativeKind::String
                    | NativeKind::StringV2
                    | NativeKind::DecimalV2
                    | NativeKind::Ptr(_)
            ) {
            1u64 << OPTION_PAYLOAD
        } else {
            0
        };
        let field_kinds: Arc<[NativeKind]> =
            Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());
        let ptr = TypedObjectStorage::_new(
            schema_id,
            vec![ValueSlot::from_int(variant), payload_slot].into_boxed_slice(),
            heap_mask,
            field_kinds,
        );
        std::mem::forget(payload);
        KindedSlot::from_typed_object_raw(ptr)
    }

    #[test]
    fn schema_backed_result_projects_to_wire_result() {
        let ctx = ExecutionContext::new_empty();
        let schemas = resolve_builtin_schema_ids(ctx.type_schema_registry()).unwrap();
        let ok = variant_object(
            schemas.result as u64,
            RESULT_VARIANT_OK,
            KindedSlot::from_int(42),
        );
        let err = variant_object(
            schemas.result as u64,
            RESULT_VARIANT_ERR,
            KindedSlot::from_string_arc(Arc::new("bad".to_string())),
        );

        match slot_to_wire(ok.raw(), ok.kind(), &ctx) {
            WireValue::Result { ok, value } => {
                assert!(ok);
                assert_eq!(*value, WireValue::Integer(42));
            }
            other => panic!("expected wire Result Ok, got {other:?}"),
        }
        match slot_to_wire(err.raw(), err.kind(), &ctx) {
            WireValue::Result { ok, value } => {
                assert!(!ok);
                assert_eq!(*value, WireValue::String("bad".to_string()));
            }
            other => panic!("expected wire Result Err, got {other:?}"),
        }
    }

    #[test]
    fn schema_backed_option_projects_to_null_coding() {
        let ctx = ExecutionContext::new_empty();
        let schemas = resolve_builtin_schema_ids(ctx.type_schema_registry()).unwrap();
        let some = variant_object(
            schemas.option as u64,
            OPTION_VARIANT_SOME,
            KindedSlot::from_int(7),
        );
        let none = variant_object(
            schemas.option as u64,
            OPTION_VARIANT_NONE,
            KindedSlot::none(),
        );

        assert_eq!(
            slot_to_wire(some.raw(), some.kind(), &ctx),
            WireValue::Integer(7)
        );
        assert_eq!(slot_to_wire(none.raw(), none.kind(), &ctx), WireValue::Null);
    }

    #[test]
    fn wire_result_and_null_restore_to_schema_backed_typed_objects() {
        let ok = WireValue::Result {
            ok: true,
            value: Box::new(WireValue::Integer(11)),
        };
        let ok_bits =
            wire_to_slot(&ok, NativeKind::Ptr(HeapKind::TypedObject)).expect("restore ok");
        let ok_storage = unsafe { &*(ok_bits as *const TypedObjectStorage) };
        assert_eq!(ok_storage.slots()[0].as_i64(), RESULT_VARIANT_OK);
        assert_eq!(ok_storage.field_kinds[1], NativeKind::Int64);

        let none_bits = wire_to_slot(&WireValue::Null, NativeKind::Ptr(HeapKind::TypedObject))
            .expect("restore none");
        let none_storage = unsafe { &*(none_bits as *const TypedObjectStorage) };
        assert_eq!(none_storage.slots()[0].as_i64(), OPTION_VARIANT_NONE);
        assert_eq!(none_storage.field_kinds[1], NativeKind::Null);

        drop(KindedSlot::new(
            ValueSlot::from_raw(ok_bits),
            NativeKind::Ptr(HeapKind::TypedObject),
        ));
        drop(KindedSlot::new(
            ValueSlot::from_raw(none_bits),
            NativeKind::Ptr(HeapKind::TypedObject),
        ));
    }
}
