//! Typed array element access opcodes (local-slot based).
//!
//! These opcodes skip the HeapValue enum dispatch when the compiler proves the
//! element type. The array lives in a local slot (Operand::Local) in one of two
//! representations, distinguished by the parallel kind track:
//!
//! 1. Legacy heap-boxed `HeapValue::TypedArray(TypedArrayData::I64|F64|...)`
//!    — kind = `NativeKind::Ptr(HeapKind::TypedArray)`. Bits =
//!    `Arc::into_raw(Arc<TypedArrayData>)`.
//! 2. Raw v2 pointer `*mut TypedArray<T>` — kind = `NativeKind::UInt64`
//!    (NativeScalar shape). Bits = raw pointer (no Arc).
//!
//! Both representations are supported transparently — the compiler emits these
//! opcodes whenever a local is tracked in `v2_typed_array_locals`, and that
//! map covers both paths.
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C: kinded API. Dispatch on the kind
//! recorded in the parallel kind track for the local slot.
//!
//! ## Opcodes handled here
//!
//! | Opcode        | Stack in         | Stack out | Operand      |
//! |---------------|------------------|-----------|--------------|
//! | GetElemI64    | [index]          | [value]   | Local(slot)  |
//! | GetElemF64    | [index]          | [value]   | Local(slot)  |
//! | SetElemI64    | [index, value]   | []        | Local(slot)  |
//! | SetElemF64    | [index, value]   | []        | Local(slot)  |
//! | ArrayPushI64  | [value]          | []        | Local(slot)  |
//! | ArrayPushF64  | [value]          | []        | Local(slot)  |
//! | ArrayLenTyped | []               | [len]     | Local(slot)  |

use std::sync::Arc;

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::v2::typed_array::TypedArray;
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;

impl VirtualMachine {
    /// Dispatch for the typed array element access opcodes.
    pub(crate) fn exec_typed_array_elem_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::GetElemI64 => self.op_get_elem_i64(instruction),
            OpCode::GetElemF64 => self.op_get_elem_f64(instruction),
            OpCode::SetElemI64 => self.op_set_elem_i64(instruction),
            OpCode::SetElemF64 => self.op_set_elem_f64(instruction),
            OpCode::ArrayPushI64 => self.op_array_push_i64_elem(instruction),
            OpCode::ArrayPushF64 => self.op_array_push_f64_elem(instruction),
            OpCode::ArrayLenTyped => self.op_array_len_typed(instruction),
            _ => unreachable!("exec_typed_array_elem_ops called with {:?}", instruction.opcode),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Resolve the absolute stack slot from the instruction's Local operand.
    #[inline(always)]
    fn resolve_local_slot(
        &self,
        instruction: &Instruction,
    ) -> Result<usize, VMError> {
        match instruction.operand {
            Some(Operand::Local(idx)) => {
                let slot = self.current_locals_base() + idx as usize;
                if slot >= self.stack.len() {
                    return Err(VMError::RuntimeError(format!(
                        "Local slot {} out of bounds (stack size {})",
                        idx,
                        self.stack.len()
                    )));
                }
                Ok(slot)
            }
            _ => Err(VMError::InvalidOperand),
        }
    }

    // -----------------------------------------------------------------------
    // GetElemI64
    // -----------------------------------------------------------------------

