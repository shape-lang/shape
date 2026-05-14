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
    ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I16, ELEM_TYPE_I32,
    ELEM_TYPE_I64, ELEM_TYPE_I8, ELEM_TYPE_U16, ELEM_TYPE_U32, ELEM_TYPE_U8, stamp_elem_type,
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

            // ── W12 S1 (2026-05-13) — sized-integer monomorphizations ──────
            //
            // Each kind follows the same shape as the F64/I64/I32/Bool
            // arms above, parametrised by the storage type T and the
            // result NativeKind. Sign-extension (I8/I16) and zero-extension
            // (U8/U16/U32) preserve the value's semantic when the i64 slot
            // is later decoded by `decode_i64` per kind.

            OpCode::NewTypedArrayI8 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i8>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I8) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetI8 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<i8>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                // Sign-extend through i64 to preserve negative values.
                self.push_kinded(val as i64 as u64, NativeKind::Int8)?;
                Ok(())
            }
            OpCode::TypedArrayPushI8 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64 as i8;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i8>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetI8 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64 as i8;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i8>;
                unsafe { TypedArray::set(arr, index, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::NewTypedArrayU8 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<u8>::with_capacity(cap);
                // Distinct ELEM_TYPE_U8 (not ELEM_TYPE_BOOL) — the buffer
                // is byte-equivalent but the user-facing kind is U8 vs Bool.
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_U8) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetU8 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<u8>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val as u64, NativeKind::UInt8)?;
                Ok(())
            }
            OpCode::TypedArrayPushU8 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as u8;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u8>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetU8 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as u8;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u8>;
                unsafe { TypedArray::set(arr, index, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::NewTypedArrayI16 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<i16>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I16) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetI16 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<i16>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val as i64 as u64, NativeKind::Int16)?;
                Ok(())
            }
            OpCode::TypedArrayPushI16 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64 as i16;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i16>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetI16 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as i64 as i16;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<i16>;
                unsafe { TypedArray::set(arr, index, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::NewTypedArrayU16 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<u16>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_U16) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetU16 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<u16>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val as u64, NativeKind::UInt16)?;
                Ok(())
            }
            OpCode::TypedArrayPushU16 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as u16;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u16>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetU16 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as u16;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u16>;
                unsafe { TypedArray::set(arr, index, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::NewTypedArrayU32 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<u32>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_U32) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetU32 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<u32>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(val as u64, NativeKind::UInt32)?;
                Ok(())
            }
            OpCode::TypedArrayPushU32 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u32>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetU32 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = val_bits as u32;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<u32>;
                unsafe { TypedArray::set(arr, index, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            // U64 typed-array opcode handlers intentionally NOT minted —
            // see opcode_defs.rs comment block. The S1.5 sub-cluster
            // re-mints OpCode::{New,Get,Push,Set}TypedArrayU64 + their
            // handler bodies once the §2.7.7/Q9 NativeKind discriminator
            // for "pointer to TypedArray<T>" vs "scalar u64" lands.

            // ── Wave 2 Agent A1 (2026-05-14) — F32 + Char monomorphizations ──
            //
            // F32 and Char are `Copy + 4-byte` scalars per R19 S1.5 amendment
            // (W12-nativekind-scalar-additions). Same shape as I8/U16/I32
            // arms above: raw bit transfer through `push_kinded`, with the
            // result `NativeKind::Float32` / `NativeKind::Char` per audit
            // §2.1 row. No new HeapKind, no Arc share, no refcount probe.

            OpCode::NewTypedArrayF32 => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<f32>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_F32) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetF32 => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<f32>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                // Pass f32 bits in the low 32 bits; high bits zero.
                self.push_kinded(val.to_bits() as u64, NativeKind::Float32)?;
                Ok(())
            }
            OpCode::TypedArrayPushF32 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                // F32 bits stored in the low 32 bits of the slot.
                let val = f32::from_bits(val_bits as u32);
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<f32>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetF32 => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = f32::from_bits(val_bits as u32);
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<f32>;
                unsafe { TypedArray::set(arr, index, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::NewTypedArrayChar => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<char>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_CHAR) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetChar => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<char>;
                let len = unsafe { TypedArray::len(arr) };
                let val = unsafe {
                    TypedArray::get(arr, index).ok_or(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len as usize,
                    })?
                };
                drop_with_kind(arr_bits, arr_kind);
                // Char codepoint pushed as inline bits per
                // §2.7.6/Q8 KindedSlot::from_char shape.
                self.push_kinded(val as u32 as u64, NativeKind::Char)?;
                Ok(())
            }
            OpCode::TypedArrayPushChar => {
                let (val_bits, _vk) = self.pop_kinded()?;
                // Codepoint validity check — `from_u32` rejects surrogates
                // and out-of-range values. A runtime corruption here is a
                // VM-internal error; surface-and-stop with a structured
                // error rather than panic on `unwrap`.
                let val = char::from_u32(val_bits as u32).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "TypedArrayPushChar: invalid char codepoint 0x{:X}",
                        val_bits as u32
                    ))
                })?;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<char>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetChar => {
                let (val_bits, _vk) = self.pop_kinded()?;
                let val = char::from_u32(val_bits as u32).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "TypedArraySetChar: invalid char codepoint 0x{:X}",
                        val_bits as u32
                    ))
                })?;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<char>;
                unsafe { TypedArray::set(arr, index, val); }
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
