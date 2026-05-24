//! Runtime detection and uniform access for v2 typed arrays.
//!
//! v2 typed arrays are heap-allocated `TypedArray<T>` instances, where the
//! element type `T` is monomorphized at compile time. The bytecode compiler
//! emits typed allocation/push opcodes (e.g. `NewTypedArrayF64`,
//! `TypedArrayPushF64`) that create the right `TypedArray<T>` instantiation.
//!
//! However, generic consumer-side opcodes (`Length`, `GetProp`, `SetProp`,
//! `IterNext`) and generic method dispatch (`.len()`, `.first()`, `.last()`,
//! `.clone()`, `.sum()`, `.push()`, `.map()`, `.filter()`) only have a runtime
//! `(bits, NativeKind)` pair to inspect — they need to recognize the v2 typed
//! array pointer and dispatch to a typed implementation based on the element
//! type.
//!
//! ## Element type encoding
//!
//! The compile-time element type is preserved at runtime by stamping the
//! `_pad` byte (offset 7) of the `HeapHeader` with an `ElemType` discriminant.
//! This piggybacks on existing layout — no struct change required.
//!
//! Allocation handlers in `array.rs` stamp the byte after allocating;
//! consumer paths in this module read the byte to dispatch.
//!
//! ## ADR-006 §2.7.7 / Wave 6.5 cluster D-v2-array-detect
//!
//! API surface uses the kinded `(u64, NativeKind)` carrier shape. v2 typed
//! array pointers flow through the VM stack as raw `*mut TypedArray<T>` bits
//! tagged with `NativeKind::UInt64` (no Arc, no refcount — see
//! `v2_handlers/array.rs`). Detection rejects any other kind. Element reads
//! return the element's native bit pattern paired with the element's
//! `NativeKind` (Float64 / Int64 / Int32 / Bool). Writes accept the same
//! pair, decode bits per kind, and reject incompatible kinds.

use shape_value::NativeKind;
use shape_value::heap_value::TypedObjectStorage;
use shape_value::v2::decimal_obj::DecimalObj;
use shape_value::v2::heap_element::HeapElement;
use shape_value::v2::heap_header::{HEAP_KIND_V2_TYPED_ARRAY, HeapHeader};
use shape_value::v2::refcount::v2_retain;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::TypedArray;
use shape_value::HeapKind;

// ── Element type discriminants ──────────────────────────────────────────────
//
// r5c-2-β-δ-(α): the canonical discriminant definitions moved to
// `shape_value::v2::typed_array` so the kind-blind `release_v2_typed_array`
// helper there (called by the four `Ptr(HeapKind::TypedArray)` lockstep
// clone/drop tables, two of which live in the `shape-value` crate) can
// dispatch on the stamped `_pad` byte without a cross-crate constant
// duplication. The `pub use` below preserves every existing import path
// (`v2_array_detect::ELEM_TYPE_*`) for `shape-vm` / `shape-jit` consumers.
//
// W12 S1 sized-integer discriminants; Wave 2 Agent A1 F32 + Char; Wave 2
// Agent A2 String + Decimal; Phase 4b Round 4 W16.2-A TypedObject. ELEM_TYPE
// discriminant 10 stays reserved for `Array<u64>` (deferred per the S1
// reopen — Array<u64> excluded pending the §2.7.7/Q9 native-kind
// discriminator).
pub use shape_value::v2::typed_array::{
    ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64,
    ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_I8, ELEM_TYPE_STRING,
    ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U16, ELEM_TYPE_U32, ELEM_TYPE_U8, ELEM_TYPE_UNKNOWN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2ElemType {
    F64,
    I64,
    I32,
    Bool,
    // W12 S1 — sized-integer monomorphizations.
    I8,
    U8,
    I16,
    U16,
    U32,
    // U64 omitted — deferred to S1.5 per S1 reopen.
    // Wave 2 Agent A1 (2026-05-14) — F32 + Char scalar monomorphizations.
    F32,
    Char,
    // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element monomorphizations
    // per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit §3.2 S2-prime. Each is a v2-raw
    // heap-pointer carrier (`*const StringObj` / `*const DecimalObj`); element-read
    // pushes the carrier pointer with `NativeKind::StringV2` / `NativeKind::DecimalV2`
    // after per-element `v2_retain` of the header.
    String,
    Decimal,
    // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
    // v2-raw heap-pointer carrier (`*const TypedObjectStorage`); element-read
    // pushes the carrier pointer with `NativeKind::Ptr(HeapKind::TypedObject)`
    // after per-element `v2_retain` of the header.
    TypedObject,
}

impl V2ElemType {
    #[inline]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            ELEM_TYPE_F64 => Some(V2ElemType::F64),
            ELEM_TYPE_I64 => Some(V2ElemType::I64),
            ELEM_TYPE_I32 => Some(V2ElemType::I32),
            ELEM_TYPE_BOOL => Some(V2ElemType::Bool),
            ELEM_TYPE_I8 => Some(V2ElemType::I8),
            ELEM_TYPE_U8 => Some(V2ElemType::U8),
            ELEM_TYPE_I16 => Some(V2ElemType::I16),
            ELEM_TYPE_U16 => Some(V2ElemType::U16),
            ELEM_TYPE_U32 => Some(V2ElemType::U32),
            // Tag byte 10 (ELEM_TYPE_U64) reserved for S1.5; not produced
            // by any current allocation path.
            ELEM_TYPE_F32 => Some(V2ElemType::F32),
            ELEM_TYPE_CHAR => Some(V2ElemType::Char),
            ELEM_TYPE_STRING => Some(V2ElemType::String),
            ELEM_TYPE_DECIMAL => Some(V2ElemType::Decimal),
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
            ELEM_TYPE_TYPED_OBJECT => Some(V2ElemType::TypedObject),
            _ => None,
        }
    }

    /// Native kind of the array's elements (read result kind / write input
    /// kind family).
    #[inline]
    pub fn elem_kind(self) -> NativeKind {
        match self {
            V2ElemType::F64 => NativeKind::Float64,
            V2ElemType::I64 => NativeKind::Int64,
            V2ElemType::I32 => NativeKind::Int32,
            V2ElemType::Bool => NativeKind::Bool,
            V2ElemType::I8 => NativeKind::Int8,
            V2ElemType::U8 => NativeKind::UInt8,
            V2ElemType::I16 => NativeKind::Int16,
            V2ElemType::U16 => NativeKind::UInt16,
            V2ElemType::U32 => NativeKind::UInt32,
            V2ElemType::F32 => NativeKind::Float32,
            V2ElemType::Char => NativeKind::Char,
            V2ElemType::String => NativeKind::StringV2,
            V2ElemType::Decimal => NativeKind::DecimalV2,
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
            // element-read result carries the existing TypedObject pointer kind label.
            V2ElemType::TypedObject => {
                NativeKind::Ptr(shape_value::HeapKind::TypedObject)
            }
        }
    }
}

// ── Detection ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct V2TypedArrayView {
    pub ptr: *mut u8,
    pub elem_type: V2ElemType,
    pub len: u32,
}

/// Stamp the element type byte (`_pad` at offset 7 of the HeapHeader) on a
/// freshly-allocated v2 typed array.
#[inline]
pub unsafe fn stamp_elem_type(ptr: *mut u8, elem_type: u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let pad = ptr.add(7);
        *pad = elem_type;
    }
}

/// Read the element type byte from a v2 typed array's header.
#[inline]
unsafe fn read_elem_type_byte(ptr: *const u8) -> u8 {
    if ptr.is_null() {
        return ELEM_TYPE_UNKNOWN;
    }
    unsafe { *ptr.add(7) }
}

/// Try to interpret a `(bits, kind)` pair as a v2 typed array pointer.
///
/// v2 typed array pointers flow through the kinded API under a *single*
/// carrier kind — `NativeKind::Ptr(HeapKind::TypedArray)` — holding the raw
/// `*mut TypedArray<T>` pointer. This is the canonical carrier for every
/// producer: the `NewTypedArray*` allocation opcodes
/// (`v2_handlers/array.rs`), the `op_slice_access` / `op_array_push`
/// re-stash sites (`objects/array_operations.rs`), the struct-field array
/// read (`field_tag_to_native_kind` maps `FIELD_TAG_ARRAY` here) and the
/// closure capture (`closure_layout.rs::native_kind_from_concrete_type`
/// maps `ConcreteType::Array(_)` here). The slot's `clone_with_kind` /
/// `drop_with_kind` arms route through `retain_v2_typed_array` /
/// `release_v2_typed_array`. The on-header `HeapHeader.kind` check below
/// confirms the pointee is a genuine `TypedArray<T>`.
///
/// Any other `kind` is rejected — crucially `NativeKind::UInt64` is NOT
/// accepted. r5c-2-β-CKPT-C u64-carrier-disambiguation (2026-05-20): the
/// pre-fix arm `NativeKind::UInt64 | Ptr(HeapKind::TypedArray)` overloaded
/// the scalar-`u64` kind with the array-pointer carrier, so a genuine
/// scalar `u64` value (e.g. `u64::MAX`) reaching this function was
/// dereferenced as a `*const HeapHeader` → SIGSEGV (`let x: u64 = ...;
/// print(x)`). The producers now stamp the array carrier with
/// `Ptr(HeapKind::TypedArray)` exclusively, so the kind track itself is
/// the discriminator: a `UInt64` slot is unambiguously a scalar and never
/// reaches a pointer dereference here. NO value/low-address heuristic, NO
/// `is_heap()` probe — the kind track separates the two carriers
/// structurally (CLAUDE.md §"Parallel-implementation across
/// producer/consumer carrier-shape boundaries").
#[inline]
pub fn as_v2_typed_array(bits: u64, kind: NativeKind) -> Option<V2TypedArrayView> {
    if !matches!(kind, NativeKind::Ptr(HeapKind::TypedArray)) {
        return None;
    }
    if bits == 0 {
        return None;
    }
    let ptr = bits as usize as *mut u8;
    let header = unsafe { &*(ptr as *const HeapHeader) };
    if header.kind != HEAP_KIND_V2_TYPED_ARRAY {
        return None;
    }
    let elem_byte = unsafe { read_elem_type_byte(ptr) };
    let elem_type = V2ElemType::from_byte(elem_byte)?;
    let arr_u8 = ptr as *const TypedArray<u8>;
    let len = unsafe { (*arr_u8).len };
    Some(V2TypedArrayView {
        ptr,
        elem_type,
        len,
    })
}

// ── Bit/kind decode helpers (call-site, ADR-006 §2.7.6) ─────────────────────

/// Decode `(bits, kind)` to an `f64`. Accepts `Float64` directly and any
/// integer-family kind (cast to f64). Returns `None` on incompatible kinds.
#[inline]
fn decode_f64(bits: u64, kind: NativeKind) -> Option<f64> {
    if matches!(kind, NativeKind::Float64 | NativeKind::NullableFloat64) {
        return Some(f64::from_bits(bits));
    }
    if kind.is_integer_family() {
        return Some(decode_i64(bits, kind)? as f64);
    }
    None
}

/// Decode `(bits, kind)` to an `i64`. Accepts integer-family kinds with the
/// proper sign-extension; also accepts `Float64` (truncate). Returns `None`
/// on incompatible kinds.
#[inline]
fn decode_i64(bits: u64, kind: NativeKind) -> Option<i64> {
    match kind {
        NativeKind::Int64 | NativeKind::NullableInt64 => Some(bits as i64),
        NativeKind::Int32 | NativeKind::NullableInt32 => Some(bits as u32 as i32 as i64),
        NativeKind::Int16 | NativeKind::NullableInt16 => Some(bits as u16 as i16 as i64),
        NativeKind::Int8 | NativeKind::NullableInt8 => Some(bits as u8 as i8 as i64),
        NativeKind::IntSize | NativeKind::NullableIntSize => Some(bits as isize as i64),
        NativeKind::UInt64 | NativeKind::NullableUInt64 => Some(bits as i64),
        NativeKind::UInt32 | NativeKind::NullableUInt32 => Some(bits as u32 as i64),
        NativeKind::UInt16 | NativeKind::NullableUInt16 => Some(bits as u16 as i64),
        NativeKind::UInt8 | NativeKind::NullableUInt8 => Some(bits as u8 as i64),
        NativeKind::UIntSize | NativeKind::NullableUIntSize => Some(bits as usize as i64),
        NativeKind::Float64 | NativeKind::NullableFloat64 => Some(f64::from_bits(bits) as i64),
        _ => None,
    }
}

/// Decode `(bits, kind)` to a `bool`. Accepts only `NativeKind::Bool`.
#[inline]
fn decode_bool(bits: u64, kind: NativeKind) -> Option<bool> {
    if matches!(kind, NativeKind::Bool) {
        Some(bits != 0)
    } else {
        None
    }
}

/// Decode `(bits, kind)` to an `f32`. Accepts `Float32` directly (low 32
/// bits hold the f32 bit pattern), `Float64` (narrowed via cast), and any
/// integer-family kind (cast to f32). Returns `None` on incompatible kinds.
#[inline]
fn decode_f32(bits: u64, kind: NativeKind) -> Option<f32> {
    if matches!(kind, NativeKind::Float32) {
        return Some(f32::from_bits(bits as u32));
    }
    if let Some(v) = decode_f64(bits, kind) {
        return Some(v as f32);
    }
    None
}

/// Decode `(bits, kind)` to a `char`. Accepts `NativeKind::Char` directly
/// (bits are the codepoint per `KindedSlot::from_char`); for integer kinds
/// in the valid range (0..=0x10FFFF, excluding surrogates), produces the
/// corresponding `char`. Returns `None` on out-of-range codepoints or
/// incompatible kinds.
#[inline]
fn decode_char(bits: u64, kind: NativeKind) -> Option<char> {
    if matches!(kind, NativeKind::Char) {
        return char::from_u32(bits as u32);
    }
    if kind.is_integer_family() {
        let cp = decode_i64(bits, kind)?;
        if cp < 0 {
            return None;
        }
        return char::from_u32(cp as u32);
    }
    None
}

/// Read element `index` from a v2 typed array, returning `(bits, NativeKind)`.
///
/// The `NativeKind` is the element kind (`Float64` / `Int64` / `Int32` /
/// `Bool` / sized-integer kinds) — callers consume it directly without
/// further inspection.
#[inline]
pub fn read_element(view: &V2TypedArrayView, index: u32) -> Option<(u64, NativeKind)> {
    if index >= view.len {
        return None;
    }
    let pair = match view.elem_type {
        V2ElemType::F64 => unsafe {
            let arr = view.ptr as *const TypedArray<f64>;
            let v = TypedArray::<f64>::get_unchecked(arr, index);
            (v.to_bits(), NativeKind::Float64)
        },
        V2ElemType::I64 => unsafe {
            let arr = view.ptr as *const TypedArray<i64>;
            let v = TypedArray::<i64>::get_unchecked(arr, index);
            (v as u64, NativeKind::Int64)
        },
        V2ElemType::I32 => unsafe {
            let arr = view.ptr as *const TypedArray<i32>;
            let v = TypedArray::<i32>::get_unchecked(arr, index) as i64;
            (v as u64, NativeKind::Int32)
        },
        V2ElemType::Bool => unsafe {
            let arr = view.ptr as *const TypedArray<u8>;
            let v = TypedArray::<u8>::get_unchecked(arr, index) != 0;
            (v as u64, NativeKind::Bool)
        },
        // W12 S1 (2026-05-13) — sized-integer element reads.
        V2ElemType::I8 => unsafe {
            let arr = view.ptr as *const TypedArray<i8>;
            let v = TypedArray::<i8>::get_unchecked(arr, index) as i64;
            (v as u64, NativeKind::Int8)
        },
        V2ElemType::U8 => unsafe {
            let arr = view.ptr as *const TypedArray<u8>;
            let v = TypedArray::<u8>::get_unchecked(arr, index) as u64;
            (v, NativeKind::UInt8)
        },
        V2ElemType::I16 => unsafe {
            let arr = view.ptr as *const TypedArray<i16>;
            let v = TypedArray::<i16>::get_unchecked(arr, index) as i64;
            (v as u64, NativeKind::Int16)
        },
        V2ElemType::U16 => unsafe {
            let arr = view.ptr as *const TypedArray<u16>;
            let v = TypedArray::<u16>::get_unchecked(arr, index) as u64;
            (v, NativeKind::UInt16)
        },
        V2ElemType::U32 => unsafe {
            let arr = view.ptr as *const TypedArray<u32>;
            let v = TypedArray::<u32>::get_unchecked(arr, index) as u64;
            (v, NativeKind::UInt32)
        },
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char element reads.
        V2ElemType::F32 => unsafe {
            let arr = view.ptr as *const TypedArray<f32>;
            let v = TypedArray::<f32>::get_unchecked(arr, index);
            (v.to_bits() as u64, NativeKind::Float32)
        },
        V2ElemType::Char => unsafe {
            let arr = view.ptr as *const TypedArray<char>;
            let v = TypedArray::<char>::get_unchecked(arr, index);
            (v as u32 as u64, NativeKind::Char)
        },
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element reads.
        // Per audit §4.1.B.4 migration recipe: retain the element header before
        // pushing the slot bits — the array owns one share per stored pointer;
        // the caller of read_element gets a fresh share that must be released
        // when the slot is dropped (via NativeKind::StringV2 / DecimalV2 arm in
        // `clone_with_kind` / `drop_with_kind` lockstep per Agent B Round 1).
        V2ElemType::String => unsafe {
            let arr = view.ptr as *const TypedArray<*const StringObj>;
            let elem_ptr = TypedArray::<*const StringObj>::get_unchecked(arr, index);
            v2_retain(&(*elem_ptr).header);
            (elem_ptr as u64, NativeKind::StringV2)
        },
        V2ElemType::Decimal => unsafe {
            let arr = view.ptr as *const TypedArray<*const DecimalObj>;
            let elem_ptr = TypedArray::<*const DecimalObj>::get_unchecked(arr, index);
            v2_retain(&(*elem_ptr).header);
            (elem_ptr as u64, NativeKind::DecimalV2)
        },
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
        // mirror of the String/Decimal arms. The array owns one share per stored
        // pointer (refcount discipline on the on-header `v2_retain`/`v2_release`
        // counter; matches the existing single-TypedObject carrier in
        // `vm_impl/stack.rs:115`). The caller of read_element gets a fresh share
        // released by the matching `clone_with_kind` / `drop_with_kind`
        // `NativeKind::Ptr(HeapKind::TypedObject)` arm.
        V2ElemType::TypedObject => unsafe {
            let arr = view.ptr as *const TypedArray<*const TypedObjectStorage>;
            let elem_ptr =
                TypedArray::<*const TypedObjectStorage>::get_unchecked(arr, index);
            v2_retain(&(*elem_ptr).header);
            (elem_ptr as u64, NativeKind::Ptr(HeapKind::TypedObject))
        },
    };
    Some(pair)
}

