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
use shape_value::v2::heap_header::{HEAP_KIND_V2_TYPED_ARRAY, HeapHeader};
use shape_value::v2::typed_array::TypedArray;

// ── Element type discriminants ──────────────────────────────────────────────

pub const ELEM_TYPE_UNKNOWN: u8 = 0;
pub const ELEM_TYPE_F64: u8 = 1;
pub const ELEM_TYPE_I64: u8 = 2;
pub const ELEM_TYPE_I32: u8 = 3;
pub const ELEM_TYPE_BOOL: u8 = 4;
// W12 S1 (2026-05-13) — sized-integer element-type discriminants.
pub const ELEM_TYPE_I8: u8 = 5;
pub const ELEM_TYPE_U8: u8 = 6;
pub const ELEM_TYPE_I16: u8 = 7;
pub const ELEM_TYPE_U16: u8 = 8;
pub const ELEM_TYPE_U32: u8 = 9;
pub const ELEM_TYPE_U64: u8 = 10;

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
    U64,
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
            ELEM_TYPE_U64 => Some(V2ElemType::U64),
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
            V2ElemType::U64 => NativeKind::UInt64,
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
/// v2 typed array pointers flow through the kinded API as raw `*mut TypedArray<T>`
/// values stored in `NativeKind::UInt64` slots — no Arc, no refcount (see
/// `v2_handlers/array.rs` allocation path). Any other `kind` is rejected so a
/// stray heap pointer (e.g. `Ptr(HeapKind::TypedArray)` carrying an
/// `Arc<TypedArrayData>` from the legacy boxed path) is not misinterpreted.
#[inline]
pub fn as_v2_typed_array(bits: u64, kind: NativeKind) -> Option<V2TypedArrayView> {
    if kind != NativeKind::UInt64 {
        return None;
    }
    if bits == 0 {
        return None;
    }
    // W12 S1 (2026-05-13) — low-address-pointer guard. v2 heap allocations
    // come from `std::alloc::alloc(Layout::new::<TypedArray<_>>())`, which
    // on every supported target returns a pointer well above the first
    // memory page (e.g. on Linux x86_64 the minimum mmap address is
    // 0x1_0000 / 64 KiB). Small `NativeKind::UInt64` *scalar* values that
    // happen to flow through the same kinded slot (e.g. an element read
    // from a `TypedArray<u64>` via `TypedArrayGetU64`) would otherwise
    // hit an unmapped-memory deref at `*(bits as *const HeapHeader)`.
    // The guard preserves the documented "kind-only" classification —
    // there's no pointer-bit probing fast path that should ever resolve
    // a small integer as a v2 typed array; the check just keeps unsafe
    // dereferences confined to the heap-pointer regime.
    if bits < 0x1_0000 {
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
        V2ElemType::U64 => unsafe {
            let arr = view.ptr as *const TypedArray<u64>;
            let v = TypedArray::<u64>::get_unchecked(arr, index);
            (v, NativeKind::UInt64)
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
        V2ElemType::U64 => {
            let v = decode_i64(bits, kind).ok_or("expected u64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u64>;
                TypedArray::<u64>::set(arr, index, v as u64);
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
        V2ElemType::U64 => {
            let v = decode_i64(bits, kind).ok_or("expected u64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u64>;
                TypedArray::<u64>::push(arr, v as u64);
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
        V2ElemType::U64 => unsafe {
            let arr = view.ptr as *mut TypedArray<u64>;
            TypedArray::<u64>::pop(arr).map(|v| (v, NativeKind::UInt64))
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
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::U64 => None,
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
            | V2ElemType::U64 => None,
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
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::U64 => None,
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
            | V2ElemType::U64 => None,
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
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::U64 => None,
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
            | V2ElemType::U64 => None,
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
        V2ElemType::Bool
        | V2ElemType::I8
        | V2ElemType::U8
        | V2ElemType::I16
        | V2ElemType::U16
        | V2ElemType::U32
        | V2ElemType::U64 => None,
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
        V2ElemType::U64 => {
            let new_arr = TypedArray::<u64>::with_capacity(view.len);
            unsafe {
                let src = view.ptr as *const TypedArray<u64>;
                let src_data = (*src).data;
                let dst_data = (*new_arr).data;
                if view.len > 0 && !src_data.is_null() && !dst_data.is_null() {
                    std::ptr::copy_nonoverlapping(src_data, dst_data, view.len as usize);
                }
                (*new_arr).len = view.len;
                let p = new_arr as *mut u8;
                stamp_elem_type(p, ELEM_TYPE_U64);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the kinded `(bits, kind)` pair for a v2 typed array pointer
    /// (the shape `v2_handlers/array.rs` push: raw ptr bits + `UInt64`).
    #[inline]
    fn ptr_pair(ptr: *mut u8) -> (u64, NativeKind) {
        (ptr as usize as u64, NativeKind::UInt64)
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

    #[test]
    fn test_non_pointer_value_returns_none() {
        // Wrong kind: integer literal, not a pointer.
        assert!(as_v2_typed_array(42u64, NativeKind::Int64).is_none());

        // Wrong kind: float bits.
        assert!(as_v2_typed_array(3.14_f64.to_bits(), NativeKind::Float64).is_none());

        // Wrong kind: bool.
        assert!(as_v2_typed_array(1u64, NativeKind::Bool).is_none());

        // Right kind but null pointer.
        assert!(as_v2_typed_array(0u64, NativeKind::UInt64).is_none());
    }
}
