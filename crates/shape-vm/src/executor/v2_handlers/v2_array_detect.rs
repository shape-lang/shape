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
//! `ValueWord` to inspect — they need to recognize the v2 typed array pointer
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

use shape_value::ValueWord;
use shape_value::heap_value::NativeScalar;
use shape_value::v2::heap_header::{HEAP_KIND_V2_TYPED_ARRAY, HeapHeader};
use shape_value::v2::typed_array::TypedArray;

// ── Element type discriminants ──────────────────────────────────────────────

pub const ELEM_TYPE_UNKNOWN: u8 = 0;
pub const ELEM_TYPE_F64: u8 = 1;
pub const ELEM_TYPE_I64: u8 = 2;
pub const ELEM_TYPE_I32: u8 = 3;
pub const ELEM_TYPE_BOOL: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2ElemType {
    F64,
    I64,
    I32,
    Bool,
}

impl V2ElemType {
    #[inline]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            ELEM_TYPE_F64 => Some(V2ElemType::F64),
            ELEM_TYPE_I64 => Some(V2ElemType::I64),
            ELEM_TYPE_I32 => Some(V2ElemType::I32),
            ELEM_TYPE_BOOL => Some(V2ElemType::Bool),
            _ => None,
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

/// Try to interpret a `ValueWord` as a v2 typed array pointer.
#[inline]
pub fn as_v2_typed_array(vw: &ValueWord) -> Option<V2TypedArrayView> {
    let ptr = match vw.as_native_scalar()? {
        NativeScalar::Ptr(p) if p != 0 => p as *mut u8,
        _ => return None,
    };
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

/// Read element `index` from a v2 typed array as a `ValueWord`.
#[inline]
pub fn read_element(view: &V2TypedArrayView, index: u32) -> Option<ValueWord> {
    if index >= view.len {
        return None;
    }
    let val = match view.elem_type {
        V2ElemType::F64 => unsafe {
            let arr = view.ptr as *const TypedArray<f64>;
            ValueWord::from_f64(TypedArray::<f64>::get_unchecked(arr, index))
        },
        V2ElemType::I64 => unsafe {
            let arr = view.ptr as *const TypedArray<i64>;
            ValueWord::from_i64(TypedArray::<i64>::get_unchecked(arr, index))
        },
        V2ElemType::I32 => unsafe {
            let arr = view.ptr as *const TypedArray<i32>;
            ValueWord::from_i64(TypedArray::<i32>::get_unchecked(arr, index) as i64)
        },
        V2ElemType::Bool => unsafe {
            let arr = view.ptr as *const TypedArray<u8>;
            ValueWord::from_bool(TypedArray::<u8>::get_unchecked(arr, index) != 0)
        },
    };
    Some(val)
}

/// Write `value` to element `index` of a v2 typed array.
#[inline]
pub fn write_element(
    view: &V2TypedArrayView,
    index: u32,
    value: &ValueWord,
) -> Result<(), &'static str> {
    if index >= view.len {
        return Err("index out of bounds");
    }
    match view.elem_type {
        V2ElemType::F64 => {
            let v = value
                .as_f64()
                .or_else(|| value.as_i64().map(|i| i as f64))
                .ok_or("expected f64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f64>;
                TypedArray::<f64>::set(arr, index, v);
            }
        }
        V2ElemType::I64 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i64>;
                TypedArray::<i64>::set(arr, index, v);
            }
        }
        V2ElemType::I32 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i32>;
                TypedArray::<i32>::set(arr, index, v as i32);
            }
        }
        V2ElemType::Bool => {
            let v = value.as_bool().ok_or("expected bool value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::set(arr, index, if v { 1 } else { 0 });
            }
        }
    }
    Ok(())
}