/// Write `(bits, kind)` to element `index` of a v2 typed array.
#[inline]
pub fn write_element(
    view: &V2TypedArrayView,
    index: u32,
    bits: u64,
    kind: NativeKind,
) -> Result<(), &'static str> {
    if index >= view.len {
        return Err("index out of bounds");
    }
    match view.elem_type {
        V2ElemType::F64 => {
            let v = decode_f64(bits, kind).ok_or("expected f64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f64>;
                TypedArray::<f64>::set(arr, index, v);
            }
        }
        V2ElemType::I64 => {
            let v = decode_i64(bits, kind).ok_or("expected i64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i64>;
                TypedArray::<i64>::set(arr, index, v);
            }
        }
        V2ElemType::I32 => {
            let v = decode_i64(bits, kind).ok_or("expected i32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i32>;
                TypedArray::<i32>::set(arr, index, v as i32);
            }
        }
        V2ElemType::Bool => {
            let v = decode_bool(bits, kind).ok_or("expected bool value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::set(arr, index, if v { 1 } else { 0 });
            }
        }
        // W12 S1 (2026-05-13) — sized-integer element writes.
        V2ElemType::I8 => {
            let v = decode_i64(bits, kind).ok_or("expected i8-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i8>;
                TypedArray::<i8>::set(arr, index, v as i8);
            }
        }
        V2ElemType::U8 => {
            let v = decode_i64(bits, kind).ok_or("expected u8-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::set(arr, index, v as u8);
            }
        }
        V2ElemType::I16 => {
            let v = decode_i64(bits, kind).ok_or("expected i16-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i16>;
                TypedArray::<i16>::set(arr, index, v as i16);
            }
        }
        V2ElemType::U16 => {
            let v = decode_i64(bits, kind).ok_or("expected u16-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u16>;
                TypedArray::<u16>::set(arr, index, v as u16);
            }
        }
        V2ElemType::U32 => {
            let v = decode_i64(bits, kind).ok_or("expected u32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u32>;
                TypedArray::<u32>::set(arr, index, v as u32);
            }
        }
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char element writes.
        V2ElemType::F32 => {
            let v = decode_f32(bits, kind).ok_or("expected f32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f32>;
                TypedArray::<f32>::set(arr, index, v);
            }
        }
        V2ElemType::Char => {
            let v = decode_char(bits, kind).ok_or("expected char-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<char>;
                TypedArray::<char>::set(arr, index, v);
            }
        }
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element writes.
        // Per audit §4.1.B.4 migration recipe: release the prior element (the
        // array's owned share), transfer the caller's share to the array. Kind
        // mismatch refuses on sight — `NativeKind::String` (Phase-2c Arc<String>)
        // is structurally NOT the same carrier as `NativeKind::StringV2`
        // (v2-raw *const StringObj). No materialize-on-read fallback per
        // §4.1.B.3 forbidden patterns. Per Q25.A SUPERSEDED #3 mixed-migration
        // forbidden pattern, only StringV2 / DecimalV2 are accepted.
        V2ElemType::String => {
            if kind != NativeKind::StringV2 {
                return Err("expected NativeKind::StringV2 for Array<string> write");
            }
            let new_ptr = bits as usize as *const StringObj;
            unsafe {
                let arr = view.ptr as *mut TypedArray<*const StringObj>;
                let old_ptr = TypedArray::<*const StringObj>::get_unchecked(arr, index);
                <StringObj as HeapElement>::release_elem(old_ptr);
                TypedArray::<*const StringObj>::set(arr, index, new_ptr);
            }
        }
        V2ElemType::Decimal => {
            if kind != NativeKind::DecimalV2 {
                return Err("expected NativeKind::DecimalV2 for Array<decimal> write");
            }
            let new_ptr = bits as usize as *const DecimalObj;
            unsafe {
                let arr = view.ptr as *mut TypedArray<*const DecimalObj>;
                let old_ptr = TypedArray::<*const DecimalObj>::get_unchecked(arr, index);
                <DecimalObj as HeapElement>::release_elem(old_ptr);
                TypedArray::<*const DecimalObj>::set(arr, index, new_ptr);
            }
        }
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
        // mirror of the String/Decimal write arms. Kind discriminator strict:
        // only `NativeKind::Ptr(HeapKind::TypedObject)` accepted (matches the
        // single-TypedObject carrier label used elsewhere).
        V2ElemType::TypedObject => {
            if kind != NativeKind::Ptr(HeapKind::TypedObject) {
                return Err(
                    "expected NativeKind::Ptr(HeapKind::TypedObject) for Array<TypedObject> write",
                );
            }
            let new_ptr = bits as usize as *const TypedObjectStorage;
            unsafe {
                let arr = view.ptr as *mut TypedArray<*const TypedObjectStorage>;
                let old_ptr =
                    TypedArray::<*const TypedObjectStorage>::get_unchecked(arr, index);
                <TypedObjectStorage as HeapElement>::release_elem(old_ptr);
                TypedArray::<*const TypedObjectStorage>::set(arr, index, new_ptr);
            }
        }
    }
    Ok(())
}

/// Append `(bits, kind)` to a v2 typed array.
#[inline]
pub fn push_element(
    view: &V2TypedArrayView,
    bits: u64,
    kind: NativeKind,
) -> Result<(), &'static str> {
    match view.elem_type {
        V2ElemType::F64 => {
            let v = decode_f64(bits, kind).ok_or("expected f64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f64>;
                TypedArray::<f64>::push(arr, v);
            }
        }
        V2ElemType::I64 => {
            let v = decode_i64(bits, kind).ok_or("expected i64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i64>;
                TypedArray::<i64>::push(arr, v);
            }
        }
        V2ElemType::I32 => {
            let v = decode_i64(bits, kind).ok_or("expected i32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i32>;
                TypedArray::<i32>::push(arr, v as i32);
            }
        }
        V2ElemType::Bool => {
            let v = decode_bool(bits, kind).ok_or("expected bool value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::push(arr, if v { 1 } else { 0 });
            }
        }
        // W12 S1 (2026-05-13) — sized-integer element pushes.
        V2ElemType::I8 => {
            let v = decode_i64(bits, kind).ok_or("expected i8-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i8>;
                TypedArray::<i8>::push(arr, v as i8);
            }
        }
        V2ElemType::U8 => {
            let v = decode_i64(bits, kind).ok_or("expected u8-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::push(arr, v as u8);
            }
        }
        V2ElemType::I16 => {
            let v = decode_i64(bits, kind).ok_or("expected i16-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i16>;
                TypedArray::<i16>::push(arr, v as i16);
            }
        }
        V2ElemType::U16 => {
            let v = decode_i64(bits, kind).ok_or("expected u16-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u16>;
                TypedArray::<u16>::push(arr, v as u16);
            }
        }
        V2ElemType::U32 => {
            let v = decode_i64(bits, kind).ok_or("expected u32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u32>;
                TypedArray::<u32>::push(arr, v as u32);
            }
        }
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char element pushes.
        V2ElemType::F32 => {
            let v = decode_f32(bits, kind).ok_or("expected f32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f32>;
                TypedArray::<f32>::push(arr, v);
            }
        }
        V2ElemType::Char => {
            let v = decode_char(bits, kind).ok_or("expected char-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<char>;
                TypedArray::<char>::push(arr, v);
            }
        }
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element pushes.
        // Caller's refcount share transfers to the array (the array stores one
        // share per element; pop / drop_array_heap releases). Kind discriminator
        // refuses any non-StringV2 / DecimalV2 input per §2.7.5 stamp-at-compile-
        // time + Q25.A SUPERSEDED #3 mixed-migration forbidden pattern.
        V2ElemType::String => {
            if kind != NativeKind::StringV2 {
                return Err("expected NativeKind::StringV2 for Array<string> push");
            }
            let new_ptr = bits as usize as *const StringObj;
            unsafe {
                let arr = view.ptr as *mut TypedArray<*const StringObj>;
                TypedArray::<*const StringObj>::push(arr, new_ptr);
            }
        }
        V2ElemType::Decimal => {
            if kind != NativeKind::DecimalV2 {
                return Err("expected NativeKind::DecimalV2 for Array<decimal> push");
            }
            let new_ptr = bits as usize as *const DecimalObj;
            unsafe {
                let arr = view.ptr as *mut TypedArray<*const DecimalObj>;
                TypedArray::<*const DecimalObj>::push(arr, new_ptr);
            }
        }
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        V2ElemType::TypedObject => {
            if kind != NativeKind::Ptr(HeapKind::TypedObject) {
                return Err(
                    "expected NativeKind::Ptr(HeapKind::TypedObject) for Array<TypedObject> push",
                );
            }
            let new_ptr = bits as usize as *const TypedObjectStorage;
            unsafe {
                let arr = view.ptr as *mut TypedArray<*const TypedObjectStorage>;
                TypedArray::<*const TypedObjectStorage>::push(arr, new_ptr);
            }
        }
    }
    Ok(())
}

/// Pop the last element from a v2 typed array, returning `(bits, NativeKind)`.
#[inline]
pub fn pop_element(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    match view.elem_type {
        V2ElemType::F64 => unsafe {
            let arr = view.ptr as *mut TypedArray<f64>;
            TypedArray::<f64>::pop(arr).map(|v| (v.to_bits(), NativeKind::Float64))
        },
        V2ElemType::I64 => unsafe {
            let arr = view.ptr as *mut TypedArray<i64>;
            TypedArray::<i64>::pop(arr).map(|v| (v as u64, NativeKind::Int64))
        },
        V2ElemType::I32 => unsafe {
            let arr = view.ptr as *mut TypedArray<i32>;
            TypedArray::<i32>::pop(arr).map(|v| (v as i64 as u64, NativeKind::Int32))
        },
        V2ElemType::Bool => unsafe {
            let arr = view.ptr as *mut TypedArray<u8>;
            TypedArray::<u8>::pop(arr).map(|v| ((v != 0) as u64, NativeKind::Bool))
        },
        // W12 S1 (2026-05-13) — sized-integer element pops.
        V2ElemType::I8 => unsafe {
            let arr = view.ptr as *mut TypedArray<i8>;
            TypedArray::<i8>::pop(arr).map(|v| (v as i64 as u64, NativeKind::Int8))
        },
        V2ElemType::U8 => unsafe {
            let arr = view.ptr as *mut TypedArray<u8>;
            TypedArray::<u8>::pop(arr).map(|v| (v as u64, NativeKind::UInt8))
        },
        V2ElemType::I16 => unsafe {
            let arr = view.ptr as *mut TypedArray<i16>;
            TypedArray::<i16>::pop(arr).map(|v| (v as i64 as u64, NativeKind::Int16))
        },
        V2ElemType::U16 => unsafe {
            let arr = view.ptr as *mut TypedArray<u16>;
            TypedArray::<u16>::pop(arr).map(|v| (v as u64, NativeKind::UInt16))
        },
        V2ElemType::U32 => unsafe {
            let arr = view.ptr as *mut TypedArray<u32>;
            TypedArray::<u32>::pop(arr).map(|v| (v as u64, NativeKind::UInt32))
        },
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char element pops.
        V2ElemType::F32 => unsafe {
            let arr = view.ptr as *mut TypedArray<f32>;
            TypedArray::<f32>::pop(arr).map(|v| (v.to_bits() as u64, NativeKind::Float32))
        },
        V2ElemType::Char => unsafe {
            let arr = view.ptr as *mut TypedArray<char>;
            TypedArray::<char>::pop(arr).map(|v| (v as u32 as u64, NativeKind::Char))
        },
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element pops.
        // Transfer the array's owned share to the caller (the slot bits carry
        // an owning share; caller is responsible for releasing via the
        // StringV2 / DecimalV2 arm in drop_with_kind). No additional retain.
        V2ElemType::String => unsafe {
            let arr = view.ptr as *mut TypedArray<*const StringObj>;
            TypedArray::<*const StringObj>::pop(arr).map(|v| (v as u64, NativeKind::StringV2))
        },
        V2ElemType::Decimal => unsafe {
            let arr = view.ptr as *mut TypedArray<*const DecimalObj>;
            TypedArray::<*const DecimalObj>::pop(arr).map(|v| (v as u64, NativeKind::DecimalV2))
        },
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        // Transfer the array's owned share to the caller; release runs via the
        // `clone_with_kind` / `drop_with_kind` `Ptr(HeapKind::TypedObject)` arm.
        V2ElemType::TypedObject => unsafe {
            let arr = view.ptr as *mut TypedArray<*const TypedObjectStorage>;
            TypedArray::<*const TypedObjectStorage>::pop(arr)
                .map(|v| (v as u64, NativeKind::Ptr(HeapKind::TypedObject)))
        },
    }
}

/// Sum all elements of a numeric (F64/I64/I32) v2 typed array.
///
/// F64 and I64 variants use `wide::f64x4`/`wide::i64x4` SIMD reduction on
/// arrays with >= `SIMD_SUM_THRESHOLD` elements, delivering ~4x throughput
/// on AVX2-capable CPUs. Smaller arrays fall back to scalar accumulation
/// where the SIMD setup overhead would exceed the savings.
///
/// Returns `(bits, NativeKind::Float64)` for F64 inputs and
/// `(bits, NativeKind::Int64)` for integer inputs. `None` for Bool inputs.
pub fn sum_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    /// Minimum element count at which SIMD reduction beats scalar accumulation.
    /// Determined empirically — below this, vector load/splat overhead dominates.
    const SIMD_SUM_THRESHOLD: u32 = 16;

    match view.elem_type {
        V2ElemType::F64 => {
            let len = view.len;
            if len == 0 {
                return Some((0.0_f64.to_bits(), NativeKind::Float64));
            }
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<f64>;
                (*arr).data as *const f64
            };
            let s = unsafe { simd_sum_f64(data, len as usize, SIMD_SUM_THRESHOLD as usize) };
            Some((s.to_bits(), NativeKind::Float64))
        }
        V2ElemType::I64 => {
            let len = view.len;
            if len == 0 {
                return Some((0u64, NativeKind::Int64));
            }
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<i64>;
                (*arr).data as *const i64
            };
            let s = unsafe { simd_sum_i64(data, len as usize, SIMD_SUM_THRESHOLD as usize) };
            Some((s as u64, NativeKind::Int64))
        }
        V2ElemType::I32 => {
            let mut s: i64 = 0;
            for i in 0..view.len {
                let val = unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as i64
                };
                s = s.wrapping_add(val);
            }
            Some((s as u64, NativeKind::Int64))
        }
        // W12 S1 — sum/avg/min/max/variance/std/dot/norm not defined for
        // Bool or sized-integer-narrower-than-i64 element kinds. The
        // caller falls back to a non-SIMD path or returns an error.
        // Wave 2 Agent A1 — F32 / Char also fall through; F32 reductions
        // are domain-deferred to a follow-up SIMD lane sub-cluster.
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::F32
        | V2ElemType::Char
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element variants
        // have no numeric sum semantics; concat for String is a method-level
        // operation, not a sum reduction.
        | V2ElemType::String
        | V2ElemType::Decimal
        | V2ElemType::TypedObject => None,
    }
}

/// SIMD-accelerated f64 sum using `wide::f64x4` lanes.
///
/// # Safety
/// `data` must point to at least `len` valid, contiguous `f64` values.
#[inline]
unsafe fn simd_sum_f64(data: *const f64, len: usize, threshold: usize) -> f64 {
    use wide::f64x4;

    if len < threshold {
        let mut s = 0.0_f64;
        for i in 0..len {
            s += unsafe { *data.add(i) };
        }
        return s;
    }

    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let base = i * 4;
        let v = unsafe {
            f64x4::from([
                *data.add(base),
                *data.add(base + 1),
                *data.add(base + 2),
                *data.add(base + 3),
            ])
        };
        acc += v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        s += unsafe { *data.add(i) };
    }
    s
}

