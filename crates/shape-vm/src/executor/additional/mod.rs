//! Additional operations for the VM executor
//!
//! Handles: SliceAccess, NullCoalesce, MakeRange

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::VMError;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_additional(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            SliceAccess => self.op_slice_access()?,
            NullCoalesce => self.op_null_coalesce()?,
            MakeRange => self.op_make_range()?,
            _ => unreachable!(
                "exec_additional called with non-additional opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }
}
