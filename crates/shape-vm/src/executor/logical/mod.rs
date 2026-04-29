//! Logical operations for the VM executor
//!
//! Handles: And, Or, Not

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use crate::executor::objects::raw_helpers;
use shape_value::{FilterNode, VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_logical(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            // E+5.4: And/Or/Not are listed by `last_instruction_produces_bool()`
            // as bool-producers eligible for the `JumpIfFalse → JumpIfFalseTrusted`
            // upgrade. JumpIfFalseTrusted now pops native bool bits, so these
            // logical ops must push native bool too. The FilterExpr branches
            // (heap path used by query DSL) keep pushing a heap-tagged ValueWord
            // — those values are never fed to JumpIfFalseTrusted (filters are
            // consumed by query plumbing, not control flow).
            And => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                if a.is_heap() || b.is_heap() {
                    if let (Some(left), Some(right)) =
                        (raw_helpers::extract_filter_expr(a.raw_bits()), raw_helpers::extract_filter_expr(b.raw_bits()))
                    {
                        self.push_raw_u64(ValueWord::from_filter_expr(Arc::new(FilterNode::And(
                            Box::new(left.as_ref().clone()),
                            Box::new(right.as_ref().clone()),
                        ))))?;
                    } else {
                        self.push_native_bool(a.is_truthy() && b.is_truthy())?;
                    }
                } else {
                    self.push_native_bool(a.is_truthy() && b.is_truthy())?;
                }
            }
            Or => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                if a.is_heap() || b.is_heap() {
                    if let (Some(left), Some(right)) =
                        (raw_helpers::extract_filter_expr(a.raw_bits()), raw_helpers::extract_filter_expr(b.raw_bits()))
                    {
                        self.push_raw_u64(ValueWord::from_filter_expr(Arc::new(FilterNode::Or(
                            Box::new(left.as_ref().clone()),
                            Box::new(right.as_ref().clone()),
                        ))))?;
                    } else {
                        self.push_native_bool(a.is_truthy() || b.is_truthy())?;
                    }
                } else {
                    self.push_native_bool(a.is_truthy() || b.is_truthy())?;
                }
            }
            Not => {
                let val = self.pop_raw_u64()?;
                if val.is_heap() {
                    if let Some(node) = raw_helpers::extract_filter_expr(val.raw_bits()) {
                        self.push_raw_u64(ValueWord::from_filter_expr(Arc::new(FilterNode::Not(
                            Box::new(node.as_ref().clone()),
                        ))))?;
                    } else {
                        self.push_native_bool(!val.is_truthy())?;
                    }
                } else {
                    self.push_native_bool(!val.is_truthy())?;
                }
            }
            _ => unreachable!(
                "exec_logical called with non-logical opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Null coalescing operator: returns left if not null, otherwise right
    pub(in crate::executor) fn op_null_coalesce(&mut self) -> Result<(), VMError> {
        let right = self.pop_raw_u64()?;
        let left = self.pop_raw_u64()?;

        // Return left if not null, otherwise return right
        if left.is_none() {
            self.push_raw_u64(right)
        } else {
            self.push_raw_u64(left)
        }
    }
}
