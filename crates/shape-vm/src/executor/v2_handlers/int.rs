//! v2 sized integer (i32) arithmetic and comparison handlers.
//!
//! Values are stored on the NaN-boxed stack as i64 (sign-extended from i32).
//! Operations are performed at i32 width with wrapping semantics, then
//! sign-extended back to i64 for storage.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};

impl VirtualMachine {
    /// Execute a v2 sized integer (i32) opcode.
    pub(crate) fn exec_v2_sized_int(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::AddI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_i64(a.wrapping_add(b) as i64))
            }
            OpCode::SubI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_i64(a.wrapping_sub(b) as i64))
            }
            OpCode::MulI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_i64(a.wrapping_mul(b) as i64))
            }
            OpCode::DivI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_vw(ValueWord::from_i64(a.wrapping_div(b) as i64))
            }
            OpCode::ModI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_vw(ValueWord::from_i64(a.wrapping_rem(b) as i64))
            }
            OpCode::EqI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_bool(a == b))
            }
            OpCode::NeqI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_bool(a != b))
            }
            OpCode::LtI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_bool(a < b))
            }
            OpCode::GtI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_bool(a > b))
            }
            OpCode::LteI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_bool(a <= b))
            }
            OpCode::GteI32 => {
                let b = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                let a = self.pop_vw()?.as_i64().unwrap_or(0) as i32;
                self.push_vw(ValueWord::from_bool(a >= b))
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled v2 sized int opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}
