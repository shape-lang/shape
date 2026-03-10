//! Built-in function operations for the VM executor
//!
//! Handles: BuiltinCall, TypeCheck, Convert

// Builtin handler modules
mod array_comprehension;
mod array_ops;
mod datetime_builtins;
mod generators;
pub mod intrinsics;
mod json_helpers;
mod math;
mod matrix_intrinsics;
mod object_ops;
pub mod remote_builtins;
mod runtime_delegated;
mod special_ops;
pub mod transport_builtins;
pub mod transport_provider;
mod type_ops;
mod vector_intrinsics;

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::VMError;
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_builtins(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            BuiltinCall => self.op_builtin_call(instruction, ctx)?,
            TypeCheck => self.op_type_check(instruction)?,
            Convert => self.op_convert(instruction)?,
            _ => unreachable!(
                "exec_builtins called with non-builtin opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }
}
