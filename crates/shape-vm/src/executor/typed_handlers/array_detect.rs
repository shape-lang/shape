#![allow(dead_code)]
//! Runtime detection and uniform access for typed arrays.
//!
//! typed arrays are heap-allocated `TypedArray<T>` instances, where the
//! element type `T` is monomorphized at compile time. The bytecode compiler
//! emits typed allocation/push opcodes (e.g. `NewTypedArrayF64`,
//! `TypedArrayPushF64`) that create the right `TypedArray<T>` instantiation.
//!
//! However, generic consumer-side opcodes (`Length`, `GetProp`, `SetProp`,
//! `IterNext`) and generic method dispatch (`.len()`, `.first()`, `.last()`,
//! `.clone()`, `.sum()`, `.push()`, `.map()`, `.filter()`) only have a runtime
//! `ValueWord` to inspect — they need to recognize the typed array pointer
//! and dispatch to a typed implementation based on the element type.
//!
//! ## Element type encoding
//!
//! The compile-time element type is preserved at runtime by stamping the
//! `_pad` byte (offset 7) of the `HeapHeader` with an `ElemType` discriminant.
//! This piggybacks on existing layout — no struct change required.
//!
//! Allocation handlers in `array.rs` stamp the byte after allocating;
//! consumer paths in this module read the byte to dispatch.

use shape_value::{ValueWord, ValueWordExt};
use shape_value::value_word::*;
use shape_value::heap_value::NativeScalar;
use shape_value::native::heap_header::{HEAP_KIND_TYPED_ARRAY, HeapHeader};
use shape_value::native::typed_array::TypedArray;

// ── Element type discriminants ──────────────────────────────────────────────

pub const ELEM_TYPE_UNKNOWN: u8 = 0;
pub const ELEM_TYPE_F64: u8 = 1;
pub const ELEM_TYPE_I64: u8 = 2;
pub const ELEM_TYPE_I32: u8 = 3;
pub const ELEM_TYPE_BOOL: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeElemType {
    F64,
    I64,
    I32,
    Bool,
}

impl NativeElemType {
    #[inline]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            ELEM_TYPE_F64 => Some(NativeElemType::F64),
            ELEM_TYPE_I64 => Some(NativeElemType::I64),
            ELEM_TYPE_I32 => Some(NativeElemType::I32),
            ELEM_TYPE_BOOL => Some(NativeElemType::Bool),
            _ => None,
        }
    }
}

// ── Detection ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct NativeTypedArrayView {
    pub ptr: *mut u8,
    pub elem_type: NativeElemType,
    pub len: u32,
}

/// Stamp the element type byte (`_pad` at offset 7 of the HeapHeader) on a
/// freshly-allocated typed array.
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

/// Read the element type byte from a typed array's header.
#[inline]
unsafe fn read_elem_type_byte(ptr: *const u8) -> u8 {
    if ptr.is_null() {
        return ELEM_TYPE_UNKNOWN;
    }
    unsafe { *ptr.add(7) }
}

/// Try to interpret a `ValueWord` as a typed array pointer.
#[inline]
pub fn as_native_typed_array(vw: &ValueWord) -> Option<NativeTypedArrayView> {
    let ptr = match vw.as_native_scalar()? {
        NativeScalar::Ptr(p) if p != 0 => p as *mut u8,
        _ => return None,
    };
    let header = unsafe { &*(ptr as *const HeapHeader) };
    if header.kind != HEAP_KIND_TYPED_ARRAY {
        return None;
    }
    let elem_byte = unsafe { read_elem_type_byte(ptr) };
    let elem_type = NativeElemType::from_byte(elem_byte)?;
    let arr_u8 = ptr as *const TypedArray<u8>;
    let len = unsafe { (*arr_u8).len };
    Some(NativeTypedArrayView {
        ptr,
        elem_type,
        len,
    })
}

