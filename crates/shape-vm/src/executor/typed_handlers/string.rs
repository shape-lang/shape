//! VM executor handlers for typed string opcodes.
//!
//! These handlers operate on `StringObj` raw pointers (`*const StringObj`),
//! which are NativeScalar-shaped — non-Arc, the StringObj manages its own
//! lifetime. The pointer bits are pushed/popped through the kinded API as
//! `NativeKind::UInt64` (inline scalar — Drop is a no-op, no refcount).
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C: kinded API. These opcodes
//! (`NewStringV2`, `StringLenV2`, `StringConcatV2`, `StringEqV2`) are
//! currently dead at the compiler level (no emitter). The handlers
//! preserve their semantics for future re-emission.

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::v2::string_obj::StringObj;
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;

impl VirtualMachine {
    /// Execute a typed string opcode.
    pub(crate) fn exec_typed_string(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            // ── New string from constant ───────────────────────────────

            OpCode::NewStringV2 => {
                let str_id = match instruction.operand {
                    Some(Operand::Property(id)) => id as usize,
                    Some(Operand::Const(id)) => id as usize,
                    _ => {
                        return Err(VMError::NotImplemented(
                            "NewStringV2 requires a string operand".to_string(),
                        ));
                    }
                };
                let s = self
                    .program
                    .strings
                    .get(str_id)
                    .cloned()
                    .unwrap_or_default();
                let ptr = StringObj::new(&s);
                // StringObj raw pointer encoded as u64. NativeScalar shape —
                // not Arc-backed. UInt64 kind selects the no-op Drop arm.
                self.push_kinded(ptr as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }

            // ── String length ──────────────────────────────────────────

            OpCode::StringLenV2 => {
                let (str_bits, _str_kind) = self.pop_kinded()?;
                let str_ptr = str_bits as usize as *const StringObj;
                // Safety: str_ptr was created by NewStringV2 or string FFI.
                let len = unsafe { StringObj::len(str_ptr) };
                self.push_kinded(len as u64, NativeKind::Int64)?;
                Ok(())
            }

            // ── String concatenation ───────────────────────────────────

            OpCode::StringConcatV2 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let a_ptr = a_bits as usize as *const StringObj;
                let b_ptr = b_bits as usize as *const StringObj;
                // Safety: both pointers were created by NewStringV2 or string FFI.
                let result = unsafe { StringObj::concat(a_ptr, b_ptr) };
                self.push_kinded(result as usize as u64, NativeKind::UInt64)?;
                Ok(())
            }

            // ── String equality ────────────────────────────────────────

            OpCode::StringEqV2 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let a_ptr = a_bits as usize as *const StringObj;
                let b_ptr = b_bits as usize as *const StringObj;
                // Safety: both pointers were created by NewStringV2 or string FFI.
                let eq = unsafe { StringObj::eq(a_ptr, b_ptr) };
                self.push_kinded(eq as u64, NativeKind::Bool)?;
                Ok(())
            }

            _ => Err(VMError::NotImplemented(format!(
                "typed string opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