/// Scan a f64 buffer for any NaN. Used to short-circuit min/max where
/// hardware `min_pd`/`max_pd` don't reliably propagate NaN.
///
/// # Safety
/// `data` must point to at least `len` valid `f64` values.
#[inline]
unsafe fn contains_nan_f64(data: *const f64, len: usize) -> bool {
    for i in 0..len {
        if unsafe { *data.add(i) }.is_nan() {
            return true;
        }
    }
    false
}

/// SIMD-accelerated f64 minimum using `wide::f64x4::fast_min`. Falls back to
/// a scalar loop below the threshold. Requires `len > 0`.
///
/// Hardware `min_pd` returns the non-NaN operand rather than propagating
/// NaN, so we scan for NaN up front to match scalar `f64::min` semantics.
///
/// # Safety
/// `data` must point to at least `len` valid, contiguous `f64` values and
/// `len` must be at least 1.
#[inline]
unsafe fn simd_min_f64(data: *const f64, len: usize, threshold: usize) -> f64 {
    use wide::f64x4;
    debug_assert!(len > 0);
    if unsafe { contains_nan_f64(data, len) } {
        return f64::NAN;
    }
    if len < threshold {
        let mut m = unsafe { *data };
        for i in 1..len {
            let v = unsafe { *data.add(i) };
            if v < m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = unsafe {
        f64x4::from([
            *data,
            *data.add(1),
            *data.add(2),
            *data.add(3),
        ])
    };
    for i in 1..chunks {
        let base = i * 4;
        let v = unsafe {
            f64x4::from([
                *data.add(base),
                *data.add(base + 1),
                *data.add(base + 2),
                *data.add(base + 3),
            ])
        };
        acc = acc.fast_min(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p < m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        if v < m {
            m = v;
        }
    }
    m
}

/// SIMD-accelerated f64 maximum. Mirrors [`simd_min_f64`].
///
/// # Safety
/// See [`simd_min_f64`].
#[inline]
unsafe fn simd_max_f64(data: *const f64, len: usize, threshold: usize) -> f64 {
    use wide::f64x4;
    debug_assert!(len > 0);
    if unsafe { contains_nan_f64(data, len) } {
        return f64::NAN;
    }
    if len < threshold {
        let mut m = unsafe { *data };
        for i in 1..len {
            let v = unsafe { *data.add(i) };
            if v > m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = unsafe {
        f64x4::from([
            *data,
            *data.add(1),
            *data.add(2),
            *data.add(3),
        ])
    };
    for i in 1..chunks {
        let base = i * 4;
        let v = unsafe {
            f64x4::from([
                *data.add(base),
                *data.add(base + 1),
                *data.add(base + 2),
                *data.add(base + 3),
            ])
        };
        acc = acc.fast_max(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p > m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        if v > m {
            m = v;
        }
    }
    m
}

/// SIMD-accelerated i64 sum using `wide::i64x4` lanes.
///
/// Uses `wrapping_add` semantics at the lane level (Shape's int sum on Vec<int>
/// never panics on overflow for the v2 path — matches scalar `wrapping_add`).
///
/// # Safety
/// `data` must point to at least `len` valid, contiguous `i64` values.
#[inline]
unsafe fn simd_sum_i64(data: *const i64, len: usize, threshold: usize) -> i64 {
    use wide::i64x4;

    if len < threshold {
        let mut s: i64 = 0;
        for i in 0..len {
            s = s.wrapping_add(unsafe { *data.add(i) });
        }
        return s;
    }

    let chunks = len / 4;
    let mut acc = i64x4::splat(0);
    for i in 0..chunks {
        let base = i * 4;
        let v = unsafe {
            i64x4::from([
                *data.add(base),
                *data.add(base + 1),
                *data.add(base + 2),
                *data.add(base + 3),
            ])
        };
        // wide::i64x4 uses wrapping add on overflow. It does not implement
        // AddAssign, so reassign via the binary + operator.
        acc = acc + v;
    }
    let parts = acc.to_array();
    let mut s = parts[0]
        .wrapping_add(parts[1])
        .wrapping_add(parts[2])
        .wrapping_add(parts[3]);
    for i in (chunks * 4)..len {
        s = s.wrapping_add(unsafe { *data.add(i) });
    }
    s
}

/// Compute the average (mean) of all elements of a numeric v2 typed array.
/// Returns NaN for empty arrays. Returns `(bits, NativeKind::Float64)` always
/// (mean of integer arrays is a float).
pub fn avg_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    if view.len == 0 {
        return match view.elem_type {
            V2ElemType::F64 | V2ElemType::I64 | V2ElemType::I32 => {
                Some((f64::NAN.to_bits(), NativeKind::Float64))
            }
            // W12 S1 — sized-integer narrow kinds and Bool don't have an
            // empty-array mean sentinel at this layer; caller surfaces None.
            V2ElemType::Bool
            | V2ElemType::I8
            | V2ElemType::U8
            | V2ElemType::I16
            | V2ElemType::U16
            | V2ElemType::U32
            | V2ElemType::F32
            | V2ElemType::Char
            | V2ElemType::String
            | V2ElemType::Decimal
            | V2ElemType::TypedObject => None,
        };
    }
    match view.elem_type {
        V2ElemType::F64 => {
            // Reuse the SIMD sum path; below threshold it runs the scalar
            // fallback internally so small arrays still see the simple loop.
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<f64>;
                (*arr).data as *const f64
            };
            let s = unsafe { simd_sum_f64(data, view.len as usize, 16) };
            Some(((s / view.len as f64).to_bits(), NativeKind::Float64))
        }
        V2ElemType::I64 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<i64>;
                    TypedArray::<i64>::get_unchecked(arr, i) as f64
                };
            }
            Some(((s / view.len as f64).to_bits(), NativeKind::Float64))
        }
        V2ElemType::I32 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as f64
                };
            }
            Some(((s / view.len as f64).to_bits(), NativeKind::Float64))
        }
        // W12 S1 — sum/avg/min/max/variance/std/dot/norm not defined for
        // Bool or sized-integer-narrower-than-i64 element kinds. The
        // caller falls back to a non-SIMD path or returns an error.
        // Wave 2 Agent A1 — F32 / Char also fall through; F32 reductions
        // are domain-deferred to a follow-up SIMD lane sub-cluster.
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::F32
        | V2ElemType::Char
        | V2ElemType::String
        | V2ElemType::Decimal
        | V2ElemType::TypedObject => None,
    }
}

/// Compute the minimum element of a numeric v2 typed array.
///
/// Empty arrays return:
///   - F64 input → `(NaN.to_bits(), Float64)`
///   - I64/I32 input → `(0, Bool)` (the §2.7 null/unit sentinel)
///   - Bool input → `None`
pub fn min_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    if view.len == 0 {
        return match view.elem_type {
            V2ElemType::F64 => Some((f64::NAN.to_bits(), NativeKind::Float64)),
            V2ElemType::I64 | V2ElemType::I32 => Some((0u64, NativeKind::Bool)),
            // W12 S1 — narrow-int and Bool element kinds have no canonical
            // empty-array sentinel for min/max; caller treats None as a
            // runtime error per §2.7 sentinel discipline.
            V2ElemType::Bool
            | V2ElemType::I8
            | V2ElemType::U8
            | V2ElemType::I16
            | V2ElemType::U16
            | V2ElemType::U32
            | V2ElemType::F32
            | V2ElemType::Char
            | V2ElemType::String
            | V2ElemType::Decimal
            | V2ElemType::TypedObject => None,
        };
    }
    match view.elem_type {
        V2ElemType::F64 => {
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<f64>;
                (*arr).data as *const f64
            };
            let min = unsafe { simd_min_f64(data, view.len as usize, 16) };
            Some((min.to_bits(), NativeKind::Float64))
        }
        V2ElemType::I64 => {
            let mut min = i64::MAX;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<i64>;
                    TypedArray::<i64>::get_unchecked(arr, i)
                };
                if v < min {
                    min = v;
                }
            }
            Some((min as u64, NativeKind::Int64))
        }
        V2ElemType::I32 => {
            let mut min = i32::MAX as i64;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as i64
                };
                if v < min {
                    min = v;
                }
            }
            Some((min as u64, NativeKind::Int64))
        }
        // W12 S1 — sum/avg/min/max/variance/std/dot/norm not defined for
        // Bool or sized-integer-narrower-than-i64 element kinds. The
        // caller falls back to a non-SIMD path or returns an error.
        // Wave 2 Agent A1 — F32 / Char also fall through; F32 reductions
        // are domain-deferred to a follow-up SIMD lane sub-cluster.
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::F32
        | V2ElemType::Char
        | V2ElemType::String
        | V2ElemType::Decimal
        | V2ElemType::TypedObject => None,
    }
}

/// Compute the maximum element of a numeric v2 typed array.
pub fn max_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    if view.len == 0 {
        return match view.elem_type {
            V2ElemType::F64 => Some((f64::NAN.to_bits(), NativeKind::Float64)),
            V2ElemType::I64 | V2ElemType::I32 => Some((0u64, NativeKind::Bool)),
            // W12 S1 — narrow-int and Bool element kinds have no canonical
            // empty-array sentinel for min/max; caller treats None as a
            // runtime error per §2.7 sentinel discipline.
            V2ElemType::Bool
            | V2ElemType::I8
            | V2ElemType::U8
            | V2ElemType::I16
            | V2ElemType::U16
            | V2ElemType::U32
            | V2ElemType::F32
            | V2ElemType::Char
            | V2ElemType::String
            | V2ElemType::Decimal
            | V2ElemType::TypedObject => None,
        };
    }
    match view.elem_type {
        V2ElemType::F64 => {
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<f64>;
                (*arr).data as *const f64
            };
            let max = unsafe { simd_max_f64(data, view.len as usize, 16) };
            Some((max.to_bits(), NativeKind::Float64))
        }
        V2ElemType::I64 => {
            let mut max = i64::MIN;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<i64>;
                    TypedArray::<i64>::get_unchecked(arr, i)
                };
                if v > max {
                    max = v;
                }
            }
            Some((max as u64, NativeKind::Int64))
        }
        V2ElemType::I32 => {
            let mut max = i32::MIN as i64;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as i64
                };
                if v > max {
                    max = v;
                }
            }
            Some((max as u64, NativeKind::Int64))
        }
        // W12 S1 — sum/avg/min/max/variance/std/dot/norm not defined for
        // Bool or sized-integer-narrower-than-i64 element kinds. The
        // caller falls back to a non-SIMD path or returns an error.
        // Wave 2 Agent A1 — F32 / Char also fall through; F32 reductions
        // are domain-deferred to a follow-up SIMD lane sub-cluster.
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::F32
        | V2ElemType::Char
        | V2ElemType::String
        | V2ElemType::Decimal
        | V2ElemType::TypedObject => None,
    }
}

/// Compute the sample variance of a float v2 typed array.
/// Returns NaN for arrays with fewer than 2 elements. Always returns Float64.
pub fn variance_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    match view.elem_type {
        V2ElemType::F64 => {
            if view.len < 2 {
                return Some((f64::NAN.to_bits(), NativeKind::Float64));
            }
            let n = view.len as f64;
            let mut sum = 0.0_f64;
            for i in 0..view.len {
                sum += unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
            }
            let mean = sum / n;
            let mut var_sum = 0.0_f64;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
                let d = v - mean;
                var_sum += d * d;
            }
            Some(((var_sum / (n - 1.0)).to_bits(), NativeKind::Float64))
        }
        _ => None,
    }
}

/// Compute the sample standard deviation of a float v2 typed array.
pub fn std_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    variance_elements(view).map(|(bits, _kind)| {
        let v = f64::from_bits(bits);
        (v.sqrt().to_bits(), NativeKind::Float64)
    })
}

/// Compute the dot product of two float v2 typed arrays.
pub fn dot_elements(
    view_a: &V2TypedArrayView,
    view_b: &V2TypedArrayView,
) -> Option<(u64, NativeKind)> {
    if view_a.elem_type != V2ElemType::F64 || view_b.elem_type != V2ElemType::F64 {
        return None;
    }
    if view_a.len != view_b.len {
        return None; // caller should produce an error
    }
    let mut sum = 0.0_f64;
    for i in 0..view_a.len {
        let a = unsafe {
            let arr = view_a.ptr as *const TypedArray<f64>;
            TypedArray::<f64>::get_unchecked(arr, i)
        };
        let b = unsafe {
            let arr = view_b.ptr as *const TypedArray<f64>;
            TypedArray::<f64>::get_unchecked(arr, i)
        };
        sum += a * b;
    }
    Some((sum.to_bits(), NativeKind::Float64))
}

/// Compute the Euclidean norm of a float v2 typed array.
pub fn norm_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    match view.elem_type {
        V2ElemType::F64 => {
            let mut sum_sq = 0.0_f64;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
                sum_sq += v * v;
            }
            Some((sum_sq.sqrt().to_bits(), NativeKind::Float64))
        }
        _ => None,
    }
}

/// Count `true` values in a bool v2 typed array. Returns `(count, Int64)`.
pub fn count_true_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    match view.elem_type {
        V2ElemType::Bool => {
            let mut count = 0_i64;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v != 0 {
                    count += 1;
                }
            }
            Some((count as u64, NativeKind::Int64))
        }
        _ => None,
    }
}

/// Check if any element in a bool v2 typed array is true.
pub fn any_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    match view.elem_type {
        V2ElemType::Bool => {
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v != 0 {
                    return Some((1u64, NativeKind::Bool));
                }
            }
            Some((0u64, NativeKind::Bool))
        }
        _ => None,
    }
}

/// Check if all elements in a bool v2 typed array are true.
pub fn all_elements(view: &V2TypedArrayView) -> Option<(u64, NativeKind)> {
    match view.elem_type {
        V2ElemType::Bool => {
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v == 0 {
                    return Some((0u64, NativeKind::Bool));
                }
            }
            Some((1u64, NativeKind::Bool))
        }
        _ => None,
    }
}

/// Allocate a fresh v2 typed array, copy all elements from `view`, stamp
/// elem_type, and return its raw pointer.
pub fn clone_array(view: &V2TypedArrayView) -> *mut u8 {
    match view.elem_type {
        V2ElemType::F64 => {
            let new_arr = TypedArray::<f64>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<f64>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_F64);
                p
            }
        }
        V2ElemType::I64 => {
            let new_arr = TypedArray::<i64>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i64>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I64);
                p
            }
        }
        V2ElemType::I32 => {
            let new_arr = TypedArray::<i32>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i32>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I32);
                p
            }
        }
        V2ElemType::Bool => {
            let new_arr = TypedArray::<u8>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u8>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_BOOL);
                p
            }
        }
        // W12 S1 (2026-05-13) — sized-integer element clone implementations.
        // Each variant allocates a fresh `TypedArray<T>` with matching `T`,
        // memcpy's the element buffer, and stamps the proper `ELEM_TYPE_X`
        // byte so subsequent `as_v2_typed_array` calls dispatch correctly.
        V2ElemType::I8 => {
            let new_arr = TypedArray::<i8>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i8>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I8);
                p
            }
        }
        V2ElemType::U8 => {
            let new_arr = TypedArray::<u8>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u8>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U8);
                p
            }
        }
        V2ElemType::I16 => {
            let new_arr = TypedArray::<i16>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i16>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I16);
                p
            }
        }
        V2ElemType::U16 => {
            let new_arr = TypedArray::<u16>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u16>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U16);
                p
            }
        }
        V2ElemType::U32 => {
            let new_arr = TypedArray::<u32>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u32>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U32);
                p
            }
        }
        // V2ElemType::U64 omitted — deferred to S1.5 per S1 reopen.
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char element clone.
        V2ElemType::F32 => {
            let new_arr = TypedArray::<f32>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<f32>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_F32);
                p
            }
        }
        V2ElemType::Char => {
            let new_arr = TypedArray::<char>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<char>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_CHAR);
                p
            }
        }
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal element clone.
        // Each clone shares the same heap-element pointers as the source array
        // (no deep copy of the StringObj / DecimalObj allocations themselves);
        // we retain per-element so both arrays own valid shares.
        V2ElemType::String => {
            let new_arr = TypedArray::<*const StringObj>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<*const StringObj>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    for i in 0..(view.len as usize) {
                        let elem = *src_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_STRING);
                p
            }
        }
        V2ElemType::Decimal => {
            let new_arr = TypedArray::<*const DecimalObj>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<*const DecimalObj>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    for i in 0..(view.len as usize) {
                        let elem = *src_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_DECIMAL);
                p
            }
        }
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        // Each clone shares the same heap-element pointers as the source array
        // (no deep copy of the TypedObjectStorage allocations themselves);
        // retain per-element so both arrays own valid shares — mirror of the
        // String/Decimal clone arms above.
        V2ElemType::TypedObject => {
            let new_arr =
                TypedArray::<*const TypedObjectStorage>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<*const TypedObjectStorage>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    for i in 0..(view.len as usize) {
                        let elem = *src_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_TYPED_OBJECT);
                p
            }
        }
    }
}

// ── PC.2: SIMD-vectorized unary element-wise transforms on F64 views ────────
//
// These helpers produce a fresh v2 `TypedArray<f64>` by applying a pure
// element-wise function to each f64 element of `view`. The allocation stamps
// `ELEM_TYPE_F64` so the result is a first-class v2 typed array recognizable
// by downstream `.sum()` / `.map()` / etc.
//
// `simd_op`/`scalar_op` mirror the pattern used in the shape-runtime
// `intrinsic_vec_*` helpers. Arrays at or above `SIMD_UNARY_THRESHOLD` take
// the `wide::f64x4` fast path; smaller arrays fall back to scalar to avoid
// SIMD setup overhead.
//
// Callers use these via `dispatch_v2_typed_array_method` to implement
// `.abs()`, `.sqrt()`, `.ln()`, `.exp()` on v2 typed arrays. For non-F64
// element types the helper returns `None`, triggering the caller's legacy
// fallback.

