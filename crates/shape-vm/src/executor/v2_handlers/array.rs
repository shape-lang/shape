//! VM executor handlers for v2 typed array opcodes.
//!
//! These handlers operate on `TypedArray<T>` raw pointers (`*mut TypedArray<T>`),
//! NativeScalar-shaped (non-Arc, custom heap allocation). Pointer bits flow
//! through the kinded API as `NativeKind::UInt64` (no refcount). Element
//! kinds:
//!   F64  -> `NativeKind::Float64`
//!   I64  -> `NativeKind::Int64`
//!   I32  -> `NativeKind::Int32`
//!   Bool -> `NativeKind::Bool`
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C.

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;
use super::v2_array_detect::{
    ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I32, ELEM_TYPE_I64, stamp_elem_type,
};

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
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }

            OpCode::NewTypedArrayI64 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i64>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I64) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }

            OpCode::NewTypedArrayI32 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i32>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I32) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }

            OpCode::NewTypedArrayBool => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<u8>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_BOOL) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }

            // ── Element access (get) ────────────────────────────────

            OpCode::TypedArrayGetF64 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<f64>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val.to_bits(), NativeKind::Float64)?;
                Ok(())
            }

            OpCode::TypedArrayGetI64 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<i64>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val as u64, NativeKind::Int64)?;
                Ok(())
            }

            OpCode::TypedArrayGetI32 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<i32>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val as i64 as u64, NativeKind::Int32)?;
                Ok(())
            }

            OpCode::TypedArrayGetBool => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<u8>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index)
                        .ok_or(VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded((val != 0) as u64, NativeKind::Bool)?;
                Ok(())
            }

            // ── Element access (set) ────────────────────────────────

            OpCode::TypedArraySetF64 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = f64::from_bits(val_bits);
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<f64>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::TypedArraySetI64 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i64>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::TypedArraySetI32 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64 as i32;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i32>;
                unsafe {
                    TypedArray::set(arr, index, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::TypedArraySetBool => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits != 0;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u8>;
                unsafe {
                    TypedArray::set(arr, index, if val { 1 } else { 0 });
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            // ── Push ────────────────────────────────────────────────

            OpCode::TypedArrayPushF64 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = f64::from_bits(val_bits);
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<f64>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::TypedArrayPushI64 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i64>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::TypedArrayPushI32 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64 as i32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i32>;
                unsafe {
                    TypedArray::push(arr, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::TypedArrayPushBool => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits != 0;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u8>;
                unsafe {
                    TypedArray::push(arr, if val { 1 } else { 0 });
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            // ── Length ───────────────────────────────────────────────

            OpCode::TypedArrayLen => {
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<u8>;
                // len field is at a fixed offset regardless of T — safe to read via any T.
                let len = unsafe { TypedArray::len(arr) };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(len as u64, NativeKind::Int64)?;
                Ok(())
            }

            _ => Err(VMError::NotImplemented(format!(
                "v2 typed array opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
