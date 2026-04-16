//! Sized integer (i32) arithmetic and comparison handlers.
//!
//! Values are stored on the NaN-boxed stack as i64 (sign-extended from i32).
//! Operations are performed at i32 width with wrapping semantics, then
//! sign-extended back to i64 for storage.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::VirtualMachine;
use shape_value::VMError;

impl VirtualMachine {
    /// Execute a sized integer (i32) opcode.
    ///
    /// All operands are typed i64 (sign-extended i32) — the compiler proves
    /// this at emission time, so we use raw stack ops and skip ValueWord
    /// materialization entirely.
    pub(crate) fn exec_sized_int(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::AddI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_i64(a.wrapping_add(b) as i64)
            }
            OpCode::SubI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_i64(a.wrapping_sub(b) as i64)
            }
            OpCode::MulI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_i64(a.wrapping_mul(b) as i64)
            }
            OpCode::DivI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_raw_i64(a.wrapping_div(b) as i64)
            }
            OpCode::ModI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_raw_i64(a.wrapping_rem(b) as i64)
            }
            OpCode::EqI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_bool(a == b)
            }
            OpCode::NeqI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_bool(a != b)
            }
            OpCode::LtI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_bool(a < b)
            }
            OpCode::GtI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_bool(a > b)
            }
            OpCode::LteI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_bool(a <= b)
            }
            OpCode::GteI32 => {
                let b = self.pop_raw_i64()? as i32;
                let a = self.pop_raw_i64()? as i32;
                self.push_raw_bool(a >= b)
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled sized int opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}
