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
            And => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                // FilterExpr AND FilterExpr → compound FilterExpr (HeapValue path)
                if a.is_heap() || b.is_heap() {
                    if let (Some(left), Some(right)) =
                        (raw_helpers::extract_filter_expr(a.raw_bits()), raw_helpers::extract_filter_expr(b.raw_bits()))
                    {
                        self.push_raw_u64(ValueWord::from_filter_expr(Arc::new(FilterNode::And(
                            Box::new(left.as_ref().clone()),
                            Box::new(right.as_ref().clone()),
                        ))))?;
                    } else {
                        self.push_raw_bool(a.is_truthy() && b.is_truthy())?;
                    }
                } else {
                    // Fast path: non-heap values, just check truthiness
                    self.push_raw_bool(a.is_truthy() && b.is_truthy())?;
                }
            }
            Or => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                // FilterExpr OR FilterExpr → compound FilterExpr (HeapValue path)
                if a.is_heap() || b.is_heap() {
                    if let (Some(left), Some(right)) =
                        (raw_helpers::extract_filter_expr(a.raw_bits()), raw_helpers::extract_filter_expr(b.raw_bits()))
                    {
                        self.push_raw_u64(ValueWord::from_filter_expr(Arc::new(FilterNode::Or(
                            Box::new(left.as_ref().clone()),
                            Box::new(right.as_ref().clone()),
                        ))))?;
                    } else {
                        self.push_raw_bool(a.is_truthy() || b.is_truthy())?;
                    }
                } else {
                    // Fast path: non-heap values, just check truthiness
                    self.push_raw_bool(a.is_truthy() || b.is_truthy())?;
                }
            }
            Not => {
                let val = self.pop_raw_u64()?;
                // FilterExpr NOT → compound FilterExpr (HeapValue path)
                if val.is_heap() {
                    if let Some(node) = raw_helpers::extract_filter_expr(val.raw_bits()) {
                        self.push_raw_u64(ValueWord::from_filter_expr(Arc::new(FilterNode::Not(
                            Box::new(node.as_ref().clone()),
                        ))))?;
                    } else {
                        self.push_raw_bool(!val.is_truthy())?;
                    }
                } else {
                    // Fast path: non-heap value, just negate truthiness
                    self.push_raw_bool(!val.is_truthy())?;
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
