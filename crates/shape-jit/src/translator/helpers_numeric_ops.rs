//! Numeric helper methods split out of `helpers.rs` for maintainability.

use cranelift::prelude::*;
use shape_vm::bytecode::OpCode;
use shape_vm::type_tracking::StorageHint;

use crate::nan_boxing::*;

use super::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Returns true when next opcode immediately branches on top-of-stack bool.
    fn next_instr_is_conditional_jump(&self) -> bool {
        let next_idx = self.current_instr_idx + 1;
        if next_idx >= self.program.instructions.len() {
            return false;
        }
        matches!(
            self.program.instructions[next_idx].opcode,
            OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue
        )
    }

    /// Push comparison result while retaining raw cmp flag for fused branches.
    pub(in crate::translator) fn push_cmp_bool_result(&mut self, cmp: Value) {
        let result = if self.next_instr_is_conditional_jump() {
            self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64)
        } else {
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            self.builder.ins().select(cmp, true_val, false_val)
        };
        self.stack_push_typed(result, StorageHint::Bool);
        self.typed_stack
            .replace_top(super::storage::TypedValue::bool_with_raw_cmp(result, cmp));
    }

    /// Propagate a known result type to the top of the stack after an operation.
    /// Used by typed arithmetic opcodes that bypass `typed_binary_op()`.
    pub(in crate::translator) fn propagate_result_type(&mut self, hint: StorageHint) {
        if self.stack_depth > 0 {
            self.stack_types.insert(self.stack_depth - 1, hint);
        }
    }

    /// Raw f64 binary operation — operands are already raw f64.
    pub(in crate::translator) fn raw_f64_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let b_f64 = self.stack_pop_f64().unwrap();
            let a_f64 = self.stack_pop_f64().unwrap();
            let result_f64 = op(self.builder, a_f64, b_f64);
            let result_boxed = self.f64_to_i64(result_f64);
            self.stack_push(result_boxed);
            self.typed_stack
                .replace_top(super::storage::TypedValue::f64(result_f64));
        }
    }

    /// Raw f64 comparison — operands are already raw f64.
    pub(in crate::translator) fn raw_f64_comparison(&mut self, cc: FloatCC) {
        if self.stack_len() >= 2 {
            let b_f64 = self.stack_pop_f64().unwrap();
            let a_f64 = self.stack_pop_f64().unwrap();
            let cmp = self.builder.ins().fcmp(cc, a_f64, b_f64);
            self.push_cmp_bool_result(cmp);
        }
    }

    /// Raw i64 binary operation — operands are already raw i64.
    pub(in crate::translator) fn raw_int64_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let b = self.stack_pop().unwrap();
            let a = self.stack_pop().unwrap();
            let result = op(self.builder, a, b);
            self.stack_push(result);
            self.typed_stack
                .replace_top(super::storage::TypedValue::i64(result));
        }
    }

    /// Mixed i64 binary operation — one operand is raw i64, the other boxed.
    pub(in crate::translator) fn mixed_int64_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let (top_is_i64, second_is_i64) = self.typed_stack.top_two_i64_flags();
            let b_raw = self.stack_pop().unwrap();
            let a_raw = self.stack_pop().unwrap();

            let a = if second_is_i64 {
                a_raw
            } else {
                let f = self.i64_to_f64(a_raw);
                self.builder.ins().fcvt_to_sint_sat(types::I64, f)
            };
            let b = if top_is_i64 {
                b_raw
            } else {
                let f = self.i64_to_f64(b_raw);
                self.builder.ins().fcvt_to_sint_sat(types::I64, f)
            };

            let result = op(self.builder, a, b);
            self.stack_push(result);
            self.typed_stack
                .replace_top(super::storage::TypedValue::i64(result));
        }
    }

    /// Mixed numeric binary operation for generic arithmetic opcodes.
    pub(in crate::translator) fn mixed_numeric_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let (top_is_i64, second_is_i64) = self.typed_stack.top_two_i64_flags();
            let b_raw = self.stack_pop().unwrap();
            let a_raw = self.stack_pop().unwrap();

            let a_f64 = if second_is_i64 {
                self.builder.ins().fcvt_from_sint(types::F64, a_raw)
            } else {
                self.i64_to_f64(a_raw)
            };
            let b_f64 = if top_is_i64 {
                self.builder.ins().fcvt_from_sint(types::F64, b_raw)
            } else {
                self.i64_to_f64(b_raw)
            };

            let result_f64 = op(self.builder, a_f64, b_f64);
            let result_boxed = self.f64_to_i64(result_f64);
            self.stack_push(result_boxed);
            // Track f64 result in typed_stack so subsequent pops skip bitcasts
            self.typed_stack
                .replace_top(super::storage::TypedValue::f64(result_f64));
        }
    }

    /// Mixed i64 comparison — one operand is raw i64, the other boxed.
    pub(in crate::translator) fn mixed_int64_comparison(&mut self, cc: IntCC) {
        if self.stack_len() >= 2 {
            let (top_is_i64, second_is_i64) = self.typed_stack.top_two_i64_flags();
            let b_raw = self.stack_pop().unwrap();
            let a_raw = self.stack_pop().unwrap();

            let a = if second_is_i64 {
                a_raw
            } else {
                let f = self.i64_to_f64(a_raw);
                self.builder.ins().fcvt_to_sint_sat(types::I64, f)
            };
            let b = if top_is_i64 {
                b_raw
            } else {
                let f = self.i64_to_f64(b_raw);
                self.builder.ins().fcvt_to_sint_sat(types::I64, f)
            };

            let cmp = self.builder.ins().icmp(cc, a, b);
            self.push_cmp_bool_result(cmp);
        }
    }

    /// Mixed numeric comparison for generic comparison opcodes.
    pub(in crate::translator) fn mixed_numeric_comparison(&mut self, cc: FloatCC) {
        if self.stack_len() >= 2 {
            let (top_is_i64, second_is_i64) = self.typed_stack.top_two_i64_flags();
            let b_raw = self.stack_pop().unwrap();
            let a_raw = self.stack_pop().unwrap();

            let a_f64 = if second_is_i64 {
                self.builder.ins().fcvt_from_sint(types::F64, a_raw)
            } else {
                self.i64_to_f64(a_raw)
            };
            let b_f64 = if top_is_i64 {
                self.builder.ins().fcvt_from_sint(types::F64, b_raw)
            } else {
                self.i64_to_f64(b_raw)
            };

            let cmp = self.builder.ins().fcmp(cc, a_f64, b_f64);
            self.push_cmp_bool_result(cmp);
        }
    }

    /// Raw i64 comparison — operands are already raw i64.
    pub(in crate::translator) fn raw_int64_comparison(&mut self, cc: IntCC) {
        if self.stack_len() >= 2 {
            let b = self.stack_pop().unwrap();
            let a = self.stack_pop().unwrap();
            let cmp = self.builder.ins().icmp(cc, a, b);
            self.push_cmp_bool_result(cmp);
        }
    }
}