/// Minimum F64 element count at which unary SIMD transforms beat scalar.
/// Matches [`SIMD_SUM_THRESHOLD`]; determined empirically.
const SIMD_UNARY_THRESHOLD: u32 = 16;

/// Apply a unary element-wise f64 transform to `view`, returning a newly
/// allocated v2 `TypedArray<f64>` pointer with `ELEM_TYPE_F64` stamped.
///
/// `simd_op` must be the `wide::f64x4` form of `scalar_op`; this is checked
/// by the parity tests in `typed_array_methods::tests`.
///
/// Returns `None` for non-F64 element types — the caller should fall back to
/// the legacy FLOAT_ARRAY_METHODS handler after materializing.
pub fn unary_f64_transform(
    view: &V2TypedArrayView,
    simd_op: fn(wide::f64x4) -> wide::f64x4,
    scalar_op: fn(f64) -> f64,
) -> Option<*mut u8> {
    use wide::f64x4;

    if view.elem_type != V2ElemType::F64 {
        return None;
    }
    let len = view.len;
    let out = TypedArray::<f64>::with_capacity(len);
    if len == 0 {
        unsafe {
            (*out).len = 0;
            let p = out as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F64);
            return Some(p);
        }
    }

    unsafe {
        let src_arr = view.ptr as *const TypedArray<f64>;
        let src = (*src_arr).data as *const f64;
        let dst = (*out).data as *mut f64;

        if len >= SIMD_UNARY_THRESHOLD {
            let chunks = (len / 4) as usize;
            for i in 0..chunks {
                let base = i * 4;
                let v = f64x4::from([
                    *src.add(base),
                    *src.add(base + 1),
                    *src.add(base + 2),
                    *src.add(base + 3),
                ]);
                let r = simd_op(v);
                let arr = r.to_array();
                *dst.add(base) = arr[0];
                *dst.add(base + 1) = arr[1];
                *dst.add(base + 2) = arr[2];
                *dst.add(base + 3) = arr[3];
            }
            for i in (chunks * 4)..(len as usize) {
                *dst.add(i) = scalar_op(*src.add(i));
            }
        } else {
            for i in 0..(len as usize) {
                *dst.add(i) = scalar_op(*src.add(i));
            }
        }

        (*out).len = len;
        let p = out as *mut u8;
        stamp_elem_type(p, ELEM_TYPE_F64);
        Some(p)
    }
}

/// Stride-1 consecutive differences (`out[i] = src[i+1] - src[i]`) over a
/// v2 F64 typed array. Returns a fresh v2 `TypedArray<f64>` of length
/// `view.len - 1` (empty for `len < 2`). SIMD-accelerated via `f64x4` for
/// sufficiently large inputs (PC.2).
///
/// Returns `None` for non-F64 element types.
pub fn diff_f64(view: &V2TypedArrayView) -> Option<*mut u8> {
    use wide::f64x4;

    if view.elem_type != V2ElemType::F64 {
        return None;
    }
    let len = view.len;
    if len < 2 {
        let out = TypedArray::<f64>::with_capacity(0);
        unsafe {
            (*out).len = 0;
            let p = out as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F64);
            return Some(p);
        }
    }

    let out_len = len - 1;
    let out = TypedArray::<f64>::with_capacity(out_len);
    unsafe {
        let src_arr = view.ptr as *const TypedArray<f64>;
        let src = (*src_arr).data as *const f64;
        let dst = (*out).data as *mut f64;

        if out_len >= SIMD_UNARY_THRESHOLD {
            let mut i: usize = 0;
            // While we can still load `src[i+1 .. i+5]`, step 4 at a time.
            while i + 4 < (len as usize) {
                let prev = f64x4::from([
                    *src.add(i),
                    *src.add(i + 1),
                    *src.add(i + 2),
                    *src.add(i + 3),
                ]);
                let next = f64x4::from([
                    *src.add(i + 1),
                    *src.add(i + 2),
                    *src.add(i + 3),
                    *src.add(i + 4),
                ]);
                let d = next - prev;
                let arr = d.to_array();
                *dst.add(i) = arr[0];
                *dst.add(i + 1) = arr[1];
                *dst.add(i + 2) = arr[2];
                *dst.add(i + 3) = arr[3];
                i += 4;
            }
            // Scalar tail: remaining `out_len - i` differences.
            for j in i..(out_len as usize) {
                *dst.add(j) = *src.add(j + 1) - *src.add(j);
            }
        } else {
            for i in 0..(out_len as usize) {
                *dst.add(i) = *src.add(i + 1) - *src.add(i);
            }
        }

        (*out).len = out_len;
        let p = out as *mut u8;
        stamp_elem_type(p, ELEM_TYPE_F64);
        Some(p)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// J.5a non-blocking primitives (2026-05-24)
//
// Per `docs/cluster-audits/v0.3-j4-rest-reaudit.md` §6 J.5a row: kind-generic
// reverse / concat / slice / take / drop primitives over the existing 14-arm
// `V2ElemType` dispatch shape. Each primitive mirrors the `clone_array`
// scaffold (`v2_array_detect.rs:1429-1670`): allocate a fresh `TypedArray<T>`
// with the same element type, copy the relevant range, retain per-element for
// heap-element kinds (String / Decimal / TypedObject), stamp the result's
// element-type byte, return the raw pointer.
//
// Refusal #10 binding (re-audit §6): `flatten` is NOT implemented here —
// the outer carrier shape (`TypedArray<*const TypedArray<T>>`) requires the
// J.5d tuple-carrier-class architectural decision (§3 of the re-audit). The
// `handle_flatten_v2` site remains surface-and-stop pending that gate.
// ═══════════════════════════════════════════════════════════════════════════

/// Produce a reversed copy of `view`. Kind-generic over the 14 `V2ElemType`
/// variants. For heap-element variants (String / Decimal / TypedObject) the
/// new array's pointers share the same heap targets as the source; each is
/// retained per-element so both arrays own valid shares.
///
/// Allocator + stamp shape mirrors `clone_array`; only the per-element copy
/// loop differs (reverse iteration).
pub fn reverse_array(view: &V2TypedArrayView) -> *mut u8 {
    // Helper: copy `Copy` scalar elements in reverse order.
    #[inline]
    unsafe fn copy_reverse_scalar<T: Copy>(
        src_data: *const T,
        dst_data: *mut T,
        len: usize,
    ) {
        if len == 0 || src_data.is_null() || dst_data.is_null() {
            return;
        }
        for i in 0..len {
            unsafe {
                *dst_data.add(i) = *src_data.add(len - 1 - i);
            }
        }
    }

    match view.elem_type {
        V2ElemType::F64 => {
            let new_arr = TypedArray::<f64>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<f64>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_F64);
                p
            }
        }
        V2ElemType::I64 => {
            let new_arr = TypedArray::<i64>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i64>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I64);
                p
            }
        }
        V2ElemType::I32 => {
            let new_arr = TypedArray::<i32>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i32>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I32);
                p
            }
        }
        V2ElemType::Bool => {
            let new_arr = TypedArray::<u8>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u8>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_BOOL);
                p
            }
        }
        V2ElemType::I8 => {
            let new_arr = TypedArray::<i8>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i8>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I8);
                p
            }
        }
        V2ElemType::U8 => {
            let new_arr = TypedArray::<u8>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u8>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U8);
                p
            }
        }
        V2ElemType::I16 => {
            let new_arr = TypedArray::<i16>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<i16>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_I16);
                p
            }
        }
        V2ElemType::U16 => {
            let new_arr = TypedArray::<u16>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u16>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U16);
                p
            }
        }
        V2ElemType::U32 => {
            let new_arr = TypedArray::<u32>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u32>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U32);
                p
            }
        }
        V2ElemType::F32 => {
            let new_arr = TypedArray::<f32>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<f32>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_F32);
                p
            }
        }
        V2ElemType::Char => {
            let new_arr = TypedArray::<char>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<char>;
                copy_reverse_scalar((*src).data, (*new_arr).data, view.len as usize);
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_CHAR);
                p
            }
        }
        V2ElemType::String => {
            let new_arr = TypedArray::<*const StringObj>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<*const StringObj>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                let len = view.len as usize;
                if len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    for i in 0..len {
                        let elem = *src_data.add(len - 1 - i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_STRING);
                p
            }
        }
        V2ElemType::Decimal => {
            let new_arr = TypedArray::<*const DecimalObj>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<*const DecimalObj>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                let len = view.len as usize;
                if len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    for i in 0..len {
                        let elem = *src_data.add(len - 1 - i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_DECIMAL);
                p
            }
        }
        V2ElemType::TypedObject => {
            let new_arr =
                TypedArray::<*const TypedObjectStorage>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<*const TypedObjectStorage>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                let len = view.len as usize;
                if len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    for i in 0..len {
                        let elem = *src_data.add(len - 1 - i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_TYPED_OBJECT);
                p
            }
        }
    }
}

/// Concatenate two v2 typed arrays of the same element type. Returns the
/// raw pointer to a freshly-allocated `TypedArray<T>` with `a.len + b.len`
/// elements. Returns `Err` on element-type mismatch (per ADR-006 §2.7.5
/// stamp-at-compile-time: mixed-kind concat is structurally rejected, no
/// coercion).
///
/// Kind-generic over the 14 `V2ElemType` variants. Same retain discipline
/// as `clone_array` for heap-element variants.
pub fn concat_arrays(
    a: &V2TypedArrayView,
    b: &V2TypedArrayView,
) -> Result<*mut u8, &'static str> {
    if a.elem_type != b.elem_type {
        return Err("concat_arrays: element type mismatch");
    }
    let total_len = a
        .len
        .checked_add(b.len)
        .ok_or("concat_arrays: result length overflow")?;

    // Helper: copy two source ranges into a fresh scalar buffer.
    #[inline]
    unsafe fn copy_two_scalar<T: Copy>(
        a_data: *const T,
        a_len: usize,
        b_data: *const T,
        b_len: usize,
        dst_data: *mut T,
    ) {
        if dst_data.is_null() {
            return;
        }
        if a_len > 0 && !a_data.is_null() {
            unsafe { std::ptr::copy_nonoverlapping(a_data, dst_data, a_len) };
        }
        if b_len > 0 && !b_data.is_null() {
            unsafe { std::ptr::copy_nonoverlapping(b_data, dst_data.add(a_len), b_len) };
        }
    }

    let result = match a.elem_type {
        V2ElemType::F64 => unsafe {
            let new_arr = TypedArray::<f64>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<f64>;
            let b_arr = b.ptr as *const TypedArray<f64>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F64);
            p
        },
        V2ElemType::I64 => unsafe {
            let new_arr = TypedArray::<i64>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<i64>;
            let b_arr = b.ptr as *const TypedArray<i64>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I64);
            p
        },
        V2ElemType::I32 => unsafe {
            let new_arr = TypedArray::<i32>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<i32>;
            let b_arr = b.ptr as *const TypedArray<i32>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I32);
            p
        },
        V2ElemType::Bool => unsafe {
            let new_arr = TypedArray::<u8>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<u8>;
            let b_arr = b.ptr as *const TypedArray<u8>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_BOOL);
            p
        },
        V2ElemType::I8 => unsafe {
            let new_arr = TypedArray::<i8>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<i8>;
            let b_arr = b.ptr as *const TypedArray<i8>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I8);
            p
        },
        V2ElemType::U8 => unsafe {
            let new_arr = TypedArray::<u8>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<u8>;
            let b_arr = b.ptr as *const TypedArray<u8>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U8);
            p
        },
        V2ElemType::I16 => unsafe {
            let new_arr = TypedArray::<i16>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<i16>;
            let b_arr = b.ptr as *const TypedArray<i16>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I16);
            p
        },
        V2ElemType::U16 => unsafe {
            let new_arr = TypedArray::<u16>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<u16>;
            let b_arr = b.ptr as *const TypedArray<u16>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U16);
            p
        },
        V2ElemType::U32 => unsafe {
            let new_arr = TypedArray::<u32>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<u32>;
            let b_arr = b.ptr as *const TypedArray<u32>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U32);
            p
        },
        V2ElemType::F32 => unsafe {
            let new_arr = TypedArray::<f32>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<f32>;
            let b_arr = b.ptr as *const TypedArray<f32>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F32);
            p
        },
        V2ElemType::Char => unsafe {
            let new_arr = TypedArray::<char>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<char>;
            let b_arr = b.ptr as *const TypedArray<char>;
            copy_two_scalar(
                (*a_arr).data,
                a.len as usize,
                (*b_arr).data,
                b.len as usize,
                (*new_arr).data,
            );
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_CHAR);
            p
        },
        V2ElemType::String => unsafe {
            let new_arr = TypedArray::<*const StringObj>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<*const StringObj>;
            let b_arr = b.ptr as *const TypedArray<*const StringObj>;
            let dst_data = (*new_arr).data;
            let a_data = (*a_arr).data;
            let b_data = (*b_arr).data;
            if !dst_data.is_null() {
                if a.len > 0 && !a_data.is_null() {
                    for i in 0..(a.len as usize) {
                        let elem = *a_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                if b.len > 0 && !b_data.is_null() {
                    let off = a.len as usize;
                    for i in 0..(b.len as usize) {
                        let elem = *b_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(off + i) = elem;
                    }
                }
            }
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_STRING);
            p
        },
        V2ElemType::Decimal => unsafe {
            let new_arr = TypedArray::<*const DecimalObj>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<*const DecimalObj>;
            let b_arr = b.ptr as *const TypedArray<*const DecimalObj>;
            let dst_data = (*new_arr).data;
            let a_data = (*a_arr).data;
            let b_data = (*b_arr).data;
            if !dst_data.is_null() {
                if a.len > 0 && !a_data.is_null() {
                    for i in 0..(a.len as usize) {
                        let elem = *a_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                if b.len > 0 && !b_data.is_null() {
                    let off = a.len as usize;
                    for i in 0..(b.len as usize) {
                        let elem = *b_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(off + i) = elem;
                    }
                }
            }
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_DECIMAL);
            p
        },
        V2ElemType::TypedObject => unsafe {
            let new_arr =
                TypedArray::<*const TypedObjectStorage>::with_capacity(total_len);
            let a_arr = a.ptr as *const TypedArray<*const TypedObjectStorage>;
            let b_arr = b.ptr as *const TypedArray<*const TypedObjectStorage>;
            let dst_data = (*new_arr).data;
            let a_data = (*a_arr).data;
            let b_data = (*b_arr).data;
            if !dst_data.is_null() {
                if a.len > 0 && !a_data.is_null() {
                    for i in 0..(a.len as usize) {
                        let elem = *a_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(i) = elem;
                    }
                }
                if b.len > 0 && !b_data.is_null() {
                    let off = a.len as usize;
                    for i in 0..(b.len as usize) {
                        let elem = *b_data.add(i);
                        v2_retain(&(*elem).header);
                        *dst_data.add(off + i) = elem;
                    }
                }
            }
            (*new_arr).len = total_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_TYPED_OBJECT);
            p
        },
    };
    Ok(result)
}

/// Allocate a fresh `TypedArray<T>` containing the elements of `view` from
/// `start` (inclusive) to `end` (exclusive), clamped to `[0, view.len]`.
/// Kind-generic. Empty / out-of-order ranges produce an empty result array
/// (mirrors Rust's `slice::get(start..end)` clamping rather than panicking).
///
/// Shared internal worker for `slice_array` / `take_array` / `drop_array_n`.
fn copy_range_to_new_array(
    view: &V2TypedArrayView,
    start: u32,
    end: u32,
) -> *mut u8 {
    // Clamp the range to `[0, view.len]` and compute out_len.
    let start = start.min(view.len);
    let end = end.min(view.len);
    let out_len = end.saturating_sub(start);
    let s = start as usize;
    let n = out_len as usize;

    // Helper: scalar copy of `view.data[start..start+n]` into the fresh
    // buffer.
    #[inline]
    unsafe fn copy_scalar_range<T: Copy>(
        src_data: *const T,
        dst_data: *mut T,
        start: usize,
        len: usize,
    ) {
        if len == 0 || src_data.is_null() || dst_data.is_null() {
            return;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(src_data.add(start), dst_data, len);
        }
    }

    match view.elem_type {
        V2ElemType::F64 => unsafe {
            let new_arr = TypedArray::<f64>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<f64>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F64);
            p
        },
        V2ElemType::I64 => unsafe {
            let new_arr = TypedArray::<i64>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i64>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I64);
            p
        },
        V2ElemType::I32 => unsafe {
            let new_arr = TypedArray::<i32>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i32>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I32);
            p
        },
        V2ElemType::Bool => unsafe {
            let new_arr = TypedArray::<u8>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u8>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_BOOL);
            p
        },
        V2ElemType::I8 => unsafe {
            let new_arr = TypedArray::<i8>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i8>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I8);
            p
        },
        V2ElemType::U8 => unsafe {
            let new_arr = TypedArray::<u8>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u8>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U8);
            p
        },
        V2ElemType::I16 => unsafe {
            let new_arr = TypedArray::<i16>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i16>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I16);
            p
        },
        V2ElemType::U16 => unsafe {
            let new_arr = TypedArray::<u16>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u16>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U16);
            p
        },
        V2ElemType::U32 => unsafe {
            let new_arr = TypedArray::<u32>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u32>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U32);
            p
        },
        V2ElemType::F32 => unsafe {
            let new_arr = TypedArray::<f32>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<f32>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F32);
            p
        },
        V2ElemType::Char => unsafe {
            let new_arr = TypedArray::<char>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<char>;
            copy_scalar_range((*src).data, (*new_arr).data, s, n);
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_CHAR);
            p
        },
        V2ElemType::String => unsafe {
            let new_arr = TypedArray::<*const StringObj>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<*const StringObj>;
            let src_data = (*src).data;
            let dst_data = (*new_arr).data;
            if n > 0 && !src_data.is_null() && !dst_data.is_null() {
                for i in 0..n {
                    let elem = *src_data.add(s + i);
                    v2_retain(&(*elem).header);
                    *dst_data.add(i) = elem;
                }
            }
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_STRING);
            p
        },
        V2ElemType::Decimal => unsafe {
            let new_arr = TypedArray::<*const DecimalObj>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<*const DecimalObj>;
            let src_data = (*src).data;
            let dst_data = (*new_arr).data;
            if n > 0 && !src_data.is_null() && !dst_data.is_null() {
                for i in 0..n {
                    let elem = *src_data.add(s + i);
                    v2_retain(&(*elem).header);
                    *dst_data.add(i) = elem;
                }
            }
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_DECIMAL);
            p
        },
        V2ElemType::TypedObject => unsafe {
            let new_arr =
                TypedArray::<*const TypedObjectStorage>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<*const TypedObjectStorage>;
            let src_data = (*src).data;
            let dst_data = (*new_arr).data;
            if n > 0 && !src_data.is_null() && !dst_data.is_null() {
                for i in 0..n {
                    let elem = *src_data.add(s + i);
                    v2_retain(&(*elem).header);
                    *dst_data.add(i) = elem;
                }
            }
            (*new_arr).len = out_len;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_TYPED_OBJECT);
            p
        },
    }
}