/// Read element `index` from a typed array as a `ValueWord`.
#[inline]
pub fn read_element(view: &NativeTypedArrayView, index: u32) -> Option<ValueWord> {
    if index >= view.len {
        return None;
    }
    let val = match view.elem_type {
        NativeElemType::F64 => unsafe {
            let arr = view.ptr as *const TypedArray<f64>;
            vw_from_f64(TypedArray::<f64>::get_unchecked(arr, index))
        },
        NativeElemType::I64 => unsafe {
            let arr = view.ptr as *const TypedArray<i64>;
            vw_from_i64(TypedArray::<i64>::get_unchecked(arr, index))
        },
        NativeElemType::I32 => unsafe {
            let arr = view.ptr as *const TypedArray<i32>;
            vw_from_i64(TypedArray::<i32>::get_unchecked(arr, index) as i64)
        },
        NativeElemType::Bool => unsafe {
            let arr = view.ptr as *const TypedArray<u8>;
            vw_from_bool(TypedArray::<u8>::get_unchecked(arr, index) != 0)
        },
    };
    Some(val)
}

/// Write `value` to element `index` of a typed array.
#[inline]
pub fn write_element(
    view: &NativeTypedArrayView,
    index: u32,
    value: &ValueWord,
) -> Result<(), &'static str> {
    if index >= view.len {
        return Err("index out of bounds");
    }
    match view.elem_type {
        NativeElemType::F64 => {
            let v = value
                .as_f64()
                .or_else(|| value.as_i64().map(|i| i as f64))
                .ok_or("expected f64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f64>;
                TypedArray::<f64>::set(arr, index, v);
            }
        }
        NativeElemType::I64 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i64>;
                TypedArray::<i64>::set(arr, index, v);
            }
        }
        NativeElemType::I32 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i32>;
                TypedArray::<i32>::set(arr, index, v as i32);
            }
        }
        NativeElemType::Bool => {
            let v = value.as_bool().ok_or("expected bool value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::set(arr, index, if v { 1 } else { 0 });
            }
        }
    }
    Ok(())
}

/// Append `value` to a typed array.
#[inline]
pub fn push_element(view: &NativeTypedArrayView, value: &ValueWord) -> Result<(), &'static str> {
    match view.elem_type {
        NativeElemType::F64 => {
            let v = value
                .as_f64()
                .or_else(|| value.as_i64().map(|i| i as f64))
                .ok_or("expected f64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f64>;
                TypedArray::<f64>::push(arr, v);
            }
        }
        NativeElemType::I64 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i64>;
                TypedArray::<i64>::push(arr, v);
            }
        }
        NativeElemType::I32 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i32>;
                TypedArray::<i32>::push(arr, v as i32);
            }
        }
        NativeElemType::Bool => {
            let v = value.as_bool().ok_or("expected bool value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::push(arr, if v { 1 } else { 0 });
            }
        }
    }
    Ok(())
}

/// Pop the last element from a typed array.
#[inline]
pub fn pop_element(view: &NativeTypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        NativeElemType::F64 => unsafe {
            let arr = view.ptr as *mut TypedArray<f64>;
            TypedArray::<f64>::pop(arr).map(vw_from_f64)
        },
        NativeElemType::I64 => unsafe {
            let arr = view.ptr as *mut TypedArray<i64>;
            TypedArray::<i64>::pop(arr).map(vw_from_i64)
        },
        NativeElemType::I32 => unsafe {
            let arr = view.ptr as *mut TypedArray<i32>;
            TypedArray::<i32>::pop(arr).map(|v| vw_from_i64(v as i64))
        },
        NativeElemType::Bool => unsafe {
            let arr = view.ptr as *mut TypedArray<u8>;
            TypedArray::<u8>::pop(arr).map(|v| vw_from_bool(v != 0))
        },
    }
}

