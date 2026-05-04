//! v2 sized integer (i32) arithmetic and comparison handlers.
//!
//! Values are stored on the stack as raw native i64 bits (sign-extended from
//! i32). Operations are performed at i32 width with wrapping semantics, then
//! sign-extended back to i64 raw bits for storage. Comparisons push raw
//! native bool bits (0u64 / 1u64), matching the post-Wave-E+5 native bit
//! transport that surrounding typed opcodes (LoadLocalI32, AddInt, EqInt,
//! …) already use. Producer-side `last_emitted_native_kind` advertises this.
//!
//! Pre-flip (Wave E+5.3 baseline) the body called `pop_tagged_i64` /
//! `push_tagged_i64` / `push_tagged_bool`, but every adjacent typed opcode
//! pushes raw native bits via `push_raw_u64` (e.g. `op_load_local_i32` at
//! `variables/mod.rs:2858`), so the tagged decoders saw raw native i32 sign-
//! extended bits and read them through `sign_extend_i48(get_payload(bits))`,
//! corrupting both operands and producing the `Some(-1970324836974292)`
//! pattern observed in the i32 unit tests.

use crate::bytecode::{Instruction, OpCode};
use crate::executor::VirtualMachine;
use shape_value::VMError;

impl VirtualMachine {
    /// Execute a v2 sized integer (i32) opcode.
    ///
    /// All operands arrive as raw native i64 bits (sign-extended i32) — the
    /// compiler proves this at emission time, so we use raw native ops and
    /// skip ValueWord materialization entirely. The result is sign-extended
    /// back to i64 raw bits for the i32 stack-slot storage contract.
    pub(crate) fn exec_v2_sized_int(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::AddI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_i64(a.wrapping_add(b) as i64)
            }
            OpCode::SubI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_i64(a.wrapping_sub(b) as i64)
            }
            OpCode::MulI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_i64(a.wrapping_mul(b) as i64)
            }
            OpCode::DivI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_native_i64(a.wrapping_div(b) as i64)
            }
            OpCode::ModI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                if b == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_native_i64(a.wrapping_rem(b) as i64)
            }
            OpCode::EqI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_bool(a == b)
            }
            OpCode::NeqI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_bool(a != b)
            }
            OpCode::LtI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_bool(a < b)
            }
            OpCode::GtI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_bool(a > b)
            }
            OpCode::LteI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_bool(a <= b)
            }
            OpCode::GteI32 => {
                let b = self.pop_native_i64()? as i32;
                let a = self.pop_native_i64()? as i32;
                self.push_native_bool(a >= b)
            }
            _ => Err(VMError::RuntimeError(format!(
                "unhandled v2 sized int opcode: {:?}",
                instruction.opcode
            ))),
        }
    }
}
