//! Dedicated concatenation opcodes (StringConcat, ArrayConcat).
//!
//! These replace the generic `OpCode::AddDynamic` overload for built-in heap types
//! whose operand types the compiler can prove statically. Operator overloading
//! on user-defined types still goes through `CallMethod` (see Phase 2.5).

use crate::executor::VirtualMachine;
use crate::executor::objects::raw_helpers;
use crate::executor::v2_handlers::v2_array_detect::{
    ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I32, ELEM_TYPE_I64, V2ElemType, as_v2_typed_array,
    stamp_elem_type,
};
use shape_value::heap_value::HeapValue;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

impl VirtualMachine {
    /// Concatenate two heap strings/chars, push the resulting string.
    ///
    /// Stack: `[a, b]` → `[a ++ b]`. Accepts any combination of
    /// `String + String`, `String + Char`, `Char + String`, `Char + Char`.
    /// All other operand combinations are a runtime type error (the compiler
    /// is supposed to only emit this opcode when both operands are statically
    /// proven to be `string` or `char`).
    #[inline]
    pub(in crate::executor) fn op_string_concat(&mut self) -> Result<(), VMError> {
        let b_bits = self.pop_raw_u64()?;
        let a_bits = self.pop_raw_u64()?;
        let a = ValueWord::from_raw_bits(a_bits);
        let b = ValueWord::from_raw_bits(b_bits);

        // Fast path: both strings via raw_helpers
        let result = if let (Some(s_a), Some(s_b)) = (raw_helpers::extract_str(a.raw_bits()), raw_helpers::extract_str(b.raw_bits())) {
            format!("{}{}", s_a, s_b)
        } else {
            // cold-path: as_heap_ref retained — String/Char mixed combinations
            match (a.as_heap_ref(), b.as_heap_ref()) { // cold-path
                (Some(HeapValue::String(s)), Some(HeapValue::Char(c))) => format!("{}{}", s, c),
                (Some(HeapValue::Char(c)), Some(HeapValue::String(s))) => format!("{}{}", c, s),
                (Some(HeapValue::Char(c_a)), Some(HeapValue::Char(c_b))) => format!("{}{}", c_a, c_b),
                _ => {
                    return Err(VMError::TypeError {
                        expected: "string or char operands for StringConcat",
                        got: a.type_name(),
                    });
                }
            }
        };
        self.push_raw_u64(ValueWord::from_string(Arc::new(result)))
    }

