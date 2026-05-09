//! v2 sized integer (i32) arithmetic and comparison handlers.
//!
//! Values flow through the kinded VM stack as i64-shaped bits with
//! `NativeKind::Int32` (Wave 6.5 cluster C — ADR-006 §2.7.7). Operations
//! are performed at i32 width with wrapping semantics; the result is
//! sign-extended to i64 for the slot's u64 payload.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::VirtualMachine;
use shape_value::{NativeKind, VMError};

impl VirtualMachine {
    /// Execute a v2 sized integer (i32) opcode.
    ///
    /// All operands arrive as raw bits with `NativeKind::Int32` — the
    /// compiler proves this at emission time, so the kinded pop reads the
    /// bits as i64 (sign-extended) and discards the kind tag.
    pub(crate) fn exec_v2_sized_int(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::AddI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded(a.wrapping_add(b) as i64 as u64, NativeKind::Int32)
            }
            OpCode::SubI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded(a.wrapping_sub(b) as i64 as u64, NativeKind::Int32)
            }
            OpCode::MulI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded(a.wrapping_mul(b) as i64 as u64, NativeKind::Int32)
            }
            OpCode::DivI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_kinded(a.wrapping_div(b) as i64 as u64, NativeKind::Int32)
            }
            OpCode::ModI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_kinded(a.wrapping_rem(b) as i64 as u64, NativeKind::Int32)
            }
            OpCode::EqI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded((a == b) as u64, NativeKind::Bool)
            }
            OpCode::NeqI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded((a != b) as u64, NativeKind::Bool)
            }
            OpCode::LtI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded((a < b) as u64, NativeKind::Bool)
            }
            OpCode::GtI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded((a > b) as u64, NativeKind::Bool)
            }
            OpCode::LteI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded((a <= b) as u64, NativeKind::Bool)
            }
            OpCode::GteI32 => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let b = b_bits as i64 as i32;
                let a = a_bits as i64 as i32;
                self.push_kinded((a >= b) as u64, NativeKind::Bool)
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled v2 sized int opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}
