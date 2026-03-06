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

    /// Compute result type for binary operations on nullable floats.
    /// Any nullable input produces nullable output.
    fn compute_binary_result_type(&self) -> StorageHint {
        if self.stack_depth < 2 {
            return StorageHint::Unknown;
        }
        let a = self
            .stack_types
            .get(&(self.stack_depth - 1))
            .copied()
            .unwrap_or(StorageHint::Unknown);
        let b = self
            .stack_types
            .get(&(self.stack_depth - 2))
            .copied()
            .unwrap_or(StorageHint::Unknown);

        if a.is_float_family() || b.is_float_family() {
            return if a == StorageHint::NullableFloat64 || b == StorageHint::NullableFloat64 {
                StorageHint::NullableFloat64
            } else {
                StorageHint::Float64
            };
        }
        if let Some(int_hint) = self.combine_integer_hints(a, b) {
            return int_hint;
        }
        StorageHint::Unknown
    }

    /// Propagate a known result type to the top of the stack after an operation.
    /// Used by typed arithmetic opcodes that bypass `typed_binary_op()`.
    pub(in crate::translator) fn propagate_result_type(&mut self, hint: StorageHint) {
        if self.stack_depth > 0 {
            self.stack_types.insert(self.stack_depth - 1, hint);
        }
    }

    /// Native i64 binary operation for typed integer opcodes.
    pub(in crate::translator) fn int64_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();
            let a_f64 = self.i64_to_f64(a_boxed);
            let b_f64 = self.i64_to_f64(b_boxed);
            let a_int = self.builder.ins().fcvt_to_sint_sat(types::I64, a_f64);
            let b_int = self.builder.ins().fcvt_to_sint_sat(types::I64, b_f64);
            let result_int = op(self.builder, a_int, b_int);
            let result_f64 = self.builder.ins().fcvt_from_sint(types::F64, result_int);
            let result_boxed = self.f64_to_i64(result_f64);
            self.stack_push(result_boxed);
        }
    }

    /// Native i64 comparison for typed integer opcodes.
    pub(in crate::translator) fn int64_comparison(&mut self, cc: IntCC) {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();
            let a_f64 = self.i64_to_f64(a_boxed);
            let b_f64 = self.i64_to_f64(b_boxed);
            let a_int = self.builder.ins().fcvt_to_sint_sat(types::I64, a_f64);
            let b_int = self.builder.ins().fcvt_to_sint_sat(types::I64, b_f64);
            let cmp = self.builder.ins().icmp(cc, a_int, b_int);
            self.push_cmp_bool_result(cmp);
        }
    }

    /// Binary operation that uses compile-time type info to select optimal path.
    /// If both operands are known to be Option<f64> or f64, uses NaN-sentinel path.
    /// Otherwise falls back to dynamic type checking.
    pub(in crate::translator) fn typed_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value + Copy,
    {
        if self.can_use_nan_sentinel_binary_op() {
            let result_type = self.compute_binary_result_type();
            self.nullable_float64_binary_op(op);
            if self.stack_depth > 0 {
                self.stack_types.insert(self.stack_depth - 1, result_type);
            }
        } else {
            self.numeric_binary_op(op);
            if self.stack_depth > 0 {
                self.stack_types.remove(&(self.stack_depth - 1));
            }
        }
    }

    /// Unary operation that uses compile-time type info to select optimal path.
    pub(in crate::translator) fn typed_unary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value) -> Value,
    {
        if self.can_use_nan_sentinel_unary_op() {
            let result_type = self.peek_stack_type();
            self.nullable_float64_unary_op(op);
            if self.stack_depth > 0 {
                self.stack_types.insert(self.stack_depth - 1, result_type);
            }
        } else if let Some(a_boxed) = self.stack_pop() {
            let a_f64 = self.i64_to_f64(a_boxed);
            let result_f64 = op(self.builder, a_f64);
            let result_boxed = self.f64_to_i64(result_f64);
            self.stack_push(result_boxed);
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

    /// Mixed numeric binary operation where one operand is raw f64 and the other boxed.
    pub(in crate::translator) fn mixed_f64_numeric_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() < 2 {
            return;
        }

        let (top_is_f64, second_is_f64) = self.typed_stack.top_two_f64_flags();
        if top_is_f64 == second_is_f64 {
            return;
        }

        let b_val = if top_is_f64 {
            self.stack_pop_f64().unwrap()
        } else {
            self.stack_pop().unwrap()
        };
        let a_val = if second_is_f64 {
            self.stack_pop_f64().unwrap()
        } else {
            self.stack_pop().unwrap()
        };

        let boxed_val = if top_is_f64 { a_val } else { b_val };
        let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
        let boxed_masked = self.builder.ins().band(boxed_val, nan_base);
        let boxed_is_num = self
            .builder
            .ins()
            .icmp(IntCC::NotEqual, boxed_masked, nan_base);

        let fast_block = self.builder.create_block();
        let slow_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(boxed_is_num, fast_block, &[], slow_block, &[]);

        self.builder.switch_to_block(fast_block);
        self.builder.seal_block(fast_block);
        let boxed_f64 = self.i64_to_f64(boxed_val);
        let a_f64 = if second_is_f64 { a_val } else { boxed_f64 };
        let b_f64 = if top_is_f64 { b_val } else { boxed_f64 };
        let result_f64 = op(self.builder, a_f64, b_f64);
        let fast_result = self.f64_to_i64(result_f64);
        self.builder.ins().jump(merge_block, &[fast_result]);

        self.builder.switch_to_block(slow_block);
        self.builder.seal_block(slow_block);
        let slow_result = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.builder.ins().jump(merge_block, &[slow_result]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let result = self.builder.block_params(merge_block)[0];
        self.stack_push(result);
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

    /// Mixed numeric comparison where one operand is raw f64 and the other boxed.
    pub(in crate::translator) fn mixed_f64_comparison_with_ffi<F>(
        &mut self,
        cc: FloatCC,
        get_ffi: F,
    ) where
        F: FnOnce(&super::types::FFIFuncRefs) -> cranelift::codegen::ir::FuncRef,
    {
        if self.stack_len() < 2 {
            return;
        }

        let (top_is_f64, second_is_f64) = self.typed_stack.top_two_f64_flags();
        if top_is_f64 == second_is_f64 {
            return;
        }

        let b_val = if top_is_f64 {
            self.stack_pop_f64().unwrap()
        } else {
            self.stack_pop().unwrap()
        };
        let a_val = if second_is_f64 {
            self.stack_pop_f64().unwrap()
        } else {
            self.stack_pop().unwrap()
        };
        let boxed_val = if top_is_f64 { a_val } else { b_val };
        let ffi_func = get_ffi(&self.ffi);

        let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
        let boxed_masked = self.builder.ins().band(boxed_val, nan_base);
        let boxed_is_num = self
            .builder
            .ins()
            .icmp(IntCC::NotEqual, boxed_masked, nan_base);

        let fast_block = self.builder.create_block();
        let slow_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(boxed_is_num, fast_block, &[], slow_block, &[]);

        self.builder.switch_to_block(fast_block);
        self.builder.seal_block(fast_block);
        let boxed_f64 = self.i64_to_f64(boxed_val);
        let a_f64 = if second_is_f64 { a_val } else { boxed_f64 };
        let b_f64 = if top_is_f64 { b_val } else { boxed_f64 };
        let cmp = self.builder.ins().fcmp(cc, a_f64, b_f64);
        let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
        let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
        let fast_result = self.builder.ins().select(cmp, true_val, false_val);
        self.builder.ins().jump(merge_block, &[fast_result]);

        self.builder.switch_to_block(slow_block);
        self.builder.seal_block(slow_block);
        let a_boxed = if second_is_f64 {
            self.f64_to_i64(a_val)
        } else {
            a_val
        };
        let b_boxed = if top_is_f64 {
            self.f64_to_i64(b_val)
        } else {
            b_val
        };
        let inst = self.builder.ins().call(ffi_func, &[a_boxed, b_boxed]);
        let slow_result = self.builder.inst_results(inst)[0];
        self.builder.ins().jump(merge_block, &[slow_result]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let result = self.builder.block_params(merge_block)[0];
        self.stack_push(result);
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
