//! VM executor handlers for v2 string opcodes.
//!
//! These handlers operate on `StringObj` raw pointers (`*const StringObj`),
//! NativeScalar-shaped (non-Arc). Pointer bits round-trip through the
//! kinded API as `NativeKind::UInt64` (inline scalar — no refcount).
//!
//! ADR-006 §2.7.7 / Wave 6.5 cluster C.

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::v2::string_obj::StringObj;
use shape_value::{NativeKind, VMError};

use super::super::VirtualMachine;

impl VirtualMachine {
    /// Execute a v2 string opcode.
    pub(crate) fn exec_v2_string(
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
                "v2 string opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
