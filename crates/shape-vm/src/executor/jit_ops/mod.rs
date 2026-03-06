//! JIT-optimized operations for the VM executor
//!
//! Handles: GetFieldTyped, SetFieldTyped

use crate::{
    bytecode::{Instruction, OpCode},
    executor::{VirtualMachine, typed_object_ops::TypedObjectOps},
};
use shape_value::VMError;
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_jit_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            GetFieldTyped => self.op_get_field_typed(instruction)?,
            SetFieldTyped => self.op_set_field_typed(instruction)?,
            _ => unreachable!(
                "exec_jit_ops called with non-jit opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }
}