/// `arr.slice(start, end)` — bounded copy of `[start..end)`. `start` and
/// `end` are clamped to `[0, view.len]`; if `start >= end`, the result is
/// empty. Mirrors Rust's `slice::get(range)` clamping behaviour.
#[inline]
pub fn slice_array(view: &V2TypedArrayView, start: u32, end: u32) -> *mut u8 {
    copy_range_to_new_array(view, start, end)
}

/// `arr.take(n)` — first `n` elements. `n` clamped to `[0, view.len]`.
#[inline]
pub fn take_array(view: &V2TypedArrayView, n: u32) -> *mut u8 {
    copy_range_to_new_array(view, 0, n)
}

/// `arr.drop(n)` — all elements except the first `n`. `n` clamped to
/// `[0, view.len]`. Named `drop_array_n` to avoid collision with
/// `TypedArray::drop_array` (the allocator destructor).
#[inline]
pub fn drop_array_n(view: &V2TypedArrayView, n: u32) -> *mut u8 {
    copy_range_to_new_array(view, n, view.len)
}

// ── R8 W4 J.5b HOF-builder primitives (2026-05-24) ──────────────────────────
//
// `where` / `select` / `take_while` / `skip_while` need to build a fresh
// `TypedArray<T>` whose element kind is determined either by the input
// (filter ops: same as input view.elem_type) or by the closure's return
// kind (select). The two helpers below are the kind-mapping +
// empty-allocator pieces; the per-op two-pass scan-then-allocate driver
// lives in `objects/array_query.rs` because it must call the closure via
// `VirtualMachine::call_value_immediate_nb` (§2.7.11 / Q12 ABI), which is
// not reachable from this leaf module.
//
// Supervisor D3 binding (2026-05-24): structured error on closure-return
// kind mismatch; NO coercion (forbidden per CLAUDE.md §Type System Rules);
// NO heterogeneous `Array<Any>` carrier (Shape has no `any` type per
// CLAUDE.md "No `any` type"). First invocation establishes the closure-
// return kind; subsequent mismatch surfaces `VMError::RuntimeError` with a
// structured message naming the expected / got kinds + the offending index.

/// Map a `NativeKind` to its corresponding `V2ElemType` for HOF-builder
/// output-array allocation. Returns `None` for kinds with no monomorphized
/// `TypedArray<T>` carrier (e.g. `Null`, `Ptr(Closure)`, generic
/// `Ptr(HeapKind::HashMap)`). Pairs with `allocate_empty_typed_array` +
/// `push_element` to build a result array element-by-element.
///
/// The mapping mirrors `V2ElemType::elem_kind` in reverse (§2.7.7 / Q9
/// kind ↔ elem-type bijection over the 14 stamped carriers). Anything not
/// in the bijection is unsupported as an output-array element kind —
/// callers (e.g. `select`) surface a structured `RuntimeError` rather than
/// fabricating a Bool-default carrier (forbidden per ADR-006 §2.7.14).
#[inline]
pub fn native_kind_to_v2_elem_type(kind: NativeKind) -> Option<V2ElemType> {
    match kind {
        NativeKind::Float64 => Some(V2ElemType::F64),
        NativeKind::Int64 => Some(V2ElemType::I64),
        NativeKind::Int32 => Some(V2ElemType::I32),
        NativeKind::Bool => Some(V2ElemType::Bool),
        NativeKind::Int8 => Some(V2ElemType::I8),
        NativeKind::UInt8 => Some(V2ElemType::U8),
        NativeKind::Int16 => Some(V2ElemType::I16),
        NativeKind::UInt16 => Some(V2ElemType::U16),
        NativeKind::UInt32 => Some(V2ElemType::U32),
        NativeKind::Float32 => Some(V2ElemType::F32),
        NativeKind::Char => Some(V2ElemType::Char),
        NativeKind::StringV2 => Some(V2ElemType::String),
        NativeKind::DecimalV2 => Some(V2ElemType::Decimal),
        NativeKind::Ptr(HeapKind::TypedObject) => Some(V2ElemType::TypedObject),
        _ => None,
    }
}