    /// Concatenate two arrays, push the resulting array.
    ///
    /// Stack: `[a, b]` → `[a ++ b]`. Handles both:
    ///   * legacy v1 generic `HeapValue::Array(Arc<Vec<ValueWord>>)`
    ///   * v2 monomorphized `TypedArray<T>` (where both operands have the
    ///     same element type)
    ///
    /// Mismatched cases (one v1 + one v2, or two v2 with different element
    /// types) are a runtime type error. The compiler is supposed to only emit
    /// this opcode when both operands are statically proven to be arrays of
    /// compatible element types.
    #[inline]
    pub(in crate::executor) fn op_array_concat(&mut self) -> Result<(), VMError> {
        let b_bits = self.pop_raw_u64()?;
        let a_bits = self.pop_raw_u64()?;
        let a = ValueWord::from_raw_bits(a_bits);
        let b = ValueWord::from_raw_bits(b_bits);

        // ── v2 typed-array fast path ──────────────────────────────────────
        if let (Some(av), Some(bv)) = (as_v2_typed_array(&a), as_v2_typed_array(&b)) {
            if av.elem_type != bv.elem_type {
                return Err(VMError::TypeError {
                    expected: "v2 typed arrays with matching element types",
                    got: "mismatched element types",
                });
            }
            let new_len = av.len + bv.len;
            let (new_ptr, elem_byte) = unsafe {
                match av.elem_type {
                    V2ElemType::F64 => {
                        let new_arr = TypedArray::<f64>::with_capacity(new_len);
                        let ad = (*(av.ptr as *const TypedArray<f64>)).data;
                        let bd = (*(bv.ptr as *const TypedArray<f64>)).data;
                        if av.len > 0 {
                            std::ptr::copy_nonoverlapping(ad, (*new_arr).data, av.len as usize);
                        }
                        if bv.len > 0 {
                            std::ptr::copy_nonoverlapping(
                                bd,
                                (*new_arr).data.add(av.len as usize),
                                bv.len as usize,
                            );
                        }
                        (*new_arr).len = new_len;
                        (new_arr as *mut u8, ELEM_TYPE_F64)
                    }
                    V2ElemType::I64 => {
                        let new_arr = TypedArray::<i64>::with_capacity(new_len);
                        let ad = (*(av.ptr as *const TypedArray<i64>)).data;
                        let bd = (*(bv.ptr as *const TypedArray<i64>)).data;
                        if av.len > 0 {
                            std::ptr::copy_nonoverlapping(ad, (*new_arr).data, av.len as usize);
                        }
                        if bv.len > 0 {
                            std::ptr::copy_nonoverlapping(
                                bd,
                                (*new_arr).data.add(av.len as usize),
                                bv.len as usize,
                            );
                        }
                        (*new_arr).len = new_len;
                        (new_arr as *mut u8, ELEM_TYPE_I64)
                    }
                    V2ElemType::I32 => {
                        let new_arr = TypedArray::<i32>::with_capacity(new_len);
                        let ad = (*(av.ptr as *const TypedArray<i32>)).data;
                        let bd = (*(bv.ptr as *const TypedArray<i32>)).data;
                        if av.len > 0 {
                            std::ptr::copy_nonoverlapping(ad, (*new_arr).data, av.len as usize);
                        }
                        if bv.len > 0 {
                            std::ptr::copy_nonoverlapping(
                                bd,
                                (*new_arr).data.add(av.len as usize),
                                bv.len as usize,
                            );
                        }
                        (*new_arr).len = new_len;
                        (new_arr as *mut u8, ELEM_TYPE_I32)
                    }
                    V2ElemType::Bool => {
                        let new_arr = TypedArray::<u8>::with_capacity(new_len);
                        let ad = (*(av.ptr as *const TypedArray<u8>)).data;
                        let bd = (*(bv.ptr as *const TypedArray<u8>)).data;
                        if av.len > 0 {
                            std::ptr::copy_nonoverlapping(ad, (*new_arr).data, av.len as usize);
                        }
                        if bv.len > 0 {
                            std::ptr::copy_nonoverlapping(
                                bd,
                                (*new_arr).data.add(av.len as usize),
                                bv.len as usize,
                            );
                        }
                        (*new_arr).len = new_len;
                        (new_arr as *mut u8, ELEM_TYPE_BOOL)
                    }
                }
            };
            unsafe { stamp_elem_type(new_ptr, elem_byte) };
            return self.push_raw_u64(ValueWord::from_native_ptr(new_ptr as usize));
        }

        // Mismatched v2 + v1 → error.
        if as_v2_typed_array(&a).is_some() || as_v2_typed_array(&b).is_some() {
            return Err(VMError::TypeError {
                expected: "two arrays of the same kind (v1 or v2)",
                got: "mixed v1/v2 array operands",
            });
        }

        // ── v1 legacy generic Array path ─────────────────────────────────
        if let (Some(view_a), Some(view_b)) =
            (raw_helpers::extract_any_array(a.raw_bits()), raw_helpers::extract_any_array(b.raw_bits()))
        {
            let arr_a = view_a.to_generic();
            let arr_b = view_b.to_generic();
            let mut result = Vec::with_capacity(arr_a.len() + arr_b.len());
            result.extend_from_slice(&arr_a);
            result.extend_from_slice(&arr_b);
            return self.push_raw_u64(ValueWord::from_array(Arc::new(result)));
        }

        Err(VMError::TypeError {
            expected: "two arrays of compatible types",
            got: a.type_name(),
        })
    }
}