/// Append `value` to a v2 typed array.
#[inline]
pub fn push_element(view: &V2TypedArrayView, value: &ValueWord) -> Result<(), &'static str> {
    match view.elem_type {
        V2ElemType::F64 => {
            let v = value
                .as_f64()
                .or_else(|| value.as_i64().map(|i| i as f64))
                .ok_or("expected f64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<f64>;
                TypedArray::<f64>::push(arr, v);
            }
        }
        V2ElemType::I64 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i64-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i64>;
                TypedArray::<i64>::push(arr, v);
            }
        }
        V2ElemType::I32 => {
            let v = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .ok_or("expected i32-compatible value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<i32>;
                TypedArray::<i32>::push(arr, v as i32);
            }
        }
        V2ElemType::Bool => {
            let v = value.as_bool().ok_or("expected bool value")?;
            unsafe {
                let arr = view.ptr as *mut TypedArray<u8>;
                TypedArray::<u8>::push(arr, if v { 1 } else { 0 });
            }
        }
    }
    Ok(())
}

/// Pop the last element from a v2 typed array.
#[inline]
pub fn pop_element(view: &V2TypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        V2ElemType::F64 => unsafe {
            let arr = view.ptr as *mut TypedArray<f64>;
            TypedArray::<f64>::pop(arr).map(ValueWord::from_f64)
        },
        V2ElemType::I64 => unsafe {
            let arr = view.ptr as *mut TypedArray<i64>;
            TypedArray::<i64>::pop(arr).map(ValueWord::from_i64)
        },
        V2ElemType::I32 => unsafe {
            let arr = view.ptr as *mut TypedArray<i32>;
            TypedArray::<i32>::pop(arr).map(|v| ValueWord::from_i64(v as i64))
        },
        V2ElemType::Bool => unsafe {
            let arr = view.ptr as *mut TypedArray<u8>;
            TypedArray::<u8>::pop(arr).map(|v| ValueWord::from_bool(v != 0))
        },
    }
}

/// Sum all elements of a numeric (F64/I64/I32) v2 typed array.
pub fn sum_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        V2ElemType::F64 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                let val = unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
                s += val;
            }
            Some(ValueWord::from_f64(s))
        }
        V2ElemType::I64 => {
            let mut s: i64 = 0;
            for i in 0..view.len {
                let val = unsafe {
                    let arr = view.ptr as *const TypedArray<i64>;
                    TypedArray::<i64>::get_unchecked(arr, i)
                };
                s = s.wrapping_add(val);
            }
            Some(ValueWord::from_i64(s))
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
            Some(ValueWord::from_i64(s))
        }
        V2ElemType::Bool => None,
    }
}

/// Compute the average (mean) of all elements of a numeric v2 typed array.
/// Returns NaN for empty arrays.
pub fn avg_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    if view.len == 0 {
        return match view.elem_type {
            V2ElemType::F64 | V2ElemType::I64 | V2ElemType::I32 => {
                Some(ValueWord::from_f64(f64::NAN))
            }
            V2ElemType::Bool => None,
        };
    }
    match view.elem_type {
        V2ElemType::F64 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<f64>;
                    TypedArray::<f64>::get_unchecked(arr, i)
                };
            }
            Some(ValueWord::from_f64(s / view.len as f64))
        }
        V2ElemType::I64 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<i64>;
                    TypedArray::<i64>::get_unchecked(arr, i) as f64
                };
            }
            Some(ValueWord::from_f64(s / view.len as f64))
        }
        V2ElemType::I32 => {
            let mut s = 0.0_f64;
            for i in 0..view.len {
                s += unsafe {
                    let arr = view.ptr as *const TypedArray<i32>;
                    TypedArray::<i32>::get_unchecked(arr, i) as f64
                };
            }
            Some(ValueWord::from_f64(s / view.len as f64))
        }
        V2ElemType::Bool => None,
    }
}

/// Compute the minimum element of a numeric v2 typed array.
pub fn min_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    if view.len == 0 {
        return match view.elem_type {
            V2ElemType::F64 => Some(ValueWord::from_f64(f64::NAN)),
            V2ElemType::I64 | V2ElemType::I32 => Some(ValueWord::none()),
            V2ElemType::Bool => None,
        };
    }
    match view.elem_type {
        V2ElemType::F64 => {
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
            Some(ValueWord::from_f64(min))
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
            Some(ValueWord::from_i64(min))
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
            Some(ValueWord::from_i64(min))
        }
        V2ElemType::Bool => None,
    }
}