/// Allocate an empty `TypedArray<T>` for the given `V2ElemType`,
/// initialised with `capacity` slots, length zero, and stamped with the
/// matching element-type discriminant byte (§2.7.5 producer-side stamp).
/// Returns the raw `*mut u8` carrier pointer for wrapping into a
/// `Ptr(HeapKind::TypedArray)` `KindedSlot`. Subsequent `push_element`
/// calls grow the array.
///
/// Used by the HOF-builder handlers (`where` / `select` / `take_while` /
/// `skip_while`) for output-array allocation after the closure-return
/// kind is established (select) or known statically (filter ops, where
/// the input view's elem_type is the output's elem_type).
pub fn allocate_empty_typed_array(elem_type: V2ElemType, capacity: u32) -> *mut u8 {
    unsafe {
        let p: *mut u8 = match elem_type {
            V2ElemType::F64 => TypedArray::<f64>::with_capacity(capacity) as *mut u8,
            V2ElemType::I64 => TypedArray::<i64>::with_capacity(capacity) as *mut u8,
            V2ElemType::I32 => TypedArray::<i32>::with_capacity(capacity) as *mut u8,
            V2ElemType::Bool => TypedArray::<u8>::with_capacity(capacity) as *mut u8,
            V2ElemType::I8 => TypedArray::<i8>::with_capacity(capacity) as *mut u8,
            V2ElemType::U8 => TypedArray::<u8>::with_capacity(capacity) as *mut u8,
            V2ElemType::I16 => TypedArray::<i16>::with_capacity(capacity) as *mut u8,
            V2ElemType::U16 => TypedArray::<u16>::with_capacity(capacity) as *mut u8,
            V2ElemType::U32 => TypedArray::<u32>::with_capacity(capacity) as *mut u8,
            V2ElemType::F32 => TypedArray::<f32>::with_capacity(capacity) as *mut u8,
            V2ElemType::Char => TypedArray::<char>::with_capacity(capacity) as *mut u8,
            V2ElemType::String => {
                TypedArray::<*const StringObj>::with_capacity(capacity) as *mut u8
            }
            V2ElemType::Decimal => {
                TypedArray::<*const DecimalObj>::with_capacity(capacity) as *mut u8
            }
            V2ElemType::TypedObject => {
                TypedArray::<*const TypedObjectStorage>::with_capacity(capacity) as *mut u8
            }
        };
        let stamp_byte: u8 = match elem_type {
            V2ElemType::F64 => ELEM_TYPE_F64,
            V2ElemType::I64 => ELEM_TYPE_I64,
            V2ElemType::I32 => ELEM_TYPE_I32,
            V2ElemType::Bool => ELEM_TYPE_BOOL,
            V2ElemType::I8 => ELEM_TYPE_I8,
            V2ElemType::U8 => ELEM_TYPE_U8,
            V2ElemType::I16 => ELEM_TYPE_I16,
            V2ElemType::U16 => ELEM_TYPE_U16,
            V2ElemType::U32 => ELEM_TYPE_U32,
            V2ElemType::F32 => ELEM_TYPE_F32,
            V2ElemType::Char => ELEM_TYPE_CHAR,
            V2ElemType::String => ELEM_TYPE_STRING,
            V2ElemType::Decimal => ELEM_TYPE_DECIMAL,
            V2ElemType::TypedObject => ELEM_TYPE_TYPED_OBJECT,
        };
        stamp_elem_type(p, stamp_byte);
        p
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// R8 W4 J.5c (2026-05-24) — deep-equality primitives per supervisor D2.
//
// `eq_element(a, b, elem_type) -> bool` is a generic value-equality body
// with an internal 14-arm dispatch on `V2ElemType` (one arm per element
// kind landed in this module). Matches the existing primitives-layer ABI
// shape (mirrors `write_element` / `push_element`): the host opcode /
// handler passes the (bits, kind) pair of the needle plus the receiver's
// element type; the primitive performs the per-kind compare and returns
// a plain `bool`.
//
// `position_of(view, needle_bits, needle_kind) -> Option<u32>` is the
// per-op driver for `Array.indexOf(value)` (the `Some(i)` arm projects to
// the result; `None` projects to `-1`).
//
// `contains_element(view, needle_bits, needle_kind) -> bool` is the
// per-op driver for `Array.includes(value)`.
//
// Supervisor D2 binding (2026-05-24): generic eq_element with internal
// kind dispatch. REFUSED on sight: MethodFnV2 trait dispatch (forbidden
// MethodFn bridge pattern per CLAUDE.md §Renames-to-refuse). REFUSED on
// sight: dynamic-fallback / Bool-default on unknown / unsupported kinds —
// the kind-mismatch path returns `false` structurally (no element is
// equal to a value of a different carrier shape) without fabricating
// equality on bits, mirroring the strict §2.7.5 producer-stamp + §2.7.14
// no-Bool-default discipline.
//
// Refcount discipline: `eq_element` only READS bits, never retains or
// releases. Per-element reads inside `position_of` / `contains_element`
// do NOT use `read_element` (which would retain the element header on
// every iteration for the heap-element kinds and then immediately drop);
// instead they walk the underlying `TypedArray<T>` buffer directly via
// `get_unchecked` and compare against the needle via `eq_element` — the
// receiver's per-element shares are NOT touched, and the needle's share
// is owned by the caller (the dispatch shell `args[1]`). This keeps the
// includes / indexOf hot path allocation-free for the scalar arms.
// ═══════════════════════════════════════════════════════════════════════════

/// Per-kind value equality for the `(bits, kind)` carrier shape.
///
/// Returns `true` iff `(a_bits, kind)` and `(b_bits, kind)` denote the
/// same value under the kind's semantics. Returns `false` when either
/// pointer is null for a heap-element kind (defensive — the dispatch
/// shell rejects null receivers earlier, but the primitive must stay
/// sound under any input).
///
/// Float semantics: bitwise equality (NOT `==`). This is the
/// IEEE-754-pure choice — `NaN == NaN` is `false` under `==` but `true`
/// under bitwise compare. The choice matches the `includes` / `indexOf`
/// observable: a NaN element pushed into the array IS findable via the
/// same NaN bit pattern (the array doesn't lose track of its own
/// elements). The user can still write a custom predicate via
/// `find(|x| x != x)` when IEEE compare is desired.
///
/// String / Decimal: deref through the v2-raw `StringObj` / `DecimalObj`
/// carrier and compare content (`StringObj::as_str` → `&str` equality;
/// `DecimalObj::value` → `Decimal` `PartialEq`).
///
/// TypedObject: deep field-by-field comparison. The two objects are
/// equal iff (a) they share the same `schema_id`, (b) they share the
/// same per-field `NativeKind` table, and (c) every per-field slot
/// compares equal under the slot's `NativeKind` via a recursive
/// `eq_element` call on the slot bits. Per ADR-006 §2.7.16 typed-Arc
/// dispatch-label receiver-recovery: the comparison reads the storage
/// directly via `&*(p as *const TypedObjectStorage)` (no Box-wrap
/// reinterpret).
///
/// # Safety
/// The caller must uphold the §2.7.5 producer-side stamp invariant —
/// for heap-element kinds (`StringV2` / `DecimalV2` /
/// `Ptr(HeapKind::TypedObject)`), the bits must be either `0` (null) or
/// a live carrier pointer of the matching `T`. The `bits == 0` early-out
/// is the defensive bound; otherwise the deref is sound under the
/// construction-side contract upheld by every producer in this module.
#[inline]
pub fn eq_element(a_bits: u64, b_bits: u64, elem_type: V2ElemType) -> bool {
    match elem_type {
        // Scalar arms: bitwise equality on the slot's significant bits.
        // For width-narrowed scalars (I8 / U8 / I16 / U16 / I32 / U32 /
        // Bool / F32 / Char) the producer zero/sign-extends into the
        // 8-byte slot via `as u64`; comparing the full 8 bytes is sound
        // (the unused high bits agree by construction).
        V2ElemType::F64 => a_bits == b_bits,
        V2ElemType::I64 => a_bits == b_bits,
        V2ElemType::I32 => a_bits == b_bits,
        V2ElemType::Bool => a_bits == b_bits,
        V2ElemType::I8 => a_bits == b_bits,
        V2ElemType::U8 => a_bits == b_bits,
        V2ElemType::I16 => a_bits == b_bits,
        V2ElemType::U16 => a_bits == b_bits,
        V2ElemType::U32 => a_bits == b_bits,
        V2ElemType::F32 => a_bits == b_bits,
        V2ElemType::Char => a_bits == b_bits,
        // Heap-element arms: deref and compare content.
        V2ElemType::String => {
            if a_bits == 0 || b_bits == 0 {
                return a_bits == b_bits;
            }
            let a_ptr = a_bits as usize as *const StringObj;
            let b_ptr = b_bits as usize as *const StringObj;
            if a_ptr == b_ptr {
                return true;
            }
            unsafe { StringObj::as_str(a_ptr) == StringObj::as_str(b_ptr) }
        }
        V2ElemType::Decimal => {
            if a_bits == 0 || b_bits == 0 {
                return a_bits == b_bits;
            }
            let a_ptr = a_bits as usize as *const DecimalObj;
            let b_ptr = b_bits as usize as *const DecimalObj;
            if a_ptr == b_ptr {
                return true;
            }
            unsafe { DecimalObj::value(a_ptr) == DecimalObj::value(b_ptr) }
        }
        V2ElemType::TypedObject => {
            if a_bits == 0 || b_bits == 0 {
                return a_bits == b_bits;
            }
            let a_ptr = a_bits as usize as *const TypedObjectStorage;
            let b_ptr = b_bits as usize as *const TypedObjectStorage;
            if a_ptr == b_ptr {
                return true;
            }
            // SAFETY: per the construction-side contract for the
            // `*const TypedObjectStorage` v2-raw carrier (Wave 2 Agent D1,
            // ADR-006 §2.3 typed-Arc dispatch-label receiver-recovery):
            // a non-null `*const TypedObjectStorage` slot bits value is
            // a live carrier (HeapHeader at offset 0). The borrow is
            // bounded to this scope; no share is retained or released.
            unsafe { typed_object_deep_eq(&*a_ptr, &*b_ptr) }
        }
    }
}

/// Deep equality for two `TypedObjectStorage` instances.
///
/// Returns `true` iff (a) `schema_id` matches, (b) `slots.len()` matches,
/// (c) `field_kinds` table matches (this is the production-time
/// per-schema invariant — same schema_id → identical field_kinds slice),
/// and (d) every slot compares equal via `eq_element` dispatching on the
/// per-field `NativeKind`.
///
/// Per ADR-005 §1 single-discriminator + ADR-006 §2.7.6 / Q8 carrier-
/// API-bound: the per-field `NativeKind` is the discriminator; the
/// recursion maps `NativeKind` back to `V2ElemType` for the per-field
/// `eq_element` call (the field families covered are the ones the v2-raw
/// `TypedArray<T>` element-storage layer supports — scalar primitives +
/// String / Decimal / nested TypedObject). Other `NativeKind` variants
/// (e.g. `Ptr(HeapKind::HashMap)` field) surface-and-stop with `false` —
/// the §2.7.14 no-Bool-default discipline is preserved by being the
/// strict-equal answer, not a fabricated truth value.
///
/// # Safety
/// `a` / `b` must be live `&TypedObjectStorage` borrows bounded to the
/// caller's scope.
unsafe fn typed_object_deep_eq(
    a: &TypedObjectStorage,
    b: &TypedObjectStorage,
) -> bool {
    if a.schema_id != b.schema_id {
        return false;
    }
    if a.slots.len() != b.slots.len() {
        return false;
    }
    // field_kinds is `Arc<[NativeKind]>` shared per-schema; equal
    // schema_ids guarantee equal field_kinds in production. The
    // length-then-elementwise compare below is defensive.
    if a.field_kinds.len() != b.field_kinds.len() {
        return false;
    }
    for (k1, k2) in a.field_kinds.iter().zip(b.field_kinds.iter()) {
        if k1 != k2 {
            return false;
        }
    }
    for i in 0..a.slots.len() {
        let bits_a = a.slots[i].raw();
        let bits_b = b.slots[i].raw();
        let kind = a.field_kinds[i];
        // Map the per-field NativeKind back to the V2ElemType the
        // primitive dispatches on. Fields whose kind lies outside the
        // supported families return `false` (the strict, no-Bool-default
        // answer per §2.7.14): a TypedObject with a HashMap-valued field
        // compares unequal under deep-equality at the structural layer
        // until that field's comparison primitive lands; a future
        // amendment can extend this map without changing the call shape.
        let field_elem = match kind {
            NativeKind::Float64 => Some(V2ElemType::F64),
            NativeKind::Int64 => Some(V2ElemType::I64),
            NativeKind::Int32 => Some(V2ElemType::I32),
            NativeKind::Int16 => Some(V2ElemType::I16),
            NativeKind::Int8 => Some(V2ElemType::I8),
            NativeKind::UInt8 => Some(V2ElemType::U8),
            NativeKind::UInt16 => Some(V2ElemType::U16),
            NativeKind::UInt32 => Some(V2ElemType::U32),
            NativeKind::Float32 => Some(V2ElemType::F32),
            NativeKind::Char => Some(V2ElemType::Char),
            NativeKind::Bool => Some(V2ElemType::Bool),
            NativeKind::StringV2 => Some(V2ElemType::String),
            NativeKind::DecimalV2 => Some(V2ElemType::Decimal),
            NativeKind::Ptr(HeapKind::TypedObject) => Some(V2ElemType::TypedObject),
            // Null-as-sentinel field: equal iff both slots agree on the
            // null tag. The §2.7.5 stamp guarantees the per-slot
            // discriminator already filtered non-null bits before reach.
            NativeKind::Null => {
                if bits_a != bits_b {
                    return false;
                }
                continue;
            }
            // NativeKind::String (Arc<String> carrier) — deref via Arc
            // raw pointer and compare content. NOT covered by V2ElemType
            // (that variant is the StringV2 v2-raw carrier); handled
            // inline here.
            NativeKind::String => {
                if bits_a == bits_b {
                    continue;
                }
                if bits_a == 0 || bits_b == 0 {
                    return false;
                }
                let s_a = unsafe { &*(bits_a as usize as *const String) };
                let s_b = unsafe { &*(bits_b as usize as *const String) };
                if s_a != s_b {
                    return false;
                }
                continue;
            }
            // Other heap-kinded fields (HashMap / Deque / TraitObject /
            // Channel / TypedArray / etc.): strict no-Bool-default —
            // compare equal only when the raw pointer bits agree
            // (identity equality). The structurally-typed deep-equality
            // for these field shapes is out of J.5c scope; extending
            // this match without a Bool-default fallback is the correct
            // forward path when a future driver needs them.
            _ => {
                if bits_a != bits_b {
                    return false;
                }
                continue;
            }
        };
        if let Some(et) = field_elem {
            if !eq_element(bits_a, bits_b, et) {
                return false;
            }
        }
    }
    true
}

/// Return the first index `i` where `view[i] == needle` under the
/// element-type's equality, or `None` if no element matches.
///
/// The needle's bits must be the value of kind matching `view.elem_type`
/// — the dispatch shell at `handle_index_of_v2` enforces the
/// kind-precondition (returns `-1` on mismatch without invoking this
/// primitive, mirroring the JS semantics that `[1,2,3].indexOf("1")` is
/// `-1` not an error).
///
/// Reads the underlying `TypedArray<T>` buffer directly via
/// `get_unchecked` (no `read_element` indirection) — no per-iteration
/// retain/release on heap-element kinds.
///
/// # Safety
/// Caller must guarantee `view` is a live `V2TypedArrayView` (produced
/// by `as_v2_typed_array`) and `needle_bits` is a value of `view.elem_type`
/// per the §2.7.5 producer-stamp contract.
#[inline]
pub fn position_of(view: &V2TypedArrayView, needle_bits: u64) -> Option<u32> {
    let n = view.len;
    if n == 0 {
        return None;
    }
    // Per-element pointer read path for heap-element kinds avoids the
    // per-iteration `v2_retain` work that `read_element` does — we only
    // need the bits, not a fresh share.
    macro_rules! scan_scalar {
        ($t:ty, $to_bits:expr) => {{
            let arr = view.ptr as *const TypedArray<$t>;
            for i in 0..n {
                let v = unsafe { TypedArray::<$t>::get_unchecked(arr, i) };
                if $to_bits(v) == needle_bits {
                    return Some(i);
                }
            }
            None
        }};
    }
    match view.elem_type {
        V2ElemType::F64 => scan_scalar!(f64, |v: f64| v.to_bits()),
        V2ElemType::I64 => scan_scalar!(i64, |v: i64| v as u64),
        V2ElemType::I32 => scan_scalar!(i32, |v: i32| (v as i64) as u64),
        V2ElemType::Bool => scan_scalar!(u8, |v: u8| (v != 0) as u64),
        V2ElemType::I8 => scan_scalar!(i8, |v: i8| (v as i64) as u64),
        V2ElemType::U8 => scan_scalar!(u8, |v: u8| v as u64),
        V2ElemType::I16 => scan_scalar!(i16, |v: i16| (v as i64) as u64),
        V2ElemType::U16 => scan_scalar!(u16, |v: u16| v as u64),
        V2ElemType::U32 => scan_scalar!(u32, |v: u32| v as u64),
        V2ElemType::F32 => scan_scalar!(f32, |v: f32| v.to_bits() as u64),
        V2ElemType::Char => scan_scalar!(char, |v: char| v as u32 as u64),
        V2ElemType::String => unsafe {
            let arr = view.ptr as *const TypedArray<*const StringObj>;
            for i in 0..n {
                let elem_ptr = TypedArray::<*const StringObj>::get_unchecked(arr, i);
                if eq_element(elem_ptr as u64, needle_bits, V2ElemType::String) {
                    return Some(i);
                }
            }
            None
        },
        V2ElemType::Decimal => unsafe {
            let arr = view.ptr as *const TypedArray<*const DecimalObj>;
            for i in 0..n {
                let elem_ptr = TypedArray::<*const DecimalObj>::get_unchecked(arr, i);
                if eq_element(elem_ptr as u64, needle_bits, V2ElemType::Decimal) {
                    return Some(i);
                }
            }
            None
        },
        V2ElemType::TypedObject => unsafe {
            let arr = view.ptr as *const TypedArray<*const TypedObjectStorage>;
            for i in 0..n {
                let elem_ptr = TypedArray::<*const TypedObjectStorage>::get_unchecked(arr, i);
                if eq_element(elem_ptr as u64, needle_bits, V2ElemType::TypedObject) {
                    return Some(i);
                }
            }
            None
        }
    }
}

// ── R8 W4 J.5f sort/orderBy/thenBy primitives (2026-05-24) ──────────────────
//
// Kind-generic permutation + natural-ordering compare primitives backing the
// `array_sort.rs` sort / orderBy / thenBy handlers (supervisor D4 v0.3 scope:
// basic sort + orderBy + thenBy; relational joins + groupBy → v0.4).
//
// `permute_array` copies elements at the given indices from `view` into a
// fresh `TypedArray<T>` of the SAME element kind — output elem_type =
// input elem_type. Heap-element kinds (`String`/`Decimal`/`TypedObject`)
// bump the per-elem refcount once per stored slot (matches the
// `copy_range_to_new_array` heap-arm contract; the J.5b `allocate_empty +
// per-element push_element` pattern works too but allocates one share per
// push, which over-counts when the same index appears twice in an
// `indices` slice; the explicit retain-per-stored-slot here is the
// canonical refcount discipline for the permute case).
//
// `cmp_element_natural` compares two `(bits, kind)` pairs read from the
// SAME view (so the discriminator on both sides is the view's elem_type)
// per the supervisor D4 v0.3 natural-ordering matrix:
//   - scalar (F64/F32/I64/I32/I16/I8/U32/U16/U8): direct `<`/`==`
//   - Bool: false < true
//   - Char: codepoint order
//   - String: lexicographic compare
//   - Decimal: rust_decimal::Decimal::cmp
//   - TypedObject: SURFACE — natural ordering on heap aggregates needs an
//     ADR-006-amendment-level decision (`Ord` trait + per-field projection).
//     Refused per supervisor D3 + ADR-006 §2.7.14 (no fabricated default).
//
// NaN handling: `f64::total_cmp` / `f32::total_cmp` — NaN is the largest
// element (Rust precedent). No silent NaN-skip / NaN-equals fabrication.

/// Compare two elements of the SAME view by natural ordering.
///
/// Returns `Some(Ordering)` on success; `None` if the element kind has no
/// canonical natural ordering at v0.3 (`TypedObject` SURFACE per supervisor
/// D3 + ADR-006 §2.7.14 — Bool-default refused). Callers surface a
/// structured `RuntimeError` when `None` is returned.
///
/// `bits_a` and `bits_b` are read via `read_element(view, i)` and refer to
/// the same `view.elem_type` (the SAME-view discipline). For heap-element
/// reads (`StringV2` / `DecimalV2`), the returned `(bits, kind)` carries
/// the raw `*const StringObj/DecimalObj` pointer — the comparison here
/// dereferences but does not retain or release the pointer (the caller's
/// share remains live for the comparator's borrow).
#[inline]
pub fn cmp_element_natural(
    view: &V2TypedArrayView,
    bits_a: u64,
    bits_b: u64,
) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match view.elem_type {
        V2ElemType::F64 => Some(f64::from_bits(bits_a).total_cmp(&f64::from_bits(bits_b))),
        V2ElemType::F32 => Some(
            f32::from_bits(bits_a as u32).total_cmp(&f32::from_bits(bits_b as u32)),
        ),
        V2ElemType::I64 => Some((bits_a as i64).cmp(&(bits_b as i64))),
        V2ElemType::I32 => Some((bits_a as u32 as i32).cmp(&(bits_b as u32 as i32))),
        V2ElemType::I16 => Some((bits_a as u16 as i16).cmp(&(bits_b as u16 as i16))),
        V2ElemType::I8 => Some((bits_a as u8 as i8).cmp(&(bits_b as u8 as i8))),
        V2ElemType::U32 => Some((bits_a as u32).cmp(&(bits_b as u32))),
        V2ElemType::U16 => Some((bits_a as u16).cmp(&(bits_b as u16))),
        V2ElemType::U8 => Some((bits_a as u8).cmp(&(bits_b as u8))),
        V2ElemType::Bool => {
            // false (0) < true (non-zero) — fold any non-zero to true for
            // ordering parity with Rust `bool::cmp`.
            let a = bits_a != 0;
            let b = bits_b != 0;
            Some(a.cmp(&b))
        }
        V2ElemType::Char => {
            // Codepoint order — `read_element` produces a valid codepoint
            // per the V2ElemType::Char arm; cmp on the raw u32 produces the
            // same ordering as `char::cmp`.
            Some((bits_a as u32).cmp(&(bits_b as u32)))
        }
        V2ElemType::String => unsafe {
            let a_ptr = bits_a as usize as *const StringObj;
            let b_ptr = bits_b as usize as *const StringObj;
            if a_ptr.is_null() || b_ptr.is_null() {
                return None;
            }
            let a = StringObj::as_str(a_ptr);
            let b = StringObj::as_str(b_ptr);
            Some(a.cmp(b))
        },
        V2ElemType::Decimal => unsafe {
            let a_ptr = bits_a as usize as *const DecimalObj;
            let b_ptr = bits_b as usize as *const DecimalObj;
            if a_ptr.is_null() || b_ptr.is_null() {
                return None;
            }
            let a = DecimalObj::value(a_ptr);
            let b = DecimalObj::value(b_ptr);
            Some(a.cmp(&b))
        },
        V2ElemType::TypedObject => {
            // SURFACE per supervisor D3 + ADR-006 §2.7.14: natural ordering
            // on heap aggregates (TypedObject) requires an Ord-trait
            // mechanism + per-field projection decision. v0.4 territory.
            // Bool-default refused; return None and let caller surface the
            // structured RuntimeError naming the elem_type.
            None
        }
        _ => {
            // Defensive: any other elem_type without natural ordering.
            // Returns None; caller surfaces structured RuntimeError.
            #[allow(unreachable_patterns)]
            None
        }
    }
}

/// Materialize a permutation of `view` into a fresh `TypedArray<T>` of the
/// same element kind. `indices[i]` selects the source element at position
/// `i` of the output. Indices out of range `[0, view.len)` are skipped
/// (defensive — callers should validate up front; out-of-range indices
/// indicate a sort comparator bug, not a user-input edge).
///
/// Refcount discipline for heap-element kinds (`String` / `Decimal` /
/// `TypedObject`): each stored slot receives one `v2_retain` on the
/// per-element header so the output array independently owns its shares.
/// This matches the `copy_range_to_new_array` heap-arm contract — the
/// caller's input view share is unaffected by this function (read-only).
///
/// Used by `array_sort::handle_sort_v2` / `handle_order_by_v2` /
/// `handle_then_by_v2` to materialize the sorted permutation. Kind-generic
/// across all 14 monomorphized `TypedArray<T>` carriers per supervisor D4
/// v0.3 scope.
pub fn permute_array(view: &V2TypedArrayView, indices: &[u32]) -> *mut u8 {
    let out_len = indices.len() as u32;

    /// Helper: scalar permute — read source[indices[i]] and write to
    /// dst[i]. Out-of-range indices are skipped silently (sort
    /// comparator bugs surface elsewhere); the corresponding output
    /// slot is left at its `with_capacity` initial bit pattern. Output
    /// length is set to the number of successfully written slots so
    /// the output never claims an uninitialized slot is valid.
    ///
    /// Returns the actual write count (== indices.len() in the
    /// well-formed case where every index is in `[0, src_len)`).
    #[inline]
    unsafe fn permute_scalar<T: Copy>(
        src_data: *const T,
        dst_data: *mut T,
        src_len: u32,
        indices: &[u32],
    ) -> u32 {
        if src_data.is_null() || dst_data.is_null() {
            return 0;
        }
        let mut w: u32 = 0;
        for &idx in indices {
            if idx >= src_len {
                continue;
            }
            unsafe {
                let v = *src_data.add(idx as usize);
                *dst_data.add(w as usize) = v;
            }
            w += 1;
        }
        w
    }

    match view.elem_type {
        V2ElemType::F64 => unsafe {
            let new_arr = TypedArray::<f64>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<f64>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F64);
            p
        },
        V2ElemType::I64 => unsafe {
            let new_arr = TypedArray::<i64>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i64>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I64);
            p
        },
        V2ElemType::I32 => unsafe {
            let new_arr = TypedArray::<i32>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i32>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I32);
            p
        },
        V2ElemType::Bool => unsafe {
            let new_arr = TypedArray::<u8>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u8>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_BOOL);
            p
        },
        V2ElemType::I8 => unsafe {
            let new_arr = TypedArray::<i8>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i8>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I8);
            p
        },
        V2ElemType::U8 => unsafe {
            let new_arr = TypedArray::<u8>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u8>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U8);
            p
        },
        V2ElemType::I16 => unsafe {
            let new_arr = TypedArray::<i16>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<i16>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_I16);
            p
        },
        V2ElemType::U16 => unsafe {
            let new_arr = TypedArray::<u16>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u16>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U16);
            p
        },
        V2ElemType::U32 => unsafe {
            let new_arr = TypedArray::<u32>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<u32>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_U32);
            p
        },
        V2ElemType::F32 => unsafe {
            let new_arr = TypedArray::<f32>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<f32>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_F32);
            p
        },
        V2ElemType::Char => unsafe {
            let new_arr = TypedArray::<char>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<char>;
            let written = permute_scalar((*src).data, (*new_arr).data, view.len, indices);
            (*new_arr).len = written;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_CHAR);
            p
        },
        V2ElemType::String => unsafe {
            let new_arr = TypedArray::<*const StringObj>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<*const StringObj>;
            let src_data = (*src).data;
            let dst_data = (*new_arr).data;
            let mut w: u32 = 0;
            if !src_data.is_null() && !dst_data.is_null() {
                for &idx in indices {
                    if idx >= view.len {
                        continue;
                    }
                    let elem = *src_data.add(idx as usize);
                    v2_retain(&(*elem).header);
                    *dst_data.add(w as usize) = elem;
                    w += 1;
                }
            }
            (*new_arr).len = w;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_STRING);
            p
        },
        V2ElemType::Decimal => unsafe {
            let new_arr = TypedArray::<*const DecimalObj>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<*const DecimalObj>;
            let src_data = (*src).data;
            let dst_data = (*new_arr).data;
            let mut w: u32 = 0;
            if !src_data.is_null() && !dst_data.is_null() {
                for &idx in indices {
                    if idx >= view.len {
                        continue;
                    }
                    let elem = *src_data.add(idx as usize);
                    v2_retain(&(*elem).header);
                    *dst_data.add(w as usize) = elem;
                    w += 1;
                }
            }
            (*new_arr).len = w;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_DECIMAL);
            p
        },
        V2ElemType::TypedObject => unsafe {
            let new_arr =
                TypedArray::<*const TypedObjectStorage>::with_capacity(out_len);
            let src = view.ptr as *const TypedArray<*const TypedObjectStorage>;
            let src_data = (*src).data;
            let dst_data = (*new_arr).data;
            let mut w: u32 = 0;
            if !src_data.is_null() && !dst_data.is_null() {
                for &idx in indices {
                    if idx >= view.len {
                        continue;
                    }
                    let elem = *src_data.add(idx as usize);
                    v2_retain(&(*elem).header);
                    *dst_data.add(w as usize) = elem;
                    w += 1;
                }
            }
            (*new_arr).len = w;
            let p = new_arr as *mut u8;
            stamp_elem_type(p, ELEM_TYPE_TYPED_OBJECT);
            p
        },
    }
}

