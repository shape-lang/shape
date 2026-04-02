//! VM executor handlers for v2 string opcodes.
//!
//! These handlers operate on `StringObj` pointers stored as
//! `ValueWord::from_native_ptr()` (heap-boxed `NativeScalar::Ptr`).

use crate::bytecode::{Instruction, OpCode, Operand};
use shape_value::heap_value::NativeScalar;
use shape_value::v2::string_obj::StringObj;
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
                self.push_vw(ValueWord::from_native_ptr(ptr as usize))?;
                Ok(())
            }

            // ── String length ──────────────────────────────────────────

            OpCode::StringLenV2 => {
                let str_vw = self.pop_vw()?;
                let str_ptr = extract_ptr(&str_vw) as *const StringObj;
                // Safety: str_ptr was created by NewStringV2 or string FFI.
                let len = unsafe { StringObj::len(str_ptr) };
                self.push_vw(ValueWord::from_i64(len as i64))?;
                Ok(())
            }

            // ── String concatenation ───────────────────────────────────

            OpCode::StringConcatV2 => {
                let b_vw = self.pop_vw()?;
                let a_vw = self.pop_vw()?;
                let a_ptr = extract_ptr(&a_vw) as *const StringObj;
                let b_ptr = extract_ptr(&b_vw) as *const StringObj;
                // Safety: both pointers were created by NewStringV2 or string FFI.
                let result = unsafe { StringObj::concat(a_ptr, b_ptr) };
                self.push_vw(ValueWord::from_native_ptr(result as usize))?;
                Ok(())
            }

            // ── String equality ────────────────────────────────────────

            OpCode::StringEqV2 => {
                let b_vw = self.pop_vw()?;
                let a_vw = self.pop_vw()?;
                let a_ptr = extract_ptr(&a_vw) as *const StringObj;
                let b_ptr = extract_ptr(&b_vw) as *const StringObj;
                // Safety: both pointers were created by NewStringV2 or string FFI.
                let eq = unsafe { StringObj::eq(a_ptr, b_ptr) };
                self.push_vw(ValueWord::from_bool(eq))?;
                Ok(())
            }

            _ => Err(VMError::NotImplemented(format!(
                "v2 string opcode {:?} not implemented",
                instruction.opcode
            ))),
        }
    }
}
