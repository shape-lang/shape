//! Stack operations for the VM executor
//!
//! Handles basic stack manipulation: push, pop, dup, swap

use std::sync::Arc;

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::{VMError, ValueWord, ValueWordExt};
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_stack_ops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            PushConst => self.op_push_const(instruction)?,
            PushNull => self.push_raw_u64(ValueWord::none())?,
            Pop => {
                self.pop_raw_u64()?;
            }
            Dup => {
                // Clone the ValueWord directly from the stack, avoiding ValueWord round-trip
                let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
                if index >= self.stack.len() {
                    return Err(VMError::StackUnderflow);
                }
                let val = self.stack_read_raw(index);
                self.push_raw_u64(val)?;
            }
            Swap => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                self.push_raw_u64(b)?;
                self.push_raw_u64(a)?;
            }
            PromoteToOwned => self.op_promote_to_owned()?,
            _ => unreachable!(
                "exec_stack_ops called with non-stack opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Promote the top-of-stack value from shared (Arc) to owned (Box) allocation
    /// if the heap refcount is exactly 1 (sole owner). This is a safe optimization:
    /// - Inline values (int, float, bool, null): no-op.
    /// - Already-owned heap values: no-op.
    /// - Shared heap values with refcount > 1: no-op (cannot convert safely).
    /// - Shared heap values with refcount == 1: Arc is unwrapped into Box.
    ///
    /// When the `gc` feature is enabled, ownership is managed by the GC, so
    /// this is a no-op.
    #[inline(always)]
    fn op_promote_to_owned(&mut self) -> Result<(), VMError> {
        #[cfg(feature = "gc")]
        {
            return Ok(());
        }

        #[cfg(not(feature = "gc"))]
        {
            use shape_value::tags::{
                get_payload, get_tag, is_tagged, HEAP_OWNED_BIT, HEAP_PTR_MASK, TAG_HEAP,
            };
            use shape_value::heap_value::HeapValue;

            let index = self.sp.checked_sub(1).ok_or(VMError::StackUnderflow)?;
            let bits = self.stack[index];

            // Fast exit: not a heap-tagged value (inline scalar, function, etc.)
            if !is_tagged(bits) || get_tag(bits) != TAG_HEAP {
                return Ok(());
            }

            let payload = get_payload(bits);

            // Already owned — nothing to do
            if (payload & HEAP_OWNED_BIT) != 0 {
                return Ok(());
            }

            // Shared (Arc-backed). Check if we are the sole owner.
            let ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
            if ptr.is_null() {
                return Ok(());
            }

            // Reconstruct Arc without consuming it (ManuallyDrop prevents decrement).
            let arc = std::mem::ManuallyDrop::new(unsafe { Arc::from_raw(ptr) });
            if Arc::strong_count(&arc) == 1 {
                // Sole owner — safe to convert Arc -> Box.
                let arc = std::mem::ManuallyDrop::into_inner(arc);
                match Arc::try_unwrap(arc) {
                    Ok(hv) => {
                        let new_bits = shape_value::tags::vw_heap_box_owned(hv);
                        // Replace top of stack in-place (no push/pop needed).
                        self.stack[index] = new_bits;
                    }
                    Err(_arc) => {
                        // Race: someone else got a ref between check and unwrap.
                        // Err(arc) returns the Arc; its Drop will decrement.
                        // The stack still holds the original bits expecting an
                        // Arc refcount, so bump to compensate for the drop.
                        let raw_ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
                        unsafe { Arc::increment_strong_count(raw_ptr); }
                    }
                }
            }
            // If refcount > 1, leave as shared. ManuallyDrop prevents decrement.
            Ok(())
        }
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
                    return self.push_raw_u64(ValueWord::from_i64(*i));
                }
                crate::bytecode::Constant::UInt(u) => {
                    // In-range i48 (u <= I48_MAX): push raw tagged bits.
                    // Otherwise fall back to ValueWord constructors.
                    if *u <= shape_value::tags::I48_MAX as u64 {
                        return self.push_raw_i64(*u as i64);
                    }
                    return if *u <= i64::MAX as u64 {
                        self.push_raw_u64(ValueWord::from_i64(*u as i64))
                    } else {
                        self.push_raw_u64(ValueWord::from_native_u64(*u))
                    };
                }
                crate::bytecode::Constant::Bool(b) => {
                    return self.push_raw_bool(*b);
                }
                crate::bytecode::Constant::Null => return self.push_raw_u64(ValueWord::none()),
                crate::bytecode::Constant::Unit => return self.push_raw_u64(ValueWord::unit()),
                crate::bytecode::Constant::Function(id) => {
                    return self.push_raw_u64(ValueWord::from_function(*id));
                }
                _ => {}
            }

            // For types with direct ValueWord constructors, skip ValueWord
            match constant {
                crate::bytecode::Constant::String(s) => {
                    return self.push_raw_u64(ValueWord::from_string(Arc::new(s.clone())));
                }
                crate::bytecode::Constant::Char(c) => {
                    return self.push_raw_u64(ValueWord::from_char(*c));
                }
                crate::bytecode::Constant::Decimal(d) => {
                    return self.push_raw_u64(ValueWord::from_decimal(*d));
                }
                _ => {}
            }

            // For remaining complex types, construct HeapValue directly (no ValueWord)
            use shape_value::heap_value::HeapValue;
            let heap_val = match constant {
                crate::bytecode::Constant::Timeframe(tf) => HeapValue::Temporal(shape_value::TemporalData::Timeframe(*tf)),
                crate::bytecode::Constant::Duration(duration) => {
                    // Convert AST Duration to chrono::Duration (TimeSpan) so it
                    // participates in DateTime arithmetic (Time +/- TimeSpan).
                    let chrono_dur =
                        crate::executor::builtins::datetime_builtins::ast_duration_to_chrono(
                            duration,
                        );
                    HeapValue::Temporal(shape_value::TemporalData::TimeSpan(chrono_dur))
                }
                crate::bytecode::Constant::TimeReference(time_ref) => {
                    HeapValue::Temporal(shape_value::TemporalData::TimeReference(Box::new(time_ref.clone())))
                }
                crate::bytecode::Constant::DateTimeExpr(expr) => {
                    HeapValue::Temporal(shape_value::TemporalData::DateTimeExpr(Box::new(expr.clone())))
                }
                crate::bytecode::Constant::DataDateTimeRef(expr) => {
                    HeapValue::Temporal(shape_value::TemporalData::DataDateTimeRef(Box::new(expr.clone())))
                }
                crate::bytecode::Constant::TypeAnnotation(type_annotation) => {
                    HeapValue::Rare(shape_value::RareHeapData::TypeAnnotation(Box::new(type_annotation.clone())))
                }
                crate::bytecode::Constant::Value(val) => {
                    return self.push_raw_u64(val.clone());
                }
                // Simple types and String/Decimal already handled above
                _ => unreachable!(),
            };

            self.push_raw_u64(ValueWord::from_heap_value(heap_val))?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }
}
