//! VM executor handlers for v2 typed array opcodes.
//!
//! These handlers operate on `TypedArray<T>` pointers stored as
//! `ValueWord::from_native_ptr()` (heap-boxed `NativeScalar::Ptr`).
//! f64 values are stored as NaN-boxed f64. i64 values are NaN-boxed i64.

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::heap_value::NativeScalar;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{VMError, ValueWord};

use super::super::VirtualMachine;

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
                self.push_vw(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            OpCode::NewTypedArrayI64 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i64>::with_capacity(cap);
                self.push_vw(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            OpCode::NewTypedArrayI32 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i32>::with_capacity(cap);
                self.push_vw(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            // ── Element access (get) ────────────────────────────────

            OpCode::TypedArrayGetF64 => {
                let index_vw = self.pop_vw()?;
                let arr_vw = self.pop_vw()?;
                let index = unsafe { index_vw.as_i64_unchecked() } as u32;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<f64>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                self.push_vw(ValueWord::from_f64(val))?;
                Ok(())
            }

            OpCode::TypedArrayGetI64 => {
                let index_vw = self.pop_vw()?;
                let arr_vw = self.pop_vw()?;
                let index = unsafe { index_vw.as_i64_unchecked() } as u32;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<i64>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                self.push_vw(ValueWord::from_i64(val))?;
                Ok(())
            }

            OpCode::TypedArrayGetI32 => {
                let index_vw = self.pop_vw()?;
                let arr_vw = self.pop_vw()?;
                let index = unsafe { index_vw.as_i64_unchecked() } as u32;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<i32>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                // Store i32 as NaN-boxed i64 (sign-extended).
                self.push_vw(ValueWord::from_i64(val as i64))?;
                Ok(())
            }

            // ── Element access (set) ────────────────────────────────

            OpCode::TypedArraySetF64 => {
                let val_vw = self.pop_vw()?;
                let index_vw = self.pop_vw()?;
                let arr_vw = self.pop_vw()?;
                let val = unsafe { val_vw.as_f64_unchecked() };
                let index = unsafe { index_vw.as_i64_unchecked() } as u32;
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<f64>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                Ok(())
            }

            // ── Push ────────────────────────────────────────────────

            OpCode::TypedArrayPushF64 => {
                let val_vw = self.pop_vw()?;
                let arr_vw = self.pop_vw()?;
                let val = unsafe { val_vw.as_f64_unchecked() };
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<f64>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                Ok(())
            }

            OpCode::TypedArrayPushI64 => {
                let val_vw = self.pop_vw()?;
                let arr_vw = self.pop_vw()?;
                let val = unsafe { val_vw.as_i64_unchecked() };
                let arr = extract_ptr(&arr_vw) as *mut TypedArray<i64>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                Ok(())
            }

            // ── Length ───────────────────────────────────────────────

            OpCode::TypedArrayLen => {
                let arr_vw = self.pop_vw()?;
                let arr = extract_ptr(&arr_vw) as *const TypedArray<u8>;
                // len field is at a fixed offset regardless of T — safe to read via any T.
                let len = unsafe { TypedArray::len(arr) };
                self.push_vw(ValueWord::from_i64(len as i64))?;
                Ok(())
            }

            _ => Err(VMError::NotImplemented(format!(
                "v2 typed array opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