/// Sum all elements of a numeric (F64/I64/I32) typed array.
///
/// F64 and I64 variants use `wide::f64x4` / `wide::i64x4` SIMD reduction on
/// arrays with >= `SIMD_SUM_THRESHOLD` elements. Smaller arrays fall back to
/// scalar accumulation where SIMD setup overhead would exceed the savings.
pub fn sum_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    /// Minimum element count at which SIMD reduction beats scalar.
    const SIMD_SUM_THRESHOLD: usize = 16;

    match view.elem_type {
        NativeElemType::F64 => {
            let len = view.len as usize;
            if len == 0 {
                return Some(vw_from_f64(0.0));
            }
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<f64>;
                (*arr).data as *const f64
            };
            let s = unsafe { simd_sum_f64(data, len, SIMD_SUM_THRESHOLD) };
            Some(vw_from_f64(s))
        }
        NativeElemType::I64 => {
            let len = view.len as usize;
            if len == 0 {
                return Some(vw_from_i64(0));
            }
            let data = unsafe {
                let arr = view.ptr as *const TypedArray<i64>;
                (*arr).data as *const i64
            };
            let s = unsafe { simd_sum_i64(data, len, SIMD_SUM_THRESHOLD) };
            Some(vw_from_i64(s))
        }
        NativeElemType::I32 => {
            let mut s: i64 = 0;
            for i in 0..view.len {
                let val = unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as i64
                };
                s = s.wrapping_add(val);
            }
            Some(vw_from_i64(s))
        }
        NativeElemType::Bool => None,
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

/// SIMD-accelerated i64 sum using `wide::i64x4` lanes. Wrapping semantics.
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
        // wide::i64x4 lacks AddAssign; use binary + and reassign.
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

/// Compute the average (mean) of all elements of a numeric typed array.
/// Returns NaN for empty arrays.
pub fn avg_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    if view.len == 0 {
        return match view.elem_type {
            NativeElemType::F64 | NativeElemType::I64 | NativeElemType::I32 => {
                Some(vw_from_f64(f64::NAN))
            }
            NativeElemType::Bool => None,
        };
    }
    match view.elem_type {
        NativeElemType::F64 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
            }
            Some(vw_from_f64(s / view.len as f64))
        }
        NativeElemType::I64 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<i64>;
                    TypedArray::<i64>::get_unchecked(arr, i) as f64
                };
            }
            Some(vw_from_f64(s / view.len as f64))
        }
        NativeElemType::I32 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as f64
                };
            }
            Some(vw_from_f64(s / view.len as f64))
        }
        NativeElemType::Bool => None,
    }
}

/// Compute the minimum element of a numeric typed array.
pub fn min_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    if view.len == 0 {
        return match view.elem_type {
            NativeElemType::F64 => Some(vw_from_f64(f64::NAN)),
            NativeElemType::I64 | NativeElemType::I32 => Some(vw_none()),
            NativeElemType::Bool => None,
        };
    }
    match view.elem_type {
        NativeElemType::F64 => {
            let mut min = f64::INFINITY;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
                if v < min {
                    min = v;
                }
            }
            Some(vw_from_f64(min))
        }
        NativeElemType::I64 => {
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
            Some(vw_from_i64(min))
        }
        NativeElemType::I32 => {
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
            Some(vw_from_i64(min))
        }
        NativeElemType::Bool => None,
    }
}

/// Compute the maximum element of a numeric typed array.
pub fn max_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    if view.len == 0 {
        return match view.elem_type {
            NativeElemType::F64 => Some(vw_from_f64(f64::NAN)),
            NativeElemType::I64 | NativeElemType::I32 => Some(vw_none()),
            NativeElemType::Bool => None,
        };
    }
    match view.elem_type {
        NativeElemType::F64 => {
            let mut max = f64::NEG_INFINITY;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
                if v > max {
                    max = v;
                }
            }
            Some(vw_from_f64(max))
        }
        NativeElemType::I64 => {
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
            Some(vw_from_i64(max))
        }
        NativeElemType::I32 => {
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
            Some(vw_from_i64(max))
        }
        NativeElemType::Bool => None,
    }
}

/// Compute the sample variance of a float typed array.
/// Returns NaN for arrays with fewer than 2 elements.
pub fn variance_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        NativeElemType::F64 => {
            if view.len < 2 {
                return Some(vw_from_f64(f64::NAN));
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
            Some(vw_from_f64(var_sum / (n - 1.0)))
        }
        _ => None,
    }
}