/// Return `true` iff any element of `view` equals `needle_bits` under
/// the element-type's equality.
///
/// Thin wrapper over `position_of` — present as a named primitive so the
/// per-op driver naming in `array_query.rs` (`handle_includes_v2`)
/// matches the surface comment (J.5 territory: per-kind value-equality
/// `v2_array_detect::contains_element` primitive).
#[inline]
pub fn contains_element(view: &V2TypedArrayView, needle_bits: u64) -> bool {
    position_of(view, needle_bits).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the kinded `(bits, kind)` pair for a v2 typed array pointer
    /// (the shape `v2_handlers/array.rs` push: raw ptr bits +
    /// `Ptr(HeapKind::TypedArray)` per r5c-2-β-CKPT-C).
    #[inline]
    fn ptr_pair(ptr: *mut u8) -> (u64, NativeKind) {
        (ptr as usize as u64, NativeKind::Ptr(HeapKind::TypedArray))
    }

    #[test]
    fn test_stamp_and_read_elem_type_f64() {
        let arr = TypedArray::<f64>::with_capacity(0);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
            let byte = read_elem_type_byte(arr as *const u8);
            assert_eq!(byte, ELEM_TYPE_F64);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_v2_typed_array_recognizes_stamped_f64() {
        let arr = TypedArray::<f64>::with_capacity(4);
        unsafe {
            TypedArray::push(arr, 1.5);
            TypedArray::push(arr, 2.5);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).expect("should recognize v2 typed array");
        assert_eq!(view.elem_type, V2ElemType::F64);
        assert_eq!(view.len, 2);
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_read_element_i64_indices() {
        let arr = TypedArray::<i64>::from_slice(&[10, 20, 30]);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        assert_eq!(read_element(&view, 0), Some((10u64, NativeKind::Int64)));
        assert_eq!(read_element(&view, 1), Some((20u64, NativeKind::Int64)));
        assert_eq!(read_element(&view, 2), Some((30u64, NativeKind::Int64)));
        assert!(read_element(&view, 3).is_none());
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_clone_array_i64() {
        let arr = TypedArray::<i64>::from_slice(&[100, 200, 300]);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        let cloned_ptr = clone_array(&view);
        let (cb, ck) = ptr_pair(cloned_ptr);
        let cloned_view = as_v2_typed_array(cb, ck).expect("clone should be detectable");
        assert_eq!(cloned_view.elem_type, V2ElemType::I64);
        assert_eq!(cloned_view.len, 3);
        assert_eq!(read_element(&cloned_view, 0), Some((100u64, NativeKind::Int64)));
        unsafe {
            TypedArray::<i64>::drop_array(cloned_ptr as *mut TypedArray<i64>);
            TypedArray::drop_array(arr);
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Wave 2 Agent A1 (2026-05-14) — F32 + Char round-trip smokes.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_stamp_and_read_elem_type_f32_char() {
        let arr_f32 = TypedArray::<f32>::with_capacity(0);
        let arr_char = TypedArray::<char>::with_capacity(0);
        unsafe {
            stamp_elem_type(arr_f32 as *mut u8, ELEM_TYPE_F32);
            stamp_elem_type(arr_char as *mut u8, ELEM_TYPE_CHAR);
            assert_eq!(read_elem_type_byte(arr_f32 as *const u8), ELEM_TYPE_F32);
            assert_eq!(read_elem_type_byte(arr_char as *const u8), ELEM_TYPE_CHAR);
            TypedArray::drop_array(arr_f32);
            TypedArray::drop_array(arr_char);
        }
    }

    #[test]
    fn test_as_v2_typed_array_recognizes_stamped_f32() {
        let arr = TypedArray::<f32>::with_capacity(4);
        unsafe {
            TypedArray::push(arr, 1.5_f32);
            TypedArray::push(arr, 2.5_f32);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F32);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).expect("should recognize v2 typed array");
        assert_eq!(view.elem_type, V2ElemType::F32);
        assert_eq!(view.len, 2);
        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_as_v2_typed_array_recognizes_stamped_char() {
        let arr = TypedArray::<char>::with_capacity(4);
        unsafe {
            TypedArray::push(arr, 'A');
            TypedArray::push(arr, '☃');
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_CHAR);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).expect("should recognize v2 typed array");
        assert_eq!(view.elem_type, V2ElemType::Char);
        assert_eq!(view.len, 2);
        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_read_element_f32() {
        let arr = TypedArray::<f32>::from_slice(&[1.5_f32, 2.25_f32, 3.0_f32]);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F32); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        let r0 = read_element(&view, 0).unwrap();
        let r1 = read_element(&view, 1).unwrap();
        let r2 = read_element(&view, 2).unwrap();
        assert_eq!(r0.1, NativeKind::Float32);
        assert_eq!(f32::from_bits(r0.0 as u32), 1.5_f32);
        assert_eq!(f32::from_bits(r1.0 as u32), 2.25_f32);
        assert_eq!(f32::from_bits(r2.0 as u32), 3.0_f32);
        assert!(read_element(&view, 3).is_none());
        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_read_element_char() {
        let arr = TypedArray::<char>::from_slice(&['h', 'i', '!']);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_CHAR); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        for (i, expected) in ['h', 'i', '!'].iter().enumerate() {
            let (b, k) = read_element(&view, i as u32).unwrap();
            assert_eq!(k, NativeKind::Char);
            assert_eq!(char::from_u32(b as u32).unwrap(), *expected);
        }
        assert!(read_element(&view, 3).is_none());
        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_push_element_f32() {
        let arr = TypedArray::<f32>::with_capacity(4);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F32); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        push_element(&view, (1.5_f32).to_bits() as u64, NativeKind::Float32).unwrap();
        // Refresh view to see the new len.
        let view = as_v2_typed_array(bits, kind).unwrap();
        let (b, k) = read_element(&view, 0).unwrap();
        assert_eq!(k, NativeKind::Float32);
        assert_eq!(f32::from_bits(b as u32), 1.5_f32);
        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_push_element_char() {
        let arr = TypedArray::<char>::with_capacity(4);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_CHAR); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        push_element(&view, 'Z' as u32 as u64, NativeKind::Char).unwrap();
        let view = as_v2_typed_array(bits, kind).unwrap();
        let (b, _) = read_element(&view, 0).unwrap();
        assert_eq!(char::from_u32(b as u32).unwrap(), 'Z');
        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_clone_array_f32() {
        let arr = TypedArray::<f32>::from_slice(&[1.0_f32, 2.0_f32, 3.0_f32]);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F32); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        let cloned = clone_array(&view);
        let (cb, ck) = ptr_pair(cloned);
        let cv = as_v2_typed_array(cb, ck).unwrap();
        assert_eq!(cv.elem_type, V2ElemType::F32);
        assert_eq!(cv.len, 3);
        unsafe {
            TypedArray::<f32>::drop_array(cloned as *mut TypedArray<f32>);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_clone_array_char() {
        let arr = TypedArray::<char>::from_slice(&['a', 'b', 'c']);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_CHAR); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        let cloned = clone_array(&view);
        let (cb, ck) = ptr_pair(cloned);
        let cv = as_v2_typed_array(cb, ck).unwrap();
        assert_eq!(cv.elem_type, V2ElemType::Char);
        assert_eq!(cv.len, 3);
        unsafe {
            TypedArray::<char>::drop_array(cloned as *mut TypedArray<char>);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_non_pointer_value_returns_none() {
        // Wrong kind: integer literal, not a pointer.
        assert!(as_v2_typed_array(42u64, NativeKind::Int64).is_none());

        // Wrong kind: float bits.
        assert!(as_v2_typed_array(3.14_f64.to_bits(), NativeKind::Float64).is_none());

        // Wrong kind: bool.
        assert!(as_v2_typed_array(1u64, NativeKind::Bool).is_none());

        // r5c-2-β-CKPT-C: a genuine scalar `u64` (kind = `NativeKind::UInt64`)
        // is NOT the typed-array carrier. The function rejects it on the kind
        // alone — the bits (here `u64::MAX`) are never dereferenced as a
        // pointer. This is the regression guard for the SIGSEGV in
        // `let x: u64 = 18446744073709551615; print(x)`.
        assert!(as_v2_typed_array(u64::MAX, NativeKind::UInt64).is_none());
        assert!(as_v2_typed_array(1000u64, NativeKind::UInt64).is_none());
        assert!(as_v2_typed_array(0u64, NativeKind::UInt64).is_none());

        // Right kind but null pointer.
        assert!(
            as_v2_typed_array(0u64, NativeKind::Ptr(HeapKind::TypedArray)).is_none()
        );
    }

    /// r5c-2-β-CKPT-C: the kind track itself is the carrier discriminator.
    /// The IDENTICAL pointer bits are recognised as a typed array under the
    /// `Ptr(HeapKind::TypedArray)` carrier kind and REJECTED under the
    /// scalar `UInt64` kind — `as_v2_typed_array` never inspects the value
    /// of the bits to guess whether it is a pointer (no low-address /
    /// is_heap heuristic).
    #[test]
    fn test_kind_track_discriminates_array_carrier_from_scalar() {
        let arr = TypedArray::<i64>::from_slice(&[100, 200]);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
        let bits = arr as usize as u64;

        // Same bits, array carrier kind → detected.
        let view = as_v2_typed_array(bits, NativeKind::Ptr(HeapKind::TypedArray))
            .expect("array carrier kind must detect the typed array");
        assert_eq!(view.elem_type, V2ElemType::I64);
        assert_eq!(view.len, 2);

        // Same bits, scalar `UInt64` kind → rejected on the kind alone.
        assert!(
            as_v2_typed_array(bits, NativeKind::UInt64).is_none(),
            "a scalar-u64 kind must NOT be treated as the array carrier"
        );

        unsafe { TypedArray::<i64>::drop_array(arr) };
    }

    // ──────────────────────────────────────────────────────────────────────
    // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element round-trip
    // smokes. Per audit §3.2 S2-prime + §4.1.B.4 migration recipe:
    // `TypedArray<*const StringObj/DecimalObj>` element-read retains the
    // per-element header before pushing the slot bits with NativeKind::
    // StringV2 / DecimalV2 (Agent B's Round 1 carrier-shape variants).
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_stamp_and_read_elem_type_string_decimal() {
        let arr_string = TypedArray::<*const StringObj>::with_capacity(0);
        let arr_decimal = TypedArray::<*const DecimalObj>::with_capacity(0);
        unsafe {
            stamp_elem_type(arr_string as *mut u8, ELEM_TYPE_STRING);
            stamp_elem_type(arr_decimal as *mut u8, ELEM_TYPE_DECIMAL);
            assert_eq!(read_elem_type_byte(arr_string as *const u8), ELEM_TYPE_STRING);
            assert_eq!(read_elem_type_byte(arr_decimal as *const u8), ELEM_TYPE_DECIMAL);
            TypedArray::<*const StringObj>::drop_array_heap(arr_string);
            TypedArray::<*const DecimalObj>::drop_array_heap(arr_decimal);
        }
    }

    #[test]
    fn test_as_v2_typed_array_recognizes_stamped_string() {
        let arr = TypedArray::<*const StringObj>::with_capacity(4);
        unsafe {
            let s = StringObj::new("hello");
            TypedArray::push(arr, s as *const StringObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).expect("should recognize v2 typed array");
        assert_eq!(view.elem_type, V2ElemType::String);
        assert_eq!(view.len, 1);
        unsafe { TypedArray::<*const StringObj>::drop_array_heap(arr); }
    }

    #[test]
    fn test_as_v2_typed_array_recognizes_stamped_decimal() {
        use rust_decimal::Decimal;
        use rust_decimal::prelude::FromPrimitive;
        let arr = TypedArray::<*const DecimalObj>::with_capacity(4);
        unsafe {
            let d = DecimalObj::new(Decimal::from_f64(3.14).unwrap());
            TypedArray::push(arr, d as *const DecimalObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_DECIMAL);
        }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).expect("should recognize v2 typed array");
        assert_eq!(view.elem_type, V2ElemType::Decimal);
        assert_eq!(view.len, 1);
        unsafe { TypedArray::<*const DecimalObj>::drop_array_heap(arr); }
    }

    #[test]
    fn test_read_element_string_retains_share() {
        use shape_value::v2::refcount::v2_get_refcount;
        unsafe {
            let arr = TypedArray::<*const StringObj>::with_capacity(4);
            let s = StringObj::new("greetings");
            // Initial refcount: 1 (from `StringObj::new`).
            assert_eq!(v2_get_refcount(&(*s).header), 1);
            TypedArray::push(arr, s as *const StringObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            // read_element retains: refcount goes 1 → 2.
            let (read_bits, read_kind) = read_element(&view, 0).unwrap();
            assert_eq!(read_kind, NativeKind::StringV2);
            assert_eq!(read_bits, s as u64);
            assert_eq!(v2_get_refcount(&(*s).header), 2);
            // Release the read share (simulates the StringV2 arm in
            // drop_with_kind dropping the slot).
            <StringObj as HeapElement>::release_elem(s);
            assert_eq!(v2_get_refcount(&(*s).header), 1);
            // drop_array_heap releases the array's share → free.
            TypedArray::<*const StringObj>::drop_array_heap(arr);
        }
    }

    #[test]
    fn test_read_element_decimal_retains_share() {
        use rust_decimal::Decimal;
        use rust_decimal::prelude::FromPrimitive;
        use shape_value::v2::refcount::v2_get_refcount;
        unsafe {
            let arr = TypedArray::<*const DecimalObj>::with_capacity(4);
            let d = DecimalObj::new(Decimal::from_f64(2.5).unwrap());
            assert_eq!(v2_get_refcount(&(*d).header), 1);
            TypedArray::push(arr, d as *const DecimalObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_DECIMAL);
            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            let (read_bits, read_kind) = read_element(&view, 0).unwrap();
            assert_eq!(read_kind, NativeKind::DecimalV2);
            assert_eq!(read_bits, d as u64);
            assert_eq!(v2_get_refcount(&(*d).header), 2);
            <DecimalObj as HeapElement>::release_elem(d);
            assert_eq!(v2_get_refcount(&(*d).header), 1);
            TypedArray::<*const DecimalObj>::drop_array_heap(arr);
        }
    }

    #[test]
    fn test_push_element_string_kind_mismatch_refused() {
        // The architectural surface only accepts NativeKind::StringV2 — the
        // Q25.A SUPERSEDED #3 mixed-migration forbidden pattern. Pushing
        // legacy `NativeKind::String` (Phase-2c `Arc<String>` carrier) at
        // this layer would silently corrupt the buffer (the bits are an Arc,
        // not a *const StringObj). The arm returns Err structurally.
        let arr = TypedArray::<*const StringObj>::with_capacity(4);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING); }
        let (bits, kind) = ptr_pair(arr as *mut u8);
        let view = as_v2_typed_array(bits, kind).unwrap();
        // Pretend we have an Arc<String> bit pattern with the legacy
        // NativeKind::String — this is the cross-tier mismatch.
        let result = push_element(&view, 0xDEAD_BEEF, NativeKind::String);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("StringV2"), "expected error to cite StringV2, got: {}", err);
        unsafe { TypedArray::<*const StringObj>::drop_array_heap(arr); }
    }

    #[test]
    fn test_clone_array_string_retains_each_element() {
        use shape_value::v2::refcount::v2_get_refcount;
        unsafe {
            let arr = TypedArray::<*const StringObj>::with_capacity(2);
            let s1 = StringObj::new("foo");
            let s2 = StringObj::new("bar");
            TypedArray::push(arr, s1 as *const StringObj);
            TypedArray::push(arr, s2 as *const StringObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            // Each element starts at refcount 1.
            assert_eq!(v2_get_refcount(&(*s1).header), 1);
            assert_eq!(v2_get_refcount(&(*s2).header), 1);
            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            let cloned = clone_array(&view);
            // Both originals now have refcount 2 (one share per array).
            assert_eq!(v2_get_refcount(&(*s1).header), 2);
            assert_eq!(v2_get_refcount(&(*s2).header), 2);
            let (cb, ck) = ptr_pair(cloned);
            let cv = as_v2_typed_array(cb, ck).unwrap();
            assert_eq!(cv.elem_type, V2ElemType::String);
            assert_eq!(cv.len, 2);
            // Drop the clone — refcounts drop back to 1.
            TypedArray::<*const StringObj>::drop_array_heap(cloned as *mut TypedArray<*const StringObj>);
            assert_eq!(v2_get_refcount(&(*s1).header), 1);
            assert_eq!(v2_get_refcount(&(*s2).header), 1);
            // Drop the original — frees both StringObj allocations.
            TypedArray::<*const StringObj>::drop_array_heap(arr);
        }
    }

    #[test]
    fn test_pop_element_string_transfers_share() {
        use shape_value::v2::refcount::v2_get_refcount;
        unsafe {
            let arr = TypedArray::<*const StringObj>::with_capacity(2);
            let s = StringObj::new("popme");
            TypedArray::push(arr, s as *const StringObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            assert_eq!(v2_get_refcount(&(*s).header), 1);
            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            // pop transfers the array's share to the caller (no retain).
            let (popped_bits, popped_kind) = pop_element(&view).unwrap();
            assert_eq!(popped_kind, NativeKind::StringV2);
            assert_eq!(popped_bits, s as u64);
            // Refcount unchanged at 1 (the share moved).
            assert_eq!(v2_get_refcount(&(*s).header), 1);
            // Release the caller-side share via HeapElement → free.
            <StringObj as HeapElement>::release_elem(s);
            TypedArray::<*const StringObj>::drop_array_heap(arr);
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // R8 W3 J.5a (2026-05-24) — reverse / concat / slice / take / drop /
    // skip primitive round-trip smokes.
    //
    // Each primitive shares the `clone_array` scaffold (per-V2ElemType
    // allocator + stamp + retain). The tests exercise the most common
    // element kinds (I64, F64) for empirical confirmation that the
    // result array has the expected length, stamped element type, and
    // element values in the expected order; heap-element retain paths
    // (String) are also smoke-tested via `reverse_array` to guard the
    // per-element `v2_retain` discipline.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_reverse_array_i64() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2, 3, 4, 5]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            let new_ptr = reverse_array(&view);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.elem_type, V2ElemType::I64);
            assert_eq!(new_view.len, 5);
            let new_arr = new_ptr as *const TypedArray<i64>;
            let data = (*new_arr).data;
            assert_eq!(*data.add(0), 5);
            assert_eq!(*data.add(1), 4);
            assert_eq!(*data.add(2), 3);
            assert_eq!(*data.add(3), 2);
            assert_eq!(*data.add(4), 1);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_reverse_array_empty() {
        unsafe {
            let arr = TypedArray::<i64>::with_capacity(0);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            let new_ptr = reverse_array(&view);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 0);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_reverse_array_string_retains_each_element() {
        use shape_value::v2::refcount::v2_get_refcount;
        unsafe {
            let s1 = StringObj::new("a");
            let s2 = StringObj::new("b");
            let arr = TypedArray::<*const StringObj>::with_capacity(2);
            TypedArray::push(arr, s1 as *const StringObj);
            TypedArray::push(arr, s2 as *const StringObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            assert_eq!(v2_get_refcount(&(*s1).header), 1);
            assert_eq!(v2_get_refcount(&(*s2).header), 1);

            let (bits, kind) = ptr_pair(arr as *mut u8);
            let view = as_v2_typed_array(bits, kind).unwrap();
            let new_ptr = reverse_array(&view);

            // After reverse, both s1 and s2 have refcount 2 — once owned by
            // the source array, once by the new reversed array.
            assert_eq!(v2_get_refcount(&(*s1).header), 2);
            assert_eq!(v2_get_refcount(&(*s2).header), 2);

            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.elem_type, V2ElemType::String);
            assert_eq!(new_view.len, 2);
            let new_arr = new_ptr as *const TypedArray<*const StringObj>;
            let data = (*new_arr).data;
            assert_eq!(*data.add(0), s2 as *const StringObj);
            assert_eq!(*data.add(1), s1 as *const StringObj);

            TypedArray::<*const StringObj>::drop_array_heap(arr);
            TypedArray::<*const StringObj>::drop_array_heap(
                new_ptr as *mut TypedArray<*const StringObj>,
            );
            // Both StringObj allocations are now freed by `drop_array_heap`.
        }
    }

    #[test]
    fn test_concat_arrays_i64() {
        unsafe {
            let a = TypedArray::<i64>::from_slice(&[1, 2]);
            let b = TypedArray::<i64>::from_slice(&[3, 4, 5]);
            stamp_elem_type(a as *mut u8, ELEM_TYPE_I64);
            stamp_elem_type(b as *mut u8, ELEM_TYPE_I64);
            let view_a = as_v2_typed_array(a as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let view_b = as_v2_typed_array(b as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let new_ptr = concat_arrays(&view_a, &view_b).unwrap();
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.elem_type, V2ElemType::I64);
            assert_eq!(new_view.len, 5);
            let new_arr = new_ptr as *const TypedArray<i64>;
            let data = (*new_arr).data;
            assert_eq!(*data.add(0), 1);
            assert_eq!(*data.add(1), 2);
            assert_eq!(*data.add(2), 3);
            assert_eq!(*data.add(3), 4);
            assert_eq!(*data.add(4), 5);
            TypedArray::<i64>::drop_array(a);
            TypedArray::<i64>::drop_array(b);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_concat_arrays_kind_mismatch() {
        unsafe {
            let a = TypedArray::<i64>::from_slice(&[1, 2]);
            let b = TypedArray::<f64>::from_slice(&[3.0, 4.0]);
            stamp_elem_type(a as *mut u8, ELEM_TYPE_I64);
            stamp_elem_type(b as *mut u8, ELEM_TYPE_F64);
            let view_a = as_v2_typed_array(a as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let view_b = as_v2_typed_array(b as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            assert!(concat_arrays(&view_a, &view_b).is_err());
            TypedArray::<i64>::drop_array(a);
            TypedArray::<f64>::drop_array(b);
        }
    }

    #[test]
    fn test_slice_array_i64() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[10, 20, 30, 40, 50]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let new_ptr = slice_array(&view, 1, 4);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.elem_type, V2ElemType::I64);
            assert_eq!(new_view.len, 3);
            let new_arr = new_ptr as *const TypedArray<i64>;
            let data = (*new_arr).data;
            assert_eq!(*data.add(0), 20);
            assert_eq!(*data.add(1), 30);
            assert_eq!(*data.add(2), 40);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_slice_array_clamps_oversize_end() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[10, 20, 30]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            // end > len → clamp to len; result is `[30]`.
            let new_ptr = slice_array(&view, 2, 100);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 1);
            let new_arr = new_ptr as *const TypedArray<i64>;
            assert_eq!(*(*new_arr).data.add(0), 30);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_slice_array_inverted_range_empty() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2, 3]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            // start > end → empty result (no panic).
            let new_ptr = slice_array(&view, 5, 2);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 0);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_take_array_i64() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2, 3, 4, 5]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let new_ptr = take_array(&view, 2);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 2);
            let new_arr = new_ptr as *const TypedArray<i64>;
            assert_eq!(*(*new_arr).data.add(0), 1);
            assert_eq!(*(*new_arr).data.add(1), 2);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_take_array_n_exceeds_len() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            // n > len → clamped to len.
            let new_ptr = take_array(&view, 100);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 2);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_drop_array_n_i64() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2, 3, 4, 5]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let new_ptr = drop_array_n(&view, 2);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 3);
            let new_arr = new_ptr as *const TypedArray<i64>;
            assert_eq!(*(*new_arr).data.add(0), 3);
            assert_eq!(*(*new_arr).data.add(1), 4);
            assert_eq!(*(*new_arr).data.add(2), 5);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    #[test]
    fn test_drop_array_n_exceeds_len_yields_empty() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let new_ptr = drop_array_n(&view, 100);
            let new_view =
                as_v2_typed_array(new_ptr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                    .unwrap();
            assert_eq!(new_view.len, 0);
            TypedArray::<i64>::drop_array(arr);
            TypedArray::<i64>::drop_array(new_ptr as *mut TypedArray<i64>);
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // R8 W4 J.5c (2026-05-24) — eq_element + position_of + contains_element
    // value-equality round-trip smokes per supervisor D2.
    //
    // Per-kind dispatch verified empirically for scalar I64 / F64 / Bool +
    // heap-element String / Decimal. TypedObject deep-equality covered via
    // schema_id mismatch (negative) + nil-payload positive (degenerate
    // schema → equal). NaN float bitwise compare verified for the
    // `[NaN, 1.0].indexOf(NaN)` edge case.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_eq_element_scalar_i64() {
        assert!(eq_element(42, 42, V2ElemType::I64));
        assert!(!eq_element(42, 43, V2ElemType::I64));
        // Negative numbers (sign-extension into the u64 slot).
        assert!(eq_element((-5i64) as u64, (-5i64) as u64, V2ElemType::I64));
        assert!(!eq_element((-5i64) as u64, (5i64) as u64, V2ElemType::I64));
    }

    #[test]
    fn test_eq_element_scalar_f64_bitwise() {
        assert!(eq_element(1.5f64.to_bits(), 1.5f64.to_bits(), V2ElemType::F64));
        assert!(!eq_element(1.5f64.to_bits(), 2.5f64.to_bits(), V2ElemType::F64));
        // IEEE bitwise: NaN bits == NaN bits is TRUE under eq_element
        // (this matches the includes/indexOf observable that an array
        // containing NaN can find its own NaN element).
        let nan = f64::NAN.to_bits();
        assert!(eq_element(nan, nan, V2ElemType::F64));
    }

    #[test]
    fn test_eq_element_scalar_bool() {
        assert!(eq_element(1, 1, V2ElemType::Bool));
        assert!(eq_element(0, 0, V2ElemType::Bool));
        assert!(!eq_element(1, 0, V2ElemType::Bool));
    }

    #[test]
    fn test_eq_element_string_content() {
        unsafe {
            let s1 = StringObj::new("hello");
            let s2 = StringObj::new("hello"); // distinct alloc, same content
            let s3 = StringObj::new("world");
            assert!(eq_element(s1 as u64, s2 as u64, V2ElemType::String));
            assert!(!eq_element(s1 as u64, s3 as u64, V2ElemType::String));
            // null-defensive
            assert!(eq_element(0, 0, V2ElemType::String));
            assert!(!eq_element(s1 as u64, 0, V2ElemType::String));
            // identity short-circuit
            assert!(eq_element(s1 as u64, s1 as u64, V2ElemType::String));
            StringObj::drop(s1);
            StringObj::drop(s2);
            StringObj::drop(s3);
        }
    }

    #[test]
    fn test_eq_element_decimal_content() {
        use rust_decimal::Decimal;
        use rust_decimal::prelude::FromPrimitive;
        unsafe {
            let d1 = DecimalObj::new(Decimal::from_f64(3.14).unwrap());
            let d2 = DecimalObj::new(Decimal::from_f64(3.14).unwrap());
            let d3 = DecimalObj::new(Decimal::from_f64(2.71).unwrap());
            assert!(eq_element(d1 as u64, d2 as u64, V2ElemType::Decimal));
            assert!(!eq_element(d1 as u64, d3 as u64, V2ElemType::Decimal));
            DecimalObj::drop(d1);
            DecimalObj::drop(d2);
            DecimalObj::drop(d3);
        }
    }

    #[test]
    fn test_position_of_i64() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[10, 20, 30, 20, 40]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            assert_eq!(position_of(&view, 10u64), Some(0));
            assert_eq!(position_of(&view, 20u64), Some(1)); // first match
            assert_eq!(position_of(&view, 30u64), Some(2));
            assert_eq!(position_of(&view, 99u64), None);
            TypedArray::<i64>::drop_array(arr);
        }
    }

    #[test]
    fn test_position_of_empty_returns_none() {
        unsafe {
            let arr = TypedArray::<i64>::with_capacity(0);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            assert_eq!(position_of(&view, 0u64), None);
            TypedArray::<i64>::drop_array(arr);
        }
    }

    #[test]
    fn test_contains_element_i64() {
        unsafe {
            let arr = TypedArray::<i64>::from_slice(&[1, 2, 3, 4, 5]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            assert!(contains_element(&view, 3u64));
            assert!(!contains_element(&view, 99u64));
            TypedArray::<i64>::drop_array(arr);
        }
    }

    #[test]
    fn test_position_of_f64_nan_bitwise() {
        unsafe {
            let arr = TypedArray::<f64>::from_slice(&[1.0, f64::NAN, 2.0]);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            // NaN findable by its own bit pattern.
            assert_eq!(position_of(&view, f64::NAN.to_bits()), Some(1));
            assert_eq!(position_of(&view, (1.0f64).to_bits()), Some(0));
            assert_eq!(position_of(&view, (99.0f64).to_bits()), None);
            TypedArray::<f64>::drop_array(arr);
        }
    }

    #[test]
    fn test_position_of_string_content() {
        unsafe {
            let s1 = StringObj::new("a");
            let s2 = StringObj::new("b");
            let s3 = StringObj::new("c");
            let arr = TypedArray::<*const StringObj>::with_capacity(3);
            TypedArray::push(arr, s1 as *const StringObj);
            TypedArray::push(arr, s2 as *const StringObj);
            TypedArray::push(arr, s3 as *const StringObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            // Needle is a SEPARATELY-allocated StringObj with the same
            // content as s2 — content-equality must find it (not pointer
            // identity).
            let needle = StringObj::new("b");
            assert_eq!(position_of(&view, needle as u64), Some(1));
            // Non-matching content.
            let other = StringObj::new("zzz");
            assert_eq!(position_of(&view, other as u64), None);
            assert!(contains_element(&view, needle as u64));
            assert!(!contains_element(&view, other as u64));
            TypedArray::<*const StringObj>::drop_array_heap(arr);
            StringObj::drop(needle);
            StringObj::drop(other);
        }
    }

    #[test]
    fn test_position_of_decimal_content() {
        use rust_decimal::Decimal;
        use rust_decimal::prelude::FromPrimitive;
        unsafe {
            let d1 = DecimalObj::new(Decimal::from_f64(1.5).unwrap());
            let d2 = DecimalObj::new(Decimal::from_f64(2.5).unwrap());
            let arr = TypedArray::<*const DecimalObj>::with_capacity(2);
            TypedArray::push(arr, d1 as *const DecimalObj);
            TypedArray::push(arr, d2 as *const DecimalObj);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_DECIMAL);
            let view = as_v2_typed_array(arr as u64, NativeKind::Ptr(HeapKind::TypedArray))
                .unwrap();
            let needle = DecimalObj::new(Decimal::from_f64(2.5).unwrap());
            assert_eq!(position_of(&view, needle as u64), Some(1));
            let other = DecimalObj::new(Decimal::from_f64(9.9).unwrap());
            assert_eq!(position_of(&view, other as u64), None);
            TypedArray::<*const DecimalObj>::drop_array_heap(arr);
            DecimalObj::drop(needle);
            DecimalObj::drop(other);
        }
    }

    #[test]
    fn test_position_of_typed_object_schema_mismatch() {
        use shape_value::slot::ValueSlot;
        use std::sync::Arc;
        unsafe {
            // Two TypedObjectStorages with the SAME field layout (empty)
            // but different schema_ids — deep-eq must return inequality.
            let kinds_a: Arc<[NativeKind]> = Arc::from(vec![].into_boxed_slice());
            let kinds_b: Arc<[NativeKind]> = Arc::from(vec![].into_boxed_slice());
            let obj_a = TypedObjectStorage::_new(
                1,
                vec![].into_boxed_slice() as Box<[ValueSlot]>,
                0,
                kinds_a,
            );
            let obj_b = TypedObjectStorage::_new(
                2,
                vec![].into_boxed_slice() as Box<[ValueSlot]>,
                0,
                kinds_b,
            );
            assert!(!eq_element(
                obj_a as u64,
                obj_b as u64,
                V2ElemType::TypedObject
            ));
            TypedObjectStorage::_drop(obj_a);
            TypedObjectStorage::_drop(obj_b);
        }
    }

    #[test]
    fn test_position_of_typed_object_same_schema_equal_fields() {
        use shape_value::slot::ValueSlot;
        use std::sync::Arc;
        unsafe {
            // Two TypedObjectStorages with the same schema_id + same single
            // i64 field value (42) — deep-eq must return TRUE.
            let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64].into_boxed_slice());
            let obj_a = TypedObjectStorage::_new(
                7,
                vec![ValueSlot::from_raw(42u64)].into_boxed_slice(),
                0,
                kinds.clone(),
            );
            let obj_b = TypedObjectStorage::_new(
                7,
                vec![ValueSlot::from_raw(42u64)].into_boxed_slice(),
                0,
                kinds.clone(),
            );
            let obj_c = TypedObjectStorage::_new(
                7,
                vec![ValueSlot::from_raw(99u64)].into_boxed_slice(),
                0,
                kinds.clone(),
            );
            assert!(eq_element(
                obj_a as u64,
                obj_b as u64,
                V2ElemType::TypedObject
            ));
            assert!(!eq_element(
                obj_a as u64,
                obj_c as u64,
                V2ElemType::TypedObject
            ));
            // Identity short-circuit.
            assert!(eq_element(
                obj_a as u64,
                obj_a as u64,
                V2ElemType::TypedObject
            ));
            // Null-defensive.
            assert!(!eq_element(0, obj_a as u64, V2ElemType::TypedObject));
            assert!(eq_element(0, 0, V2ElemType::TypedObject));
            TypedObjectStorage::_drop(obj_a);
            TypedObjectStorage::_drop(obj_b);
            TypedObjectStorage::_drop(obj_c);
        }
    }
}