    fn op_get_elem_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (idx_bits, _idx_kind) = self.pop_kinded()?;
        let index = idx_bits as i64;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let (arr_bits, arr_kind) = self.stack_read_kinded_raw(slot);
        let val: i64 = match arr_kind {
            NativeKind::UInt64 => {
                // v2 raw-pointer fast path.
                let arr = arr_bits as usize as *const TypedArray<i64>;
                let len = unsafe { TypedArray::len(arr) } as usize;
                if index >= len {
                    return Err(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len,
                    });
                }
                unsafe { TypedArray::get_unchecked(arr, index as u32) }
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                // Legacy heap-boxed Arc<TypedArrayData> path. Read without
                // taking ownership: reconstruct the Arc, project, forget.
                let arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let result = match &*arc {
                    TypedArrayData::I64(buf) => {
                        if index >= buf.data.len() {
                            Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: buf.data.len(),
                            })
                        } else {
                            Ok(buf.data[index])
                        }
                    }
                    other => Err(VMError::TypeError {
                        expected: "Array<int>",
                        got: other.type_name(),
                    }),
                };
                // Restore the Arc share (we read by reference, no ownership transfer).
                let _ = Arc::into_raw(arc);
                result?
            }
            _ => {
                return Err(VMError::TypeError {
                    expected: "Array<int>",
                    got: "non-array slot",
                });
            }
        };

        self.push_kinded(val as u64, NativeKind::Int64)
    }

    // -----------------------------------------------------------------------
    // GetElemF64
    // -----------------------------------------------------------------------

    fn op_get_elem_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (idx_bits, _idx_kind) = self.pop_kinded()?;
        let index = idx_bits as i64;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let (arr_bits, arr_kind) = self.stack_read_kinded_raw(slot);
        let val: f64 = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *const TypedArray<f64>;
                let len = unsafe { TypedArray::len(arr) } as usize;
                if index >= len {
                    return Err(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len,
                    });
                }
                unsafe { TypedArray::get_unchecked(arr, index as u32) }
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let result = match &*arc {
                    TypedArrayData::F64(buf) => {
                        if index >= buf.data.len() {
                            Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: buf.data.len(),
                            })
                        } else {
                            Ok(buf.data[index])
                        }
                    }
                    other => Err(VMError::TypeError {
                        expected: "Array<number>",
                        got: other.type_name(),
                    }),
                };
                let _ = Arc::into_raw(arc);
                result?
            }
            _ => {
                return Err(VMError::TypeError {
                    expected: "Array<number>",
                    got: "non-array slot",
                });
            }
        };

        self.push_kinded(val.to_bits(), NativeKind::Float64)
    }

    // -----------------------------------------------------------------------
    // SetElemI64
    // -----------------------------------------------------------------------

    fn op_set_elem_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (val_bits, _vk) = self.pop_kinded()?;
        let val = val_bits as i64;
        let (idx_bits, _ik) = self.pop_kinded()?;
        let index = idx_bits as i64;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        // Take-mutate-write pattern for the local slot.
        let (arr_bits, arr_kind) = self.stack_take_kinded(slot);
        let result = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *mut TypedArray<i64>;
                let len = unsafe { TypedArray::len(arr) } as usize;
                if index >= len {
                    Err(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len,
                    })
                } else {
                    unsafe { TypedArray::set(arr, index as u32, val) };
                    Ok(())
                }
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                // Reconstruct the Arc and mutate via Arc::make_mut.
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let mutate_result = match Arc::make_mut(&mut arc) {
                    TypedArrayData::I64(buf) => {
                        let buf = Arc::make_mut(buf);
                        if index >= buf.data.len() {
                            Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: buf.data.len(),
                            })
                        } else {
                            buf.data[index] = val;
                            Ok(())
                        }
                    }
                    other => Err(VMError::TypeError {
                        expected: "Array<int>",
                        got: other.type_name(),
                    }),
                };
                // Re-stash the (possibly newly-cloned) Arc.
                let new_bits = Arc::into_raw(arc) as u64;
                self.stack[slot] = new_bits;
                self.kinds[slot] = arr_kind;
                return mutate_result;
            }
            _ => Err(VMError::TypeError {
                expected: "Array<int>",
                got: "non-array slot",
            }),
        };
        // For UInt64 path: restore the bits we took.
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

    // -----------------------------------------------------------------------
    // SetElemF64
    // -----------------------------------------------------------------------

    fn op_set_elem_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (val_bits, _vk) = self.pop_kinded()?;
        let val = f64::from_bits(val_bits);
        let (idx_bits, _ik) = self.pop_kinded()?;
        let index = idx_bits as i64;
        if index < 0 {
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: 0,
            });
        }
        let index = index as usize;

        let (arr_bits, arr_kind) = self.stack_take_kinded(slot);
        let result = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *mut TypedArray<f64>;
                let len = unsafe { TypedArray::len(arr) } as usize;
                if index >= len {
                    Err(VMError::IndexOutOfBounds {
                        index: index as i32,
                        length: len,
                    })
                } else {
                    unsafe { TypedArray::set(arr, index as u32, val) };
                    Ok(())
                }
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let mutate_result = match Arc::make_mut(&mut arc) {
                    TypedArrayData::F64(buf) => {
                        let buf = Arc::make_mut(buf);
                        if index >= buf.data.len() {
                            Err(VMError::IndexOutOfBounds {
                                index: index as i32,
                                length: buf.data.len(),
                            })
                        } else {
                            buf.data[index] = val;
                            Ok(())
                        }
                    }
                    other => Err(VMError::TypeError {
                        expected: "Array<number>",
                        got: other.type_name(),
                    }),
                };
                let new_bits = Arc::into_raw(arc) as u64;
                self.stack[slot] = new_bits;
                self.kinds[slot] = arr_kind;
                return mutate_result;
            }
            _ => Err(VMError::TypeError {
                expected: "Array<number>",
                got: "non-array slot",
            }),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

    // -----------------------------------------------------------------------
    // ArrayPushI64
    // -----------------------------------------------------------------------

    fn op_array_push_i64_elem(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (val_bits, _vk) = self.pop_kinded()?;
        let val = val_bits as i64;

        let (arr_bits, arr_kind) = self.stack_take_kinded(slot);
        let result = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *mut TypedArray<i64>;
                unsafe { TypedArray::push(arr, val) };
                Ok(())
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let r = match Arc::make_mut(&mut arc) {
                    TypedArrayData::I64(buf) => {
                        Arc::make_mut(buf).data.push(val);
                        Ok(())
                    }
                    other => Err(VMError::TypeError {
                        expected: "Array<int>",
                        got: other.type_name(),
                    }),
                };
                let new_bits = Arc::into_raw(arc) as u64;
                self.stack[slot] = new_bits;
                self.kinds[slot] = arr_kind;
                return r;
            }
            _ => Err(VMError::TypeError {
                expected: "Array<int>",
                got: "non-array slot",
            }),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

    // -----------------------------------------------------------------------
    // ArrayPushF64
    // -----------------------------------------------------------------------

    fn op_array_push_f64_elem(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (val_bits, _vk) = self.pop_kinded()?;
        let val = f64::from_bits(val_bits);

        let (arr_bits, arr_kind) = self.stack_take_kinded(slot);
        let result = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *mut TypedArray<f64>;
                unsafe { TypedArray::push(arr, val) };
                Ok(())
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let r = match Arc::make_mut(&mut arc) {
                    TypedArrayData::F64(buf) => {
                        Arc::make_mut(buf).data.push(val);
                        Ok(())
                    }
                    other => Err(VMError::TypeError {
                        expected: "Array<number>",
                        got: other.type_name(),
                    }),
                };
                let new_bits = Arc::into_raw(arc) as u64;
                self.stack[slot] = new_bits;
                self.kinds[slot] = arr_kind;
                return r;
            }
            _ => Err(VMError::TypeError {
                expected: "Array<number>",
                got: "non-array slot",
            }),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

    // -----------------------------------------------------------------------
    // ArrayLenTyped
    // -----------------------------------------------------------------------

    fn op_array_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (arr_bits, arr_kind) = self.stack_read_kinded_raw(slot);

        let len: usize = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *const TypedArray<u8>;
                unsafe { TypedArray::len(arr) as usize }
            }
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(arr_bits as *const TypedArrayData)
                };
                let len = match &*arc {
                    TypedArrayData::I64(buf) => buf.data.len(),
                    TypedArrayData::F64(buf) => buf.data.len(),
                    TypedArrayData::Bool(buf) => buf.data.len(),
                    TypedArrayData::I8(buf) => buf.data.len(),
                    TypedArrayData::I16(buf) => buf.data.len(),
                    TypedArrayData::I32(buf) => buf.data.len(),
                    TypedArrayData::U8(buf) => buf.data.len(),
                    TypedArrayData::U16(buf) => buf.data.len(),
                    TypedArrayData::U32(buf) => buf.data.len(),
                    TypedArrayData::U64(buf) => buf.data.len(),
                    TypedArrayData::F32(buf) => buf.data.len(),
                    // ADR-006 §2.7.22 amendment (Round 18 S3): Matrix /
                    // FloatSlice exit `TypedArrayData`.
                    TypedArrayData::String(buf) => buf.data.len(),
                    // W17-typed-carrier-bundle-A checkpoint 3/4: Q25.A specialized arms.
                    TypedArrayData::Decimal(b) => b.data.len(),
                    TypedArrayData::BigInt(b) => b.data.len(),
                    TypedArrayData::DateTime(b) => b.data.len(),
                    TypedArrayData::Timespan(b) => b.data.len(),
                    TypedArrayData::Duration(b) => b.data.len(),
                    TypedArrayData::Instant(b) => b.data.len(),
                    TypedArrayData::Char(b) => b.data.len(),
                    TypedArrayData::TypedObject(b) => b.data.len(),
                    TypedArrayData::TraitObject(b) => b.data.len(),
                };
                let _ = Arc::into_raw(arc);
                len
            }
            _ => {
                return Err(VMError::TypeError {
                    expected: "array",
                    got: "non-array slot",
                });
            }
        };

        // Suppress unused-import warning when no clone path is reached.
        let _ = clone_with_kind;
        let _ = drop_with_kind;
        self.push_kinded(len as i64 as u64, NativeKind::Int64)
    }
}
