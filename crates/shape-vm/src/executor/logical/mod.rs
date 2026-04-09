//! Logical operations for the VM executor
//!
//! Handles: And, Or, Not

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::heap_value::HeapValue;
use shape_value::{FilterNode, VMError, ValueWord};
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
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                // FilterExpr AND FilterExpr → compound FilterExpr (HeapValue path)
                if a.is_heap() || b.is_heap() {
                    if let (Some(HeapValue::FilterExpr(left)), Some(HeapValue::FilterExpr(right))) =
                        (a.as_heap_ref(), b.as_heap_ref())
                    {
                        self.push_vw(ValueWord::from_filter_expr(Arc::new(FilterNode::And(
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
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                // FilterExpr OR FilterExpr → compound FilterExpr (HeapValue path)
                if a.is_heap() || b.is_heap() {
                    if let (Some(HeapValue::FilterExpr(left)), Some(HeapValue::FilterExpr(right))) =
                        (a.as_heap_ref(), b.as_heap_ref())
                    {
                        self.push_vw(ValueWord::from_filter_expr(Arc::new(FilterNode::Or(
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
                let val = self.pop_vw()?;
                // FilterExpr NOT → compound FilterExpr (HeapValue path)
                if val.is_heap() {
                    if let Some(HeapValue::FilterExpr(node)) = val.as_heap_ref() {
                        self.push_vw(ValueWord::from_filter_expr(Arc::new(FilterNode::Not(
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
        let right = self.pop_vw()?;
        let left = self.pop_vw()?;

        // Return left if not null, otherwise return right
        if left.is_none() {
            self.push_vw(right)
        } else {
            self.push_vw(left)
        }
    }
}