/// Compute the sample standard deviation of a float typed array.
pub fn std_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    variance_elements(view).map(|vw| {
        let v = vw.as_f64().unwrap_or(f64::NAN);
        vw_from_f64(v.sqrt())
    })
}

/// Compute the dot product of two float typed arrays.
pub fn dot_elements(
    view_a: &NativeTypedArrayView,
    view_b: &NativeTypedArrayView,
) -> Option<ValueWord> {
    if view_a.elem_type != NativeElemType::F64 || view_b.elem_type != NativeElemType::F64 {
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
    Some(vw_from_f64(sum))
}

/// Compute the Euclidean norm of a float typed array.
pub fn norm_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        NativeElemType::F64 => {
            let mut sum_sq = 0.0_f64;
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
                sum_sq += v * v;
            }
            Some(vw_from_f64(sum_sq.sqrt()))
        }
        _ => None,
    }
}

/// Count `true` values in a bool typed array.
pub fn count_true_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        NativeElemType::Bool => {
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
            Some(vw_from_i64(count))
        }
        _ => None,
    }
}

/// Check if any element in a bool typed array is true.
pub fn any_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        NativeElemType::Bool => {
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v != 0 {
                    return Some(vw_from_bool(true));
                }
            }
            Some(vw_from_bool(false))
        }
        _ => None,
    }
}

/// Check if all elements in a bool typed array are true.
pub fn all_elements(view: &NativeTypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        NativeElemType::Bool => {
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v == 0 {
                    return Some(vw_from_bool(false));
                }
            }
            Some(vw_from_bool(true))
        }
        _ => None,
    }
}

/// Allocate a fresh typed array, copy all elements from `view`, stamp
/// elem_type, and return its raw pointer.
pub fn clone_array(view: &NativeTypedArrayView) -> *mut u8 {
    match view.elem_type {
        NativeElemType::F64 => {
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
        NativeElemType::I64 => {
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
        NativeElemType::I32 => {
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
        NativeElemType::Bool => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_as_native_typed_array_recognizes_stamped_f64() {
        let arr = TypedArray::<f64>::with_capacity(4);
        unsafe {
            TypedArray::push(arr, 1.5);
            TypedArray::push(arr, 2.5);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
        }
        let vw = vw_from_native_ptr(arr as usize);
        let view = as_native_typed_array(&vw).expect("should recognize typed array");
        assert_eq!(view.elem_type, NativeElemType::F64);
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
        let vw = vw_from_native_ptr(arr as usize);
        let view = as_native_typed_array(&vw).unwrap();
        assert_eq!(read_element(&view, 0).unwrap().as_i64(), Some(10));
        assert_eq!(read_element(&view, 1).unwrap().as_i64(), Some(20));
        assert_eq!(read_element(&view, 2).unwrap().as_i64(), Some(30));
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
        let vw = vw_from_native_ptr(arr as usize);
        let view = as_native_typed_array(&vw).unwrap();
        let cloned_ptr = clone_array(&view);
        let cloned_vw = vw_from_native_ptr(cloned_ptr as usize);
        let cloned_view = as_native_typed_array(&cloned_vw).expect("clone should be detectable");
        assert_eq!(cloned_view.elem_type, NativeElemType::I64);
        assert_eq!(cloned_view.len, 3);
        assert_eq!(read_element(&cloned_view, 0).unwrap().as_i64(), Some(100));
        unsafe {
            TypedArray::<i64>::drop_array(cloned_ptr as *mut TypedArray<i64>);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_non_pointer_value_returns_none() {
        let int_vw = vw_from_i64(42);
        assert!(as_native_typed_array(&int_vw).is_none());

        let float_vw = vw_from_f64(3.14);
        assert!(as_native_typed_array(&float_vw).is_none());

        let bool_vw = vw_from_bool(true);
        assert!(as_native_typed_array(&bool_vw).is_none());
    }
}
