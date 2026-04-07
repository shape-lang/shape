//! Stack operations for the VM executor
//!
//! Handles basic stack manipulation: push, pop, dup, swap

use std::sync::Arc;

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::{VMError, ValueWord};
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_stack_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            PushConst => self.op_push_const(instruction)?,
            PushNull => self.push_vw(ValueWord::none())?,
            Pop => {
                self.pop_vw()?;
            }
            Dup => {
                // Clone the ValueWord directly from the stack, avoiding ValueWord round-trip
                let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
                if index >= self.stack.len() {
                    return Err(VMError::StackUnderflow);
                }
                let val = self.stack_read_vw(index);
                self.push_vw(val)?;
            }
            Swap => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(b)?;
                self.push_vw(a)?;
            }
            _ => unreachable!(
                "exec_stack_ops called with non-stack opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    pub(in crate::executor) fn op_push_const(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Const(idx)) = instruction.operand {
            let constant = self
                .program
                .constants
                .get(idx as usize)
                .ok_or(VMError::InvalidOperand)?;

            // Stage 2.2: For typed scalar constants (Number/Int/Bool), push the
            // raw bits directly via push_raw_* — skips the ValueWord wrapper
            // construction so downstream typed handlers (e.g. exec_typed_arithmetic)
            // can pop_raw_* without unwrapping. Encoding is identical to what
            // ValueWord::from_*() would produce, so legacy pop_vw consumers
            // (which transmute the raw bits back into a ValueWord) keep working.
            match constant {
                crate::bytecode::Constant::Number(n) => {
                    return self.push_raw_f64(*n);
                }
                crate::bytecode::Constant::Int(i) => {
                    // In-range i48: push raw tagged bits. Out-of-range falls
                    // back to ValueWord::from_i64 which heap-boxes as BigInt.
                    if *i >= shape_value::tags::I48_MIN && *i <= shape_value::tags::I48_MAX {
                        return self.push_raw_i64(*i);
                    }
                    return self.push_vw(ValueWord::from_i64(*i));
                }
                crate::bytecode::Constant::UInt(u) => {
                    // In-range i48 (u <= I48_MAX): push raw tagged bits.
                    // Otherwise fall back to ValueWord constructors.
                    if *u <= shape_value::tags::I48_MAX as u64 {
                        return self.push_raw_i64(*u as i64);
                    }
                    return if *u <= i64::MAX as u64 {
                        self.push_vw(ValueWord::from_i64(*u as i64))
                    } else {
                        self.push_vw(ValueWord::from_native_u64(*u))
                    };
                }
                crate::bytecode::Constant::Bool(b) => {
                    return self.push_raw_bool(*b);
                }
                crate::bytecode::Constant::Null => return self.push_vw(ValueWord::none()),
                crate::bytecode::Constant::Unit => return self.push_vw(ValueWord::unit()),
                crate::bytecode::Constant::Function(id) => {
                    return self.push_vw(ValueWord::from_function(*id));
                }
                _ => {}
            }

            // For types with direct ValueWord constructors, skip ValueWord
            match constant {
                crate::bytecode::Constant::String(s) => {
                    return self.push_vw(ValueWord::from_string(Arc::new(s.clone())));
                }
                crate::bytecode::Constant::Char(c) => {
                    return self.push_vw(ValueWord::from_char(*c));
                }
                crate::bytecode::Constant::Decimal(d) => {
                    return self.push_vw(ValueWord::from_decimal(*d));
                }
                _ => {}
            }

            // For remaining complex types, construct HeapValue directly (no ValueWord)
            use shape_value::heap_value::HeapValue;
            let heap_val = match constant {
                crate::bytecode::Constant::Timeframe(tf) => HeapValue::Timeframe(*tf),
                crate::bytecode::Constant::Duration(duration) => {
                    // Convert AST Duration to chrono::Duration (TimeSpan) so it
                    // participates in DateTime arithmetic (Time +/- TimeSpan).
                    let chrono_dur =
                        crate::executor::builtins::datetime_builtins::ast_duration_to_chrono(
                            duration,
                        );
                    HeapValue::TimeSpan(chrono_dur)
                }
                crate::bytecode::Constant::TimeReference(time_ref) => {
                    HeapValue::TimeReference(Box::new(time_ref.clone()))
                }
                crate::bytecode::Constant::DateTimeExpr(expr) => {
                    HeapValue::DateTimeExpr(Box::new(expr.clone()))
                }
                crate::bytecode::Constant::DataDateTimeRef(expr) => {
                    HeapValue::DataDateTimeRef(Box::new(expr.clone()))
                }
                crate::bytecode::Constant::TypeAnnotation(type_annotation) => {
                    HeapValue::TypeAnnotation(Box::new(type_annotation.clone()))
                }
                crate::bytecode::Constant::Value(val) => {
                    return self.push_vw(val.clone());
                }
                // Simple types and String/Decimal already handled above
                _ => unreachable!(),
            };

            self.push_vw(ValueWord::from_heap_value(heap_val))?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }
}
