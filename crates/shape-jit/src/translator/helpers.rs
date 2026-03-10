//! Helper methods for BytecodeToIR
//!
//! Contains stack operations, numeric operations, type tracking,
//! and bitcast helpers. Inline array/data operations are in `inline_ops`.

use cranelift::prelude::*;

use crate::context::{STACK_OFFSET, STACK_PTR_OFFSET};
use crate::nan_boxing::*;
use shape_vm::bytecode::{Constant, OpCode, Operand};
use shape_vm::type_tracking::StorageHint;

use super::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Check if we're currently inside an integer-unboxed loop context.
    /// True when either local variables or module bindings are holding raw i64.
    pub(in crate::translator) fn in_unboxed_int_context(&self) -> bool {
        !self.unboxed_int_locals.is_empty() || !self.unboxed_int_module_bindings.is_empty()
    }

    /// Check if we're currently inside a float-unboxed loop context.
    /// True when local variables are holding raw f64.
    pub(in crate::translator) fn in_unboxed_f64_context(&self) -> bool {
        !self.unboxed_f64_locals.is_empty()
    }

    /// Returns true if a NaN-boxed value is a numeric (f64) payload.
    pub(in crate::translator) fn is_boxed_number(&mut self, boxed: Value) -> Value {
        let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
        let masked = self.builder.ins().band(boxed, nan_base);
        self.builder.ins().icmp(IntCC::NotEqual, masked, nan_base)
    }

    /// Get or create the shared deopt block for this function.
    /// The block is emitted at finalize time and returns a negative signal.
    pub(in crate::translator) fn get_or_create_deopt_block(&mut self) -> Block {
        if let Some(block) = self.deopt_block {
            return block;
        }
        let block = self.builder.create_block();
        // Deopt block receives a deopt_id (u32) from each failing guard.
        // Guard sites without a specific deopt point pass u32::MAX.
        self.builder.append_block_param(block, types::I32);
        self.deopt_block = Some(block);
        block
    }

    /// Branch to deopt if `cond` is false, otherwise continue in a new block.
    pub(in crate::translator) fn deopt_if_false(&mut self, cond: Value) {
        self.deopt_if_false_with_id(cond, u32::MAX);
    }

    /// Branch to deopt with a specific `deopt_id` if `cond` is false.
    pub(in crate::translator) fn deopt_if_false_with_id(&mut self, cond: Value, deopt_id: u32) {
        let deopt = self.get_or_create_deopt_block();
        let cont = self.builder.create_block();
        let deopt_id_val = self.builder.ins().iconst(types::I32, deopt_id as i64);
        self.builder
            .ins()
            .brif(cond, cont, &[], deopt, &[deopt_id_val]);
        self.builder.switch_to_block(cont);
        self.builder.seal_block(cont);
    }

    /// Branch to a per-guard spill block if `cond` is false.
    ///
    /// Unlike `deopt_if_false_with_id` (which branches directly to the
    /// shared deopt block), this branches to a dedicated spill block that
    /// stores live state to ctx_buf first. `extra_values` are passed as
    /// block parameters to the spill block (for pre-popped SSA Values).
    pub(in crate::translator) fn deopt_if_false_with_spill(
        &mut self,
        cond: Value,
        spill_block: Block,
        extra_values: &[Value],
    ) {
        let cont = self.builder.create_block();
        self.builder
            .ins()
            .brif(cond, cont, &[], spill_block, extra_values);
        self.builder.switch_to_block(cont);
        self.builder.seal_block(cont);
    }

    /// Propagate a negative callee signal by jumping directly to the function
    /// exit with the negative signal value. Unlike `deopt_if_false`, this does
    /// NOT go through the shared deopt block (which would overwrite ctx_buf[0]
    /// with its own deopt_id). The callee already stored the correct deopt_id
    /// in ctx_buf[0], so we preserve it by bypassing the shared deopt block.
    pub(in crate::translator) fn deopt_if_negative_signal(&mut self, signal: Value) {
        let zero = self.builder.ins().iconst(types::I32, 0);
        let ok = self
            .builder
            .ins()
            .icmp(IntCC::SignedGreaterThanOrEqual, signal, zero);

        // On negative signal: set the deopt_signal_var to the callee's signal
        // (preserving its deopt_id in ctx_buf[0]) and jump to exit.
        if let (Some(deopt_signal_var), Some(exit_block)) = (self.deopt_signal_var, self.exit_block)
        {
            let fail_block = self.builder.create_block();
            let cont = self.builder.create_block();
            self.builder.ins().brif(ok, cont, &[], fail_block, &[]);
            self.builder.switch_to_block(fail_block);
            self.builder.seal_block(fail_block);
            // Propagate the callee's negative signal directly.
            self.builder.def_var(deopt_signal_var, signal);
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.ins().jump(exit_block, &[null_val]);
            self.builder.switch_to_block(cont);
            self.builder.seal_block(cont);
        } else {
            // Fallback for non-optimizing compilation: use shared deopt block.
            self.deopt_if_false(ok);
        }
    }

    /// Binary operation with type guards for NaN-boxed numeric values
    pub(in crate::translator) fn numeric_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();

            // Type check: both must be numbers
            // Correct check: (bits & NAN_BASE) != NAN_BASE
            // This handles negative f64 values correctly (they have sign bit set)
            let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
            let a_masked = self.builder.ins().band(a_boxed, nan_base);
            let b_masked = self.builder.ins().band(b_boxed, nan_base);
            let a_is_num = self.builder.ins().icmp(IntCC::NotEqual, a_masked, nan_base);
            let b_is_num = self.builder.ins().icmp(IntCC::NotEqual, b_masked, nan_base);
            let both_num = self.builder.ins().band(a_is_num, b_is_num);

            // Fast path: both are numbers
            let then_block = self.builder.create_block();
            let else_block = self.builder.create_block();
            let merge_block = self.builder.create_block();

            self.builder.append_block_param(merge_block, types::I64);
            self.builder
                .ins()
                .brif(both_num, then_block, &[], else_block, &[]);

            // Then: numeric operation
            self.builder.switch_to_block(then_block);
            self.builder.seal_block(then_block);
            let a_f64 = self.i64_to_f64(a_boxed);
            let b_f64 = self.i64_to_f64(b_boxed);
            let result_f64 = op(self.builder, a_f64, b_f64);
            let result_boxed = self.f64_to_i64(result_f64);
            self.builder.ins().jump(merge_block, &[result_boxed]);

            // Else: return NaN for non-numeric operations
            self.builder.switch_to_block(else_block);
            self.builder.seal_block(else_block);
            let nan_result = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.ins().jump(merge_block, &[nan_result]);

            // Merge
            self.builder.switch_to_block(merge_block);
            self.builder.seal_block(merge_block);
            let result = self.builder.block_params(merge_block)[0];
            self.stack_push(result);
        }
    }

    /// Optimized binary operation for NaN-nullable floats (Option<f64>)
    ///
    /// When both operands are known to be Option<f64> at compile time,
    /// we can skip the type check entirely and let NaN propagate naturally:
    /// - None is represented as NaN
    /// - IEEE 754: NaN + x = NaN, NaN * x = NaN
    /// - No branches needed - hardware handles null propagation
    ///
    /// Uses typed_stack f64 shadows when available: if the operands were
    /// produced by prior arithmetic or PushConst, their f64 values are cached
    /// and we skip the input bitcasts entirely. The output f64 is also cached
    /// for subsequent operations, enabling zero-bitcast chains like `a*b + c*d`.
    pub(in crate::translator) fn nullable_float64_binary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            // Pop operands — uses cached f64 from typed_stack when available,
            // otherwise bitcasts from i64 (the only cost is a register move)
            let b_f64 = self.stack_pop_f64().unwrap();
            let a_f64 = self.stack_pop_f64().unwrap();

            // Perform operation - NaN inputs produce NaN output
            let result_f64 = op(self.builder, a_f64, b_f64);

            // Push boxed result to legacy stack, cache f64 in typed_stack
            let result_boxed = self.f64_to_i64(result_f64);
            self.stack_push(result_boxed);
            self.typed_stack
                .replace_top(super::storage::TypedValue::f64(result_f64));
        }
    }

    /// Optimized unary operation for NaN-nullable floats (Option<f64>)
    ///
    /// Uses typed_stack f64 shadow when available to skip the input bitcast.
    pub(in crate::translator) fn nullable_float64_unary_op<F>(&mut self, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value) -> Value,
    {
        if self.stack_len() >= 1 {
            let a_f64 = self.stack_pop_f64().unwrap();

            // Perform operation - NaN input produces NaN output
            let result_f64 = op(self.builder, a_f64);

            // Push boxed result to legacy stack, cache f64 in typed_stack
            let result_boxed = self.f64_to_i64(result_f64);
            self.stack_push(result_boxed);
            self.typed_stack
                .replace_top(super::storage::TypedValue::f64(result_f64));
        }
    }

    /// Check if a boxed value is NaN (represents None for Option<f64>)
    ///
    /// For Option<f64> with NaN sentinel, this is the null check.
    /// Uses fcmp with Unordered - NaN comparisons are always unordered.
    #[allow(dead_code)] // Reserved for typed compilation
    pub(in crate::translator) fn is_nan_sentinel(&mut self, boxed_val: Value) -> Value {
        let f64_val = self.i64_to_f64(boxed_val);
        // NaN != NaN in IEEE 754, so fcmp(Unordered, x, x) is true iff x is NaN
        self.builder
            .ins()
            .fcmp(FloatCC::Unordered, f64_val, f64_val)
    }

    /// Create a NaN value (represents None for Option<f64>)
    #[allow(dead_code)] // Reserved for typed compilation
    pub(in crate::translator) fn create_nan_sentinel(&mut self) -> Value {
        let nan_f64 = self.builder.ins().f64const(f64::NAN);
        self.f64_to_i64(nan_f64)
    }

    pub(in crate::translator) fn comparison_op(&mut self, cc: FloatCC) {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();

            // Type check: both must be numbers
            // Correct check: (bits & NAN_BASE) != NAN_BASE
            // This handles negative f64 values correctly (they have sign bit set)
            let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
            let a_masked = self.builder.ins().band(a_boxed, nan_base);
            let b_masked = self.builder.ins().band(b_boxed, nan_base);
            let a_is_num = self.builder.ins().icmp(IntCC::NotEqual, a_masked, nan_base);
            let b_is_num = self.builder.ins().icmp(IntCC::NotEqual, b_masked, nan_base);
            let both_num = self.builder.ins().band(a_is_num, b_is_num);

            let then_block = self.builder.create_block();
            let else_block = self.builder.create_block();
            let merge_block = self.builder.create_block();

            self.builder.append_block_param(merge_block, types::I64);
            self.builder
                .ins()
                .brif(both_num, then_block, &[], else_block, &[]);

            // Then: numeric comparison
            self.builder.switch_to_block(then_block);
            self.builder.seal_block(then_block);
            let a_f64 = self.i64_to_f64(a_boxed);
            let b_f64 = self.i64_to_f64(b_boxed);
            let cmp = self.builder.ins().fcmp(cc, a_f64, b_f64);
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            let result_bool = self.builder.ins().select(cmp, true_val, false_val);
            self.builder.ins().jump(merge_block, &[result_bool]);

            // Else: non-numeric comparison
            // For Eq: return true if values are identical (handles null==null, true==true, etc.)
            // For other comparisons: return false
            self.builder.switch_to_block(else_block);
            self.builder.seal_block(else_block);
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            let non_num_result = if cc == FloatCC::Equal {
                // For equality, compare the raw bits
                let is_equal = self.builder.ins().icmp(IntCC::Equal, a_boxed, b_boxed);
                self.builder.ins().select(is_equal, true_val, false_val)
            } else if cc == FloatCC::NotEqual {
                // For inequality, compare the raw bits
                let is_not_equal = self.builder.ins().icmp(IntCC::NotEqual, a_boxed, b_boxed);
                self.builder.ins().select(is_not_equal, true_val, false_val)
            } else {
                // Other comparisons on non-numerics return false
                false_val
            };
            self.builder.ins().jump(merge_block, &[non_num_result]);

            // Merge
            self.builder.switch_to_block(merge_block);
            self.builder.seal_block(merge_block);
            let result = self.builder.block_params(merge_block)[0];
            self.stack_push(result);
        }
    }

    /// Binary operation with runtime type check: numeric fast path, generic FFI fallback.
    ///
    /// When `is_add` is true, the fallback calls `generic_add` (handles string concat,
    /// Time+Duration, etc.). Otherwise returns TAG_NULL for non-numeric operands.
    pub(in crate::translator) fn generic_binary_op_with_fallback<F>(&mut self, op: F, is_add: bool)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();

            let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
            let a_masked = self.builder.ins().band(a_boxed, nan_base);
            let b_masked = self.builder.ins().band(b_boxed, nan_base);
            let a_is_num = self.builder.ins().icmp(IntCC::NotEqual, a_masked, nan_base);
            let b_is_num = self.builder.ins().icmp(IntCC::NotEqual, b_masked, nan_base);
            let both_num = self.builder.ins().band(a_is_num, b_is_num);

            let then_block = self.builder.create_block();
            let else_block = self.builder.create_block();
            let merge_block = self.builder.create_block();

            self.builder.append_block_param(merge_block, types::I64);
            self.builder
                .ins()
                .brif(both_num, then_block, &[], else_block, &[]);

            // Then: numeric fast path
            self.builder.switch_to_block(then_block);
            self.builder.seal_block(then_block);
            let a_f64 = self.i64_to_f64(a_boxed);
            let b_f64 = self.i64_to_f64(b_boxed);
            let result_f64 = op(self.builder, a_f64, b_f64);
            let result_boxed = self.f64_to_i64(result_f64);
            self.builder.ins().jump(merge_block, &[result_boxed]);

            // Else: non-numeric — call generic FFI
            self.builder.switch_to_block(else_block);
            self.builder.seal_block(else_block);
            let ffi_result = if is_add {
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_add, &[a_boxed, b_boxed]);
                self.builder.inst_results(inst)[0]
            } else {
                self.builder.ins().iconst(types::I64, TAG_NULL as i64)
            };
            self.builder.ins().jump(merge_block, &[ffi_result]);

            // Merge
            self.builder.switch_to_block(merge_block);
            self.builder.seal_block(merge_block);
            let result = self.builder.block_params(merge_block)[0];
            self.stack_push(result);
        }
    }

    /// Comparison with runtime type check: numeric fast path, generic FFI fallback.
    ///
    /// For Equal/NotEqual, the fallback calls `generic_eq`/`generic_neq` which
    /// compares string contents, booleans by tag, etc.
    pub(in crate::translator) fn generic_comparison_with_fallback(&mut self, cc: FloatCC) {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();

            let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
            let a_masked = self.builder.ins().band(a_boxed, nan_base);
            let b_masked = self.builder.ins().band(b_boxed, nan_base);
            let a_is_num = self.builder.ins().icmp(IntCC::NotEqual, a_masked, nan_base);
            let b_is_num = self.builder.ins().icmp(IntCC::NotEqual, b_masked, nan_base);
            let both_num = self.builder.ins().band(a_is_num, b_is_num);

            let then_block = self.builder.create_block();
            let else_block = self.builder.create_block();
            let merge_block = self.builder.create_block();

            self.builder.append_block_param(merge_block, types::I64);
            self.builder
                .ins()
                .brif(both_num, then_block, &[], else_block, &[]);

            // Then: numeric comparison
            self.builder.switch_to_block(then_block);
            self.builder.seal_block(then_block);
            let a_f64 = self.i64_to_f64(a_boxed);
            let b_f64 = self.i64_to_f64(b_boxed);
            let cmp = self.builder.ins().fcmp(cc, a_f64, b_f64);
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            let result_bool = self.builder.ins().select(cmp, true_val, false_val);
            self.builder.ins().jump(merge_block, &[result_bool]);

            // Else: non-numeric — call generic FFI for eq/neq, raw bits for others
            self.builder.switch_to_block(else_block);
            self.builder.seal_block(else_block);
            let non_num_result = if cc == FloatCC::Equal {
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_eq, &[a_boxed, b_boxed]);
                self.builder.inst_results(inst)[0]
            } else if cc == FloatCC::NotEqual {
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_neq, &[a_boxed, b_boxed]);
                self.builder.inst_results(inst)[0]
            } else {
                // Other comparisons on non-numerics return false
                self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64)
            };
            self.builder.ins().jump(merge_block, &[non_num_result]);

            // Merge
            self.builder.switch_to_block(merge_block);
            self.builder.seal_block(merge_block);
            let result = self.builder.block_params(merge_block)[0];
            self.stack_push(result);
        }
    }

    /// Get or create a Cranelift Variable for a stack position
    /// This enables proper SSA PHI insertion at control flow merge points
    pub(in crate::translator) fn get_or_create_stack_var(&mut self, depth: usize) -> Variable {
        if let Some(var) = self.stack_vars.get(&depth) {
            return *var;
        }

        // Use next_var starting from a high offset to avoid collision with locals
        let var = Variable::new(self.next_var + 1000 + depth);
        self.next_var += 1;
        self.builder.declare_var(var, types::I64); // NaN-boxed values

        // Initialize to TAG_NULL
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.builder.def_var(var, null_val);

        self.stack_vars.insert(depth, var);
        var
    }

    /// Push a value onto the operand stack using Variables for SSA correctness.
    /// Also pushes a boxed entry to typed_stack for sync. Callers with type info
    /// should follow with `typed_stack.replace_top()` to upgrade to the real type.
    pub(in crate::translator) fn stack_push(&mut self, value: Value) {
        let var = self.get_or_create_stack_var(self.stack_depth);
        self.builder.def_var(var, value);
        self.stack_depth += 1;
        self.typed_stack
            .push(super::storage::TypedValue::boxed(value));
    }

    /// Pop a value from the operand stack using Variables for SSA correctness.
    /// Also pops from typed_stack to keep stacks in sync.
    ///
    /// For raw I64 entries (from integer unboxing), returns the cached value
    /// directly to skip unnecessary `use_var` calls. F64 and NanBoxed entries
    /// fall through to `use_var` to reuse the already-computed stack value
    /// (avoids creating duplicate bitcast instructions).
    pub(in crate::translator) fn stack_pop(&mut self) -> Option<Value> {
        if self.stack_depth == 0 {
            return None;
        }
        self.stack_depth -= 1;
        // For raw I64 entries, use the cached value to skip use_var
        if let Some(tv) = self.typed_stack.pop() {
            if tv.repr == super::storage::CraneliftRepr::I64 {
                return Some(tv.value);
            }
        }
        let var = self.get_or_create_stack_var(self.stack_depth);
        Some(self.builder.use_var(var))
    }

    /// Pop a value, ensuring it is NaN-boxed (not raw i64 from integer unboxing).
    ///
    /// Use this instead of `stack_pop()` when the value will be stored to memory
    /// (arrays, objects, ctx.stack) or passed to FFI, where NaN-boxed format is
    /// required. Raw i64 values (from loop integer unboxing) are converted via
    /// `fcvt_from_sint` → `bitcast` to produce correct NaN-boxed representation.
    pub(in crate::translator) fn stack_pop_boxed(&mut self) -> Option<Value> {
        if self.stack_depth == 0 {
            return None;
        }
        let hint = self
            .stack_types
            .get(&(self.stack_depth - 1))
            .copied()
            .unwrap_or(StorageHint::Int64);
        let is_raw_i64 = self
            .typed_stack
            .peek()
            .map(|tv| tv.repr == super::storage::CraneliftRepr::I64)
            .unwrap_or(false);
        let val = self.stack_pop()?;
        if is_raw_i64 {
            Some(self.raw_i64_to_boxed_for_hint(val, hint))
        } else {
            Some(val)
        }
    }

    /// Pop a value, returning the f64 directly if available from typed_stack.
    /// When the typed_stack has a cached F64 entry (from PushConst, prior arithmetic,
    /// or a numeric LoadLocal), returns it without bitcasting — saving ~1 cycle.
    /// Otherwise bitcasts the i64 value to f64 as usual.
    ///
    /// Avoids `use_var` when typed_stack provides the f64 directly. This
    /// eliminates unnecessary SSA uses that prevent Cranelift DCE from
    /// removing dead bitcast chains in unboxed f64 loops.
    pub(in crate::translator) fn stack_pop_f64(&mut self) -> Option<Value> {
        if self.stack_depth == 0 {
            return None;
        }
        self.stack_depth -= 1;
        // Check typed_stack first — skip use_var to allow DCE of dead defs
        if let Some(tv) = self.typed_stack.pop() {
            if tv.repr == super::storage::CraneliftRepr::F64 {
                return Some(tv.value);
            }
        }
        let var = self.get_or_create_stack_var(self.stack_depth);
        let i64_val = self.builder.use_var(var);
        Some(self.i64_to_f64(i64_val))
    }

    /// Peek at the top value without popping
    pub(in crate::translator) fn stack_peek(&mut self) -> Option<Value> {
        if self.stack_depth == 0 {
            return None;
        }
        let var = self.get_or_create_stack_var(self.stack_depth - 1);
        Some(self.builder.use_var(var))
    }

    /// Peek at a value at depth `n` from the top of the stack (0 = top).
    pub(in crate::translator) fn stack_peek_at(&mut self, n: usize) -> Option<Value> {
        if n >= self.stack_depth {
            return None;
        }
        let var = self.get_or_create_stack_var(self.stack_depth - 1 - n);
        Some(self.builder.use_var(var))
    }

    /// Get current stack depth
    pub(in crate::translator) fn stack_len(&self) -> usize {
        self.stack_depth
    }

    // ========================================================================
    // Type Tracking Methods
    // ========================================================================

    /// Push a value with known type onto the stack
    pub(in crate::translator) fn stack_push_typed(&mut self, value: Value, hint: StorageHint) {
        self.stack_types.insert(self.stack_depth, hint);
        self.stack_push(value);
    }

    /// Pop a value and its type from the stack
    #[allow(dead_code)]
    pub(in crate::translator) fn stack_pop_typed(&mut self) -> Option<(Value, StorageHint)> {
        if self.stack_depth == 0 {
            return None;
        }
        let hint = self
            .stack_types
            .remove(&(self.stack_depth - 1))
            .unwrap_or(StorageHint::Unknown);
        let value = self.stack_pop()?;
        Some((value, hint))
    }

    /// Get the type hint for the top of stack (without popping)
    pub(in crate::translator) fn peek_stack_type(&self) -> StorageHint {
        if self.stack_depth == 0 {
            return StorageHint::Unknown;
        }
        self.stack_types
            .get(&(self.stack_depth - 1))
            .copied()
            .unwrap_or(StorageHint::Unknown)
    }

    /// Get the type hint for stack position (0 = top, 1 = second from top, etc.)
    #[allow(dead_code)]
    pub(in crate::translator) fn get_stack_type_at(&self, offset: usize) -> StorageHint {
        if offset >= self.stack_depth {
            return StorageHint::Unknown;
        }
        let idx = self.stack_depth - 1 - offset;
        self.stack_types
            .get(&idx)
            .copied()
            .unwrap_or(StorageHint::Unknown)
    }

    /// Check if both top two stack slots are known numeric types.
    /// This enables the NaN-sentinel optimization for binary operations.
    /// Accepts Float64, NullableFloat64, Int64, and NullableInt64 since
    /// all are stored as f64 in NaN-boxing (ints fit exactly in f64).
    pub(in crate::translator) fn can_use_nan_sentinel_binary_op(&self) -> bool {
        if self.stack_depth < 2 {
            return false;
        }
        let a_hint = self
            .stack_types
            .get(&(self.stack_depth - 1))
            .copied()
            .unwrap_or(StorageHint::Unknown);
        let b_hint = self
            .stack_types
            .get(&(self.stack_depth - 2))
            .copied()
            .unwrap_or(StorageHint::Unknown);

        // Use NaN-sentinel op if both are known numeric types
        fn is_numeric(h: StorageHint) -> bool {
            h.is_numeric_family()
        }
        is_numeric(a_hint) && is_numeric(b_hint)
    }

    /// Check if either of the top two operands is known to be a non-numeric type
    /// (String, Bool). When true, polymorphic operations like add/sub MUST use
    /// FFI dispatch because the inline numeric path would give wrong results
    /// (e.g., String concat, Time + Duration need the generic FFI path).
    /// Returns false if both operands are Boxed (unknown) — in that case the
    /// inline runtime type check is preferred over FFI.
    pub(in crate::translator) fn either_operand_non_numeric(&self) -> bool {
        if self.stack_depth < 2 {
            return false;
        }
        let a_hint = self
            .stack_types
            .get(&(self.stack_depth - 1))
            .copied()
            .unwrap_or(StorageHint::Unknown);
        let b_hint = self
            .stack_types
            .get(&(self.stack_depth - 2))
            .copied()
            .unwrap_or(StorageHint::Unknown);

        fn is_known_non_numeric(h: StorageHint) -> bool {
            matches!(h, StorageHint::String | StorageHint::Bool)
        }
        is_known_non_numeric(a_hint) || is_known_non_numeric(b_hint)
    }

    /// Check if top of stack is a known numeric type (for unary ops)
    pub(in crate::translator) fn can_use_nan_sentinel_unary_op(&self) -> bool {
        if self.stack_depth < 1 {
            return false;
        }
        let hint = self
            .stack_types
            .get(&(self.stack_depth - 1))
            .copied()
            .unwrap_or(StorageHint::Unknown);
        hint.is_numeric_family()
    }

    pub(in crate::translator) fn integer_clif_type_and_signed(
        hint: StorageHint,
    ) -> Option<(Type, bool)> {
        let signed = hint.is_signed_integer()?;
        let ty = match hint.integer_bit_width()? {
            8 => types::I8,
            16 => types::I16,
            32 => types::I32,
            64 => types::I64,
            _ => return None,
        };
        Some((ty, signed))
    }

    pub(in crate::translator) fn normalize_i64_to_hint(
        &mut self,
        value: Value,
        hint: StorageHint,
    ) -> Value {
        let Some((ty, signed)) = Self::integer_clif_type_and_signed(hint) else {
            return value;
        };
        if ty == types::I64 {
            return value;
        }
        let narrowed = self.builder.ins().ireduce(ty, value);
        if signed {
            self.builder.ins().sextend(types::I64, narrowed)
        } else {
            self.builder.ins().uextend(types::I64, narrowed)
        }
    }

    pub(in crate::translator) fn boxed_to_i64_for_hint(
        &mut self,
        boxed_value: Value,
        hint: StorageHint,
    ) -> Value {
        let f64_val = self.i64_to_f64(boxed_value);
        let signed = hint.is_signed_integer().unwrap_or(true);
        let raw = if signed {
            self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val)
        } else {
            self.builder.ins().fcvt_to_uint(types::I64, f64_val)
        };
        self.normalize_i64_to_hint(raw, hint)
    }

    pub(in crate::translator) fn raw_i64_to_boxed_for_hint(
        &mut self,
        raw_value: Value,
        hint: StorageHint,
    ) -> Value {
        let normalized = self.normalize_i64_to_hint(raw_value, hint);
        let signed = hint.is_signed_integer().unwrap_or(true);
        let f64_val = if signed {
            self.builder.ins().fcvt_from_sint(types::F64, normalized)
        } else {
            self.builder.ins().fcvt_from_uint(types::F64, normalized)
        };
        self.f64_to_i64(f64_val)
    }

    pub(in crate::translator) fn replace_stack_top_value(&mut self, value: Value) {
        if self.stack_depth == 0 {
            return;
        }
        let var = self.get_or_create_stack_var(self.stack_depth - 1);
        self.builder.def_var(var, value);
    }

    pub(in crate::translator) fn combine_integer_hints(
        &self,
        a: StorageHint,
        b: StorageHint,
    ) -> Option<StorageHint> {
        if a.is_float_family() || b.is_float_family() {
            return None;
        }
        if a.is_integer_family() && b.is_integer_family() {
            return StorageHint::combine_integer_hints(a, b);
        }
        if a.is_integer_family() {
            return Some(a.non_nullable());
        }
        if b.is_integer_family() {
            return Some(b.non_nullable());
        }
        None
    }

    pub(in crate::translator) fn top_two_integer_result_hint(&self) -> Option<StorageHint> {
        if self.stack_depth < 2 {
            return None;
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
        self.combine_integer_hints(a, b)
    }

    pub(in crate::translator) fn intcc_for_hint(cc: IntCC, hint: StorageHint) -> IntCC {
        if hint.is_signed_integer() != Some(false) {
            return cc;
        }
        match cc {
            IntCC::SignedGreaterThan => IntCC::UnsignedGreaterThan,
            IntCC::SignedGreaterThanOrEqual => IntCC::UnsignedGreaterThanOrEqual,
            IntCC::SignedLessThan => IntCC::UnsignedLessThan,
            IntCC::SignedLessThanOrEqual => IntCC::UnsignedLessThanOrEqual,
            other => other,
        }
    }

    /// Convert I64 (NaN-boxed) to F64 via zero-cost bitcast
    ///
    /// Uses Cranelift's `bitcast` instruction which just reinterprets
    /// the register bits as a different type (no memory operations).
    pub(in crate::translator) fn i64_to_f64(&mut self, i64_val: Value) -> Value {
        self.builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), i64_val)
    }

    /// Convert F64 to I64 (NaN-boxed) via zero-cost bitcast
    ///
    /// Uses Cranelift's `bitcast` instruction which just reinterprets
    /// the register bits as a different type (no memory operations).
    pub(in crate::translator) fn f64_to_i64(&mut self, f64_val: Value) -> Value {
        self.builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), f64_val)
    }

    /// Helper to check if a value is truthy (non-zero number or true boolean)
    pub(in crate::translator) fn is_truthy(&mut self, value: Value) -> Value {
        // Check if value is:
        // 1. A number != 0
        // 2. Boolean true (TAG_BOOL_TRUE)
        // Correct check: (bits & NAN_BASE) != NAN_BASE handles negative f64 values
        let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
        let masked = self.builder.ins().band(value, nan_base);
        let is_num = self.builder.ins().icmp(IntCC::NotEqual, masked, nan_base);

        let then_block = self.builder.create_block();
        let else_block = self.builder.create_block();
        let merge_block = self.builder.create_block();

        self.builder.append_block_param(merge_block, types::I8);
        self.builder
            .ins()
            .brif(is_num, then_block, &[], else_block, &[]);

        // If number: check if != 0
        self.builder.switch_to_block(then_block);
        self.builder.seal_block(then_block);
        let as_f64 = self.i64_to_f64(value);
        let zero = self.builder.ins().f64const(0.0);
        let num_is_true = self.builder.ins().fcmp(FloatCC::NotEqual, as_f64, zero);
        self.builder.ins().jump(merge_block, &[num_is_true]);

        // If not number: check if TAG_BOOL_TRUE
        self.builder.switch_to_block(else_block);
        self.builder.seal_block(else_block);
        let true_tag = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
        let is_true_tag = self.builder.ins().icmp(IntCC::Equal, value, true_tag);
        self.builder.ins().jump(merge_block, &[is_true_tag]);

        // Merge
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        self.builder.block_params(merge_block)[0]
    }

    /// Materialize N values from the operand stack to ctx.stack for FFI calls.
    /// Values are popped from the stack and stored to ctx.stack in the correct
    /// order (first popped value goes to highest index, maintaining stack semantics).
    /// Also updates ctx.stack_ptr in the generated code.
    pub(in crate::translator) fn materialize_to_stack(&mut self, count: usize) {
        if count == 0 {
            return;
        }

        // Pop values from stack (reverse order - last pushed is first popped)
        let mut values: Vec<Value> = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(val) = self.stack_pop() {
                values.push(val);
            }
        }

        // Store to ctx.stack in correct order (reverse values so first in stack order)
        // values[0] was the top of stack (last element), should go to highest index
        // values[count-1] was bottom of the N elements, should go to lowest index
        for (i, val) in values.iter().rev().enumerate() {
            let offset = STACK_OFFSET + (self.compile_time_sp + i) as i32 * 8;
            self.builder
                .ins()
                .store(MemFlags::trusted(), *val, self.ctx_ptr, offset);
        }

        // Update compile-time stack pointer tracking
        self.compile_time_sp += count;

        // Generate code to update ctx.stack_ptr at runtime
        let new_sp = self
            .builder
            .ins()
            .iconst(types::I64, self.compile_time_sp as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_sp, self.ctx_ptr, STACK_PTR_OFFSET);
    }

    /// Update compile-time stack pointer after FFI call consumes values and pushes result.
    /// This doesn't generate any code - just updates our tracking.
    pub(in crate::translator) fn update_sp_after_ffi(&mut self, consumed: usize, produced: usize) {
        self.compile_time_sp -= consumed;
        self.compile_time_sp += produced;
    }

    /// Look back at the previous instruction to extract arg_count for Call opcodes.
    /// The bytecode pattern is: push args -> PushConst(arg_count) -> Call
    /// Returns the arg_count if it can be determined, or 0 if not.
    pub(in crate::translator) fn get_arg_count_from_prev_instruction(
        &self,
        current_idx: usize,
    ) -> usize {
        if current_idx == 0 {
            return 0;
        }
        let prev_idx = current_idx - 1;
        if prev_idx >= self.program.instructions.len() {
            return 0;
        }
        let prev_instr = &self.program.instructions[prev_idx];
        if prev_instr.opcode == OpCode::PushConst {
            if let Some(Operand::Const(const_idx)) = &prev_instr.operand {
                if let Some(Constant::Number(n)) = self.program.constants.get(*const_idx as usize) {
                    return *n as usize;
                }
            }
        }
        0
    }
}
