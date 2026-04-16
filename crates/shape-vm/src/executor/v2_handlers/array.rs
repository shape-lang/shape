//! VM executor handlers for v2 typed array opcodes.
//!
//! These handlers operate on `TypedArray<T>` pointers stored as
//! `ValueWord::from_native_ptr()` (heap-boxed `NativeScalar::Ptr`).
//! f64 values are stored as NaN-boxed f64. i64 values are NaN-boxed i64.

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::heap_value::NativeScalar;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{VMError, ValueWord, ValueWordExt};

use super::super::VirtualMachine;
use super::v2_array_detect::{
    ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I32, ELEM_TYPE_I64, stamp_elem_type,
};

/// Extract a raw pointer (usize) from a ValueWord that was created with
/// `ValueWord::from_native_ptr()`. Falls back to `raw_bits()` if the value
/// is not a NativeScalar::Ptr.
#[inline(always)]
fn extract_ptr(vw: &ValueWord) -> usize {
    if let Some(NativeScalar::Ptr(p)) = vw.as_native_scalar() {
        p
    } else {
        // Fallback: treat raw bits as a pointer (for values stored differently).
        vw.raw_bits() as usize
    }
}

impl VirtualMachine {
    /// Execute a v2 typed array opcode.
    pub(crate) fn exec_v2_typed_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            // ── Allocation ──────────────────────────────────────────

            OpCode::NewTypedArrayF64 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<f64>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_F64) };
                self.push_raw_u64(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            OpCode::NewTypedArrayI64 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i64>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I64) };
                self.push_raw_u64(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            OpCode::NewTypedArrayI32 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i32>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I32) };
                self.push_raw_u64(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            OpCode::NewTypedArrayBool => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<u8>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_BOOL) };
                self.push_raw_u64(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            // ── Element access (get) ────────────────────────────────

            OpCode::TypedArrayGetF64 => {
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<f64>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                self.push_raw_f64(val)?;
                Ok(())
            }

            OpCode::TypedArrayGetI64 => {
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<i64>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                self.push_raw_i64(val)?;
                Ok(())
            }

            OpCode::TypedArrayGetI32 => {
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<i32>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                // Store i32 sign-extended as raw i64.
                self.push_raw_i64(val as i64)?;
                Ok(())
            }

            OpCode::TypedArrayGetBool => {
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<u8>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                self.push_raw_bool(val != 0)?;
                Ok(())
            }

            // ── Element access (set) ────────────────────────────────

            OpCode::TypedArraySetF64 => {
                let val = self.pop_raw_f64()?;
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<f64>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                Ok(())
            }

            OpCode::TypedArraySetI64 => {
                let val = self.pop_raw_i64()?;
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<i64>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                Ok(())
            }

            OpCode::TypedArraySetI32 => {
                let val = self.pop_raw_i64()? as i32;
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<i32>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                Ok(())
            }

            OpCode::TypedArraySetBool => {
                let val = self.pop_raw_bool()?;
                let index = self.pop_raw_i64()? as u32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<u8>;
                unsafe {
                    TypedArray::set(arr, index, if val { 1 } else { 0 });
                }
                Ok(())
            }

            // ── Push ────────────────────────────────────────────────

            OpCode::TypedArrayPushF64 => {
                let val = self.pop_raw_f64()?;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<f64>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                Ok(())
            }

            OpCode::TypedArrayPushI64 => {
                let val = self.pop_raw_i64()?;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<i64>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                Ok(())
            }

            OpCode::TypedArrayPushI32 => {
                let val = self.pop_raw_i64()? as i32;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<i32>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                Ok(())
            }

            OpCode::TypedArrayPushBool => {
                let val = self.pop_raw_bool()?;
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<u8>;
                unsafe {
                    TypedArray::push(arr, if val { 1 } else { 0 });
                }
                Ok(())
            }

            // ── Length ───────────────────────────────────────────────

            OpCode::TypedArrayLen => {
                let arr_vw = self.pop_raw_u64()?;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<u8>;
                // len field is at a fixed offset regardless of T — safe to read via any T.
                let len = unsafe { TypedArray::len(arr) };
                self.push_raw_i64(len as i64)?;
                Ok(())
            }

            _ => Err(VMError::NotImplemented(format!(
                "v2 typed array opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