/// Compute the maximum element of a numeric v2 typed array.
pub fn max_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    if view.len == 0 {
        return match view.elem_type {
            V2ElemType::F64 => Some(ValueWord::from_f64(f64::NAN)),
            V2ElemType::I64 | V2ElemType::I32 => Some(ValueWord::none()),
            V2ElemType::Bool => None,
        };
    }
    match view.elem_type {
        V2ElemType::F64 => {
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
            Some(ValueWord::from_f64(max))
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
            Some(ValueWord::from_i64(max))
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
            Some(ValueWord::from_i64(max))
        }
        V2ElemType::Bool => None,
    }
}

/// Compute the sample variance of a float v2 typed array.
/// Returns NaN for arrays with fewer than 2 elements.
pub fn variance_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        V2ElemType::F64 => {
            if view.len < 2 {
                return Some(ValueWord::from_f64(f64::NAN));
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
            Some(ValueWord::from_f64(var_sum / (n - 1.0)))
        }
        _ => None,
    }
}

/// Compute the sample standard deviation of a float v2 typed array.
pub fn std_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    variance_elements(view).map(|vw| {
        let v = vw.as_f64().unwrap_or(f64::NAN);
        ValueWord::from_f64(v.sqrt())
    })
}

/// Compute the dot product of two float v2 typed arrays.
pub fn dot_elements(
    view_a: &V2TypedArrayView,
    view_b: &V2TypedArrayView,
) -> Option<ValueWord> {
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
    Some(ValueWord::from_f64(sum))
}

/// Compute the Euclidean norm of a float v2 typed array.
pub fn norm_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
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
            Some(ValueWord::from_f64(sum_sq.sqrt()))
        }
        _ => None,
    }
}

/// Count `true` values in a bool v2 typed array.
pub fn count_true_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
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
            Some(ValueWord::from_i64(count))
        }
        _ => None,
    }
}

/// Check if any element in a bool v2 typed array is true.
pub fn any_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        V2ElemType::Bool => {
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v != 0 {
                    return Some(ValueWord::from_bool(true));
                }
            }
            Some(ValueWord::from_bool(false))
        }
        _ => None,
    }
}

/// Check if all elements in a bool v2 typed array are true.
pub fn all_elements(view: &V2TypedArrayView) -> Option<ValueWord> {
    match view.elem_type {
        V2ElemType::Bool => {
            for i in 0..view.len {
                let v = unsafe {
                    let arr = view.ptr as *const TypedArray<u8>;
                    TypedArray::<u8>::get_unchecked(arr, i)
                };
                if v == 0 {
                    return Some(ValueWord::from_bool(false));
                }
            }
            Some(ValueWord::from_bool(true))
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
    fn test_as_v2_typed_array_recognizes_stamped_f64() {
        let arr = TypedArray::<f64>::with_capacity(4);
        unsafe {
            TypedArray::push(arr, 1.5);
            TypedArray::push(arr, 2.5);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
        }
        let vw = ValueWord::from_native_ptr(arr as usize);
        let view = as_v2_typed_array(&vw).expect("should recognize v2 typed array");
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
        let vw = ValueWord::from_native_ptr(arr as usize);
        let view = as_v2_typed_array(&vw).unwrap();
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
        let vw = ValueWord::from_native_ptr(arr as usize);
        let view = as_v2_typed_array(&vw).unwrap();
        let cloned_ptr = clone_array(&view);
        let cloned_vw = ValueWord::from_native_ptr(cloned_ptr as usize);
        let cloned_view = as_v2_typed_array(&cloned_vw).expect("clone should be detectable");
        assert_eq!(cloned_view.elem_type, V2ElemType::I64);
        assert_eq!(cloned_view.len, 3);
        assert_eq!(read_element(&cloned_view, 0).unwrap().as_i64(), Some(100));
        unsafe {
            TypedArray::<i64>::drop_array(cloned_ptr as *mut TypedArray<i64>);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_non_pointer_value_returns_none() {
        let int_vw = ValueWord::from_i64(42);
        assert!(as_v2_typed_array(&int_vw).is_none());

        let float_vw = ValueWord::from_f64(3.14);
        assert!(as_v2_typed_array(&float_vw).is_none());

        let bool_vw = ValueWord::from_bool(true);
        assert!(as_v2_typed_array(&bool_vw).is_none());
    }
}
