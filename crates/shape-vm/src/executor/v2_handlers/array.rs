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
use shape_value::v2::decimal_obj::DecimalObj;
use shape_value::v2::heap_element::HeapElement;
use shape_value::v2::refcount::v2_retain;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;
use super::v2_array_detect::{
    ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I16,
    ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_I8, ELEM_TYPE_STRING, ELEM_TYPE_U16, ELEM_TYPE_U32,
    ELEM_TYPE_U8, stamp_elem_type,
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

            // ── Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element monomorphizations ──
            //
            // Per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit §3.2 S2-prime + §4.1.B.4
            // migration recipe: `TypedArray<*const StringObj>` and `TypedArray<*const
            // DecimalObj>` are the v2-raw element-carrier shapes for `Array<string>`
            // / `Array<decimal>`. Element-read retains the per-element header before
            // pushing the slot bits with NativeKind::StringV2 / DecimalV2 (Agent B's
            // Round 1 variants); element-write/push transfers the caller's refcount
            // share; element-set additionally releases the prior element's share.
            //
            // Kind discriminator strict: any value arriving with a non-StringV2 /
            // non-DecimalV2 kind on push/set is a compile-time error surfaced at the
            // VM layer (the dispatch shell that calls these opcodes must have proven
            // the kind via §2.7.5 stamp-at-compile-time; reaching here with a kind
            // mismatch is a §2.7.7 #4 forbidden-pattern instance).

            OpCode::NewTypedArrayString => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<*const StringObj>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_STRING) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetString => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<*const StringObj>;
                let len = unsafe { TypedArray::len(arr) };
                let elem_ptr = unsafe {
                    TypedArray::<*const StringObj>::get(arr, index).ok_or(
                        VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        },
                    )?
                };
                // Retain the per-element header: the array still owns its share;
                // the caller gets a fresh share they must release via the StringV2
                // arm in drop_with_kind (Agent B Round 1 lockstep wiring).
                unsafe { v2_retain(&(*elem_ptr).header) };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(elem_ptr as u64, NativeKind::StringV2)?;
                Ok(())
            }
            OpCode::TypedArrayPushString => {
                let (val_bits, val_kind) = self.pop_kinded()?;
                if val_kind != NativeKind::StringV2 {
                    return Err(VMError::RuntimeError(format!(
                        "TypedArrayPush/SetString: expected NativeKind::StringV2, got {:?}",
                        val_kind
                    )));
                }
                let val = val_bits as usize as *const StringObj;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<*const StringObj>;
                // Caller transfers their refcount share to the array (no retain here).
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetString => {
                let (val_bits, val_kind) = self.pop_kinded()?;
                if val_kind != NativeKind::StringV2 {
                    return Err(VMError::RuntimeError(format!(
                        "TypedArrayPush/SetString: expected NativeKind::StringV2, got {:?}",
                        val_kind
                    )));
                }
                let val = val_bits as usize as *const StringObj;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<*const StringObj>;
                unsafe {
                    let old_ptr = TypedArray::<*const StringObj>::get_unchecked(arr, index);
                    <StringObj as HeapElement>::release_elem(old_ptr);
                    TypedArray::set(arr, index, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            OpCode::NewTypedArrayDecimal => {
                let cap = match instruction.operand {
                    Some(Operand::Count(n)) => n as u32,
                    _ => 0,
                };
                let ptr = TypedArray::<*const DecimalObj>::with_capacity(cap);
                unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_DECIMAL) };
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }
            OpCode::TypedArrayGetDecimal => {
                let (idx_bits, _idx_kind) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *const TypedArray<*const DecimalObj>;
                let len = unsafe { TypedArray::len(arr) };
                let elem_ptr = unsafe {
                    TypedArray::<*const DecimalObj>::get(arr, index).ok_or(
                        VMError::IndexOutOfBounds {
                            index: index as i32,
                            length: len as usize,
                        },
                    )?
                };
                unsafe { v2_retain(&(*elem_ptr).header) };
                drop_with_kind(arr_bits, arr_kind);
                self.push_kinded(elem_ptr as u64, NativeKind::DecimalV2)?;
                Ok(())
            }
            OpCode::TypedArrayPushDecimal => {
                let (val_bits, val_kind) = self.pop_kinded()?;
                if val_kind != NativeKind::DecimalV2 {
                    return Err(VMError::RuntimeError(format!(
                        "TypedArrayPush/SetDecimal: expected NativeKind::DecimalV2, got {:?}",
                        val_kind
                    )));
                }
                let val = val_bits as usize as *const DecimalObj;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<*const DecimalObj>;
                unsafe { TypedArray::push(arr, val); }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }
            OpCode::TypedArraySetDecimal => {
                let (val_bits, val_kind) = self.pop_kinded()?;
                if val_kind != NativeKind::DecimalV2 {
                    return Err(VMError::RuntimeError(format!(
                        "TypedArrayPush/SetDecimal: expected NativeKind::DecimalV2, got {:?}",
                        val_kind
                    )));
                }
                let val = val_bits as usize as *const DecimalObj;
                let (idx_bits, _ik) = self.pop_kinded()?;
                let index = idx_bits as i64 as u32;
                let (arr_bits, arr_kind) = self.pop_kinded()?;
                let arr = arr_bits as usize as *mut TypedArray<*const DecimalObj>;
                unsafe {
                    let old_ptr = TypedArray::<*const DecimalObj>::get_unchecked(arr, index);
                    <DecimalObj as HeapElement>::release_elem(old_ptr);
                    TypedArray::set(arr, index, val);
                }
                drop_with_kind(arr_bits, arr_kind);
                Ok(())
            }

            // ── Wave 3 Stabilize Round 1 V3-A2-followup-producer-cascade (2026-05-15) ──
            //
            // v2-raw heap-element literal constructors. Read the source value
            // from the program constant / string pool, allocate a fresh
            // `StringObj` / `DecimalObj` (refcount = 1), push the raw pointer
            // bits with `NativeKind::StringV2` / `NativeKind::DecimalV2`. The
            // caller's share is then transferred to the typed array on the
            // subsequent `TypedArrayPushString` / `TypedArrayPushDecimal`
            // (matches the per-element refcount discipline of the existing
            // `TypedArrayGet*` arms at lines 663+/733+).
            //
            // Per ADR-006 §2.7.5 stamp-at-compile-time: the kind is proven at
            // compile-time emission; no runtime decode/probe at the FFI boundary.

            OpCode::NewStringV2 => {
                let str_id = match instruction.operand {
                    Some(Operand::Property(id)) => id as usize,
                    Some(Operand::Const(id)) => id as usize,
                    _ => {
                        return Err(VMError::RuntimeError(
                            "NewStringV2 requires a Property/Const string-id operand".to_string(),
                        ));
                    }
                };
                let s = self
                    .program
                    .strings
                    .get(str_id)
                    .ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "NewStringV2: string id {} out of bounds (pool len = {})",
                            str_id,
                            self.program.strings.len()
                        ))
                    })?
                    .clone();
                let ptr = StringObj::new(&s);
                self.push_kinded(ptr as usize as u64, NativeKind::StringV2)?;
                Ok(())
            }

            OpCode::NewDecimalV2 => {
                let const_id = match instruction.operand {
                    Some(Operand::Const(id)) => id as usize,
                    _ => {
                        return Err(VMError::RuntimeError(
                            "NewDecimalV2 requires a Const constant-id operand".to_string(),
                        ));
                    }
                };
                let constant = self.program.constants.get(const_id).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "NewDecimalV2: constant id {} out of bounds (pool len = {})",
                        const_id,
                        self.program.constants.len()
                    ))
                })?;
                let d = match constant {
                    crate::bytecode::Constant::Decimal(d) => *d,
                    other => {
                        return Err(VMError::RuntimeError(format!(
                            "NewDecimalV2: expected Constant::Decimal, got {:?}",
                            other
                        )));
                    }
                };
                let ptr = DecimalObj::new(d);
                self.push_kinded(ptr as usize as u64, NativeKind::DecimalV2)?;
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
