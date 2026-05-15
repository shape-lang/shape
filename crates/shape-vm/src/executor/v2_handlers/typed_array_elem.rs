//! Typed array element access opcodes (local-slot based).
//!
//! These opcodes skip the HeapValue enum dispatch when the compiler proves the
//! element type. The array lives in a local slot (Operand::Local) as a raw
//! v2 pointer `*mut TypedArray<T>` — kind = `NativeKind::UInt64`
//! (NativeScalar shape). Bits = raw pointer (no Arc).
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C: kinded API. Dispatch on the kind
//! recorded in the parallel kind track for the local slot.
//!
//! ## V3-S5 ckpt-5 consumer-cascade tier 3 surface (2026-05-15)
//!
//! The pre-ckpt-1 file supported TWO carrier shapes:
//!   1. Legacy heap-boxed `Arc<TypedArrayData>` (Ptr(HeapKind::TypedArray)) — DELETED
//!   2. v2 raw-pointer `*mut TypedArray<T>` (NativeKind::UInt64) — PRESERVED
//!
//! Per V3-S5 ckpt-1..ckpt-4 cascade the `TypedArrayData` enum +
//! `TypedBuffer<T>` / `AlignedTypedBuffer` wrapper layer +
//! `HeapValue::TypedArray(Arc<TypedArrayData>)` outer arm +
//! `HeapKind::TypedArray = 8` ordinal were DELETED wholesale per
//! W12-typed-array-data-deletion audit §3.5 + §3.6 + §B + ADR-006
//! §2.7.24 Q25.A SUPERSEDED. The Arc-boxed arms in all 7 opcode handlers
//! are deleted; the UInt64 v2-raw path remains live and is the canonical
//! post-ckpt-6 STRICT close target.
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

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::v2::typed_array::TypedArray;
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder (for the deleted Ptr(HeapKind::
// TypedArray) Arc<TypedArrayData> receiver arm)
// ═══════════════════════════════════════════════════════════════════════════

#[cold]
#[inline(never)]
fn ckpt5_typed_array_surface(op: &'static str, kind: NativeKind) -> VMError {
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. \
         `Arc<TypedArrayData>` carrier + `HeapKind::TypedArray=8` ordinal \
         DELETED at V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion \
         audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED. UInt64 \
         v2-raw `*mut TypedArray<T>` path remains live. Slot kind: \
         {kind:?}. REFUSED ON SIGHT: TypedArrayData resurrection under \
         any rename (Refusal #1).",
        op = op,
        kind = kind,
    ))
}

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
            _ => return Err(ckpt5_typed_array_surface("GetElemI64", arr_kind)),
        };

        self.push_kinded(val as u64, NativeKind::Int64)
    }

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
            _ => return Err(ckpt5_typed_array_surface("GetElemF64", arr_kind)),
        };

        self.push_kinded(val.to_bits(), NativeKind::Float64)
    }

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
            _ => Err(ckpt5_typed_array_surface("SetElemI64", arr_kind)),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

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
            _ => Err(ckpt5_typed_array_surface("SetElemF64", arr_kind)),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

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
            _ => Err(ckpt5_typed_array_surface("ArrayPushI64", arr_kind)),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

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
            _ => Err(ckpt5_typed_array_surface("ArrayPushF64", arr_kind)),
        };
        self.stack[slot] = arr_bits;
        self.kinds[slot] = arr_kind;
        result
    }

    fn op_array_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot = self.resolve_local_slot(instruction)?;
        let (arr_bits, arr_kind) = self.stack_read_kinded_raw(slot);

        let len: usize = match arr_kind {
            NativeKind::UInt64 => {
                let arr = arr_bits as usize as *const TypedArray<u8>;
                unsafe { TypedArray::len(arr) as usize }
            }
            _ => return Err(ckpt5_typed_array_surface("ArrayLenTyped", arr_kind)),
        };

        self.push_kinded(len as i64 as u64, NativeKind::Int64)
    }
}
