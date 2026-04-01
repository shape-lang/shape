//! HOF (Higher-Order Function) method inlining for the JIT.
//!
//! Emits native Cranelift loops for array HOF methods (map, filter, reduce, find,
//! some, every, forEach, findIndex) instead of routing through FFI.
//!
//! For each HOF, the pattern is:
//! 1. Pop overhead (arg_count, method_name) and args from SSA stack
//! 2. Type-check receiver is array
//! 3. Extract data_ptr and length from JitArray
//! 4. Emit Cranelift for-loop with callback invocation per element
//! 5. Collect results (method-specific)
//! 6. Fall back to FFI for non-array receivers
//!
//! Performance optimizations (A5):
//! - Closure capture pre-loading: extracts captures_ptr once before the loop
//! - Numeric element fast-path: skips NaN-box tag checks for f64 elements
//! - Reduce strength reduction: unboxed f64 accumulator for simple arithmetic reduces

use cranelift::prelude::*;
use shape_value::MethodId;
use shape_vm::bytecode::OpCode;

use crate::context::{STACK_OFFSET, STACK_PTR_OFFSET};
use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;

/// Pre-extracted closure data for avoiding per-iteration unboxing.
/// If the callback is a closure, we extract function_id, captures_ptr, and
/// captures_count once before the loop and pass them through the loop body.
struct PreloadedClosure {
    /// The function_id extracted from the JITClosure.
    function_id: Value,
    /// Pointer to the captures array (pre-loaded from JITClosure.captures_ptr).
    captures_ptr: Value,
    /// Number of captures (pre-loaded from JITClosure.captures_count).
    captures_count: Value,
}

/// Detected simple binary reduce pattern for strength reduction.
#[derive(Clone, Copy)]
enum ReduceOp {
    Add,
    Mul,
}

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Try to inline a HOF method call.
    ///
    /// Called from `compile_call_method()` when the method_id matches a HOF method
    /// and the optimization plan identifies this site as eligible.
    ///
    /// Returns `Some(())` if inlined, `None` to fall back to FFI.
    pub(crate) fn try_inline_hof_method(
        &mut self,
        method_id: u16,
        arg_count: usize,
        idx: usize,
    ) -> Result<Option<()>, String> {
        // Check optimizer plan for this site
        let site = match self.optimization_plan.hof_inline.sites.get(&idx) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };

        // Need at least: receiver + args on stack (TypedMethodCall: no overhead values)
        let needed = arg_count + 1;
        if self.stack_len() < needed {
            return Ok(None);
        }

        // Pop args (callback is first, initial value for reduce is second)
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if let Some(arg) = self.stack_pop() {
                args.push(arg);
            }
        }
        if args.len() != arg_count {
            return Ok(None);
        }
        args.reverse();

        // Pop receiver
        let receiver = match self.stack_pop() {
            Some(v) => v,
            None => return Ok(None),
        };

        let callback = args[0];

        // Type-check: is receiver an array?
        let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

        let inline_block = self.builder.create_block();
        let ffi_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);

        self.builder
            .ins()
            .brif(is_array, inline_block, &[], ffi_block, &[]);

        // === Inline path ===
        self.builder.switch_to_block(inline_block);
        self.builder.seal_block(inline_block);

        let result = match method_id {
            id if id == MethodId::MAP.0 => self.emit_hof_map(receiver, callback, &site)?,
            id if id == MethodId::FILTER.0 => self.emit_hof_filter(receiver, callback, &site)?,
            id if id == MethodId::REDUCE.0 => {
                let initial = if args.len() > 1 {
                    args[1]
                } else {
                    self.builder
                        .ins()
                        .iconst(types::I64, box_number(0.0) as i64)
                };
                self.emit_hof_reduce(receiver, callback, initial, &site)?
            }
            id if id == MethodId::FIND.0 => self.emit_hof_find(receiver, callback, &site)?,
            id if id == MethodId::FIND_INDEX.0 => {
                self.emit_hof_find_index(receiver, callback, &site)?
            }
            id if id == MethodId::SOME.0 => self.emit_hof_some(receiver, callback, &site)?,
            id if id == MethodId::EVERY.0 => self.emit_hof_every(receiver, callback, &site)?,
            id if id == MethodId::FOR_EACH.0 => self.emit_hof_foreach(receiver, callback, &site)?,
            _ => {
                // Unknown HOF method — shouldn't happen, but fall through to FFI
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &args[..]);
                ffi_result
            }
        };

        self.builder.ins().jump(merge_block, &[result]);

        // === FFI fallback ===
        self.builder.switch_to_block(ffi_block);
        self.builder.seal_block(ffi_block);
        let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &args[..]);
        self.builder.ins().jump(merge_block, &[ffi_result]);

        // === Merge ===
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let result = self.builder.block_params(merge_block)[0];
        self.stack_push(result);

        self.reload_referenced_locals();
        Ok(Some(()))
    }

    // =========================================================================
    // Callback invocation helper
    // =========================================================================

    /// Invoke a callback function with (element, index) arguments via FFI.
    /// Pushes [callee, element, index, arg_count=2] onto ctx.stack, calls jit_call_value.
    fn emit_callback_call(&mut self, callback: Value, element: Value, index: Value) -> Value {
        let base_sp = self.compile_time_sp.depth();

        // Push callee
        self.builder.ins().store(
            MemFlags::trusted(),
            callback,
            self.ctx_ptr,
            STACK_OFFSET + (base_sp as i32) * 8,
        );
        // Push element (arg0)
        self.builder.ins().store(
            MemFlags::trusted(),
            element,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 1) as i32) * 8,
        );
        // Push index (arg1)
        self.builder.ins().store(
            MemFlags::trusted(),
            index,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 2) as i32) * 8,
        );
        // Push arg_count = 2
        let arg_count_bits = self
            .builder
            .ins()
            .iconst(types::I64, box_number(2.0) as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            arg_count_bits,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 3) as i32) * 8,
        );

        // Update stack_ptr
        let new_sp = self.builder.ins().iconst(types::I64, (base_sp + 4) as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_sp, self.ctx_ptr, STACK_PTR_OFFSET);

        // Call jit_call_value
        let inst = self
            .builder
            .ins()
            .call(self.ffi.call_value, &[self.ctx_ptr]);
        let result = self.builder.inst_results(inst)[0];

        // Restore stack_ptr
        let restore_sp = self.builder.ins().iconst(types::I64, base_sp as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            restore_sp,
            self.ctx_ptr,
            STACK_PTR_OFFSET,
        );

        result
    }

    /// Invoke a callback with 3 args: (accumulator, element, index) for reduce.
    fn emit_callback_call_3(
        &mut self,
        callback: Value,
        acc: Value,
        element: Value,
        index: Value,
    ) -> Value {
        let base_sp = self.compile_time_sp.depth();

        // Push callee
        self.builder.ins().store(
            MemFlags::trusted(),
            callback,
            self.ctx_ptr,
            STACK_OFFSET + (base_sp as i32) * 8,
        );
        // Push acc (arg0)
        self.builder.ins().store(
            MemFlags::trusted(),
            acc,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 1) as i32) * 8,
        );
        // Push element (arg1)
        self.builder.ins().store(
            MemFlags::trusted(),
            element,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 2) as i32) * 8,
        );
        // Push index (arg2)
        self.builder.ins().store(
            MemFlags::trusted(),
            index,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 3) as i32) * 8,
        );
        // Push arg_count = 3
        let arg_count_bits = self
            .builder
            .ins()
            .iconst(types::I64, box_number(3.0) as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            arg_count_bits,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 4) as i32) * 8,
        );

        // Update stack_ptr
        let new_sp = self.builder.ins().iconst(types::I64, (base_sp + 5) as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_sp, self.ctx_ptr, STACK_PTR_OFFSET);

        // Call jit_call_value
        let inst = self
            .builder
            .ins()
            .call(self.ffi.call_value, &[self.ctx_ptr]);
        let result = self.builder.inst_results(inst)[0];

        // Restore stack_ptr
        let restore_sp = self.builder.ins().iconst(types::I64, base_sp as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            restore_sp,
            self.ctx_ptr,
            STACK_PTR_OFFSET,
        );

        result
    }

    // =========================================================================
    // Truthy check helper
    // =========================================================================

    /// Check if a NaN-boxed value is truthy (not null, not false, not 0.0).
    /// Returns an i8 (boolean) Cranelift value.
    fn emit_is_truthy(&mut self, val: Value) -> Value {
        // Falsy values: TAG_NULL, TAG_BOOL_FALSE, box_number(0.0)
        let is_null = self
            .builder
            .ins()
            .icmp_imm(IntCC::Equal, val, TAG_NULL as i64);
        let is_false = self
            .builder
            .ins()
            .icmp_imm(IntCC::Equal, val, TAG_BOOL_FALSE as i64);
        let is_zero = self
            .builder
            .ins()
            .icmp_imm(IntCC::Equal, val, box_number(0.0) as i64);
        let falsy1 = self.builder.ins().bor(is_null, is_false);
        let falsy = self.builder.ins().bor(falsy1, is_zero);
        // Negate: truthy = !falsy
        let one = self.builder.ins().iconst(types::I8, 1);
        self.builder.ins().bxor(falsy, one)
    }

    // =========================================================================
    // A5 Optimization: Closure capture pre-loading
    // =========================================================================

    /// Pre-extract closure data (function_id, captures_ptr, captures_count) from
    /// a callback value before entering a HOF loop. This avoids the overhead of
    /// jit_unbox::<JITClosure>() on every iteration inside jit_call_value.
    ///
    /// Returns `Some(PreloadedClosure)` if the callback is detected as a closure
    /// at the IR level. The check is a runtime branch: if not a closure, the
    /// `PreloadedClosure` values are zeroed and the loop body will use the
    /// generic `emit_callback_call` path.
    fn try_preload_closure(&mut self, callback: Value) -> Option<PreloadedClosure> {
        // Check if callback is a heap closure: TAG_HEAP with kind == HK_CLOSURE
        let is_closure = self.emit_is_heap_kind(callback, HK_CLOSURE);

        // Create blocks for closure / non-closure paths
        let closure_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        // Block params: function_id, captures_ptr, captures_count
        self.builder.append_block_param(merge_block, types::I64);
        self.builder.append_block_param(merge_block, types::I64);
        self.builder.append_block_param(merge_block, types::I64);

        let not_closure_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_closure, closure_block, &[], not_closure_block, &[]);

        // === Closure path: extract fields from JITClosure ===
        self.builder.switch_to_block(closure_block);
        self.builder.seal_block(closure_block);

        // JITClosure is behind JitAlloc prefix (8 bytes).
        // Layout of JITClosure (#[repr(C)]):
        //   offset 0: function_id (u16)
        //   offset 2: captures_count (u16)
        //   offset 8: captures_ptr (*const u64) — aligned to 8
        let ptr_mask = self.builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
        let alloc_ptr = self.builder.ins().band(callback, ptr_mask);
        // Data starts at offset 8 (unified header size)
        let data_ptr = self
            .builder
            .ins()
            .iadd_imm(alloc_ptr, JIT_ALLOC_DATA_OFFSET as i64);

        // function_id: u16 at offset 0
        let fn_id_u16 = self
            .builder
            .ins()
            .load(types::I16, MemFlags::trusted(), data_ptr, 0);
        let fn_id = self.builder.ins().uextend(types::I64, fn_id_u16);

        // captures_count: u16 at offset 2
        let cap_count_u16 = self
            .builder
            .ins()
            .load(types::I16, MemFlags::trusted(), data_ptr, 2);
        let cap_count = self.builder.ins().uextend(types::I64, cap_count_u16);

        // captures_ptr: *const u64 at offset 8
        let cap_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), data_ptr, 8);

        self.builder
            .ins()
            .jump(merge_block, &[fn_id, cap_ptr, cap_count]);

        // === Non-closure path: zero everything (will use generic callback call) ===
        self.builder.switch_to_block(not_closure_block);
        self.builder.seal_block(not_closure_block);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(merge_block, &[zero, zero, zero]);

        // === Merge ===
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let function_id = self.builder.block_params(merge_block)[0];
        let captures_ptr = self.builder.block_params(merge_block)[1];
        let captures_count = self.builder.block_params(merge_block)[2];

        Some(PreloadedClosure {
            function_id,
            captures_ptr,
            captures_count,
        })
    }

    /// Optimized callback call for closures with pre-loaded data.
    /// Pushes [fn_ref, captures..., args..., arg_count] on the stack and
    /// calls jit_call_value. The fn_ref (inline function) means jit_call_value
    /// takes the fast path (no closure unboxing), while captures are included
    /// as "args" so the function sees them as its first locals.
    fn emit_closure_fast_call(&mut self, preloaded: &PreloadedClosure, args: &[Value]) -> Value {
        let base_sp = self.compile_time_sp.depth();
        let eight = self.builder.ins().iconst(types::I64, 8);

        // Construct inline function ref from pre-extracted function_id
        let tag_fn_base = self.builder.ins().iconst(
            types::I64,
            (shape_value::tags::TAG_BASE
                | (shape_value::tags::TAG_FUNCTION << shape_value::tags::TAG_SHIFT))
                as i64,
        );
        let fn_ref_val = self.builder.ins().bor(tag_fn_base, preloaded.function_id);

        // Compute stack_base before the loop so it dominates all blocks
        let stack_base = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, STACK_OFFSET as i64);

        // Store callee at base_sp
        self.builder.ins().store(
            MemFlags::trusted(),
            fn_ref_val,
            self.ctx_ptr,
            STACK_OFFSET + (base_sp as i32) * 8,
        );

        // Store captures at [base_sp+1 .. base_sp+1+captures_count)
        // Mini-loop for captures
        let cap_loop_header = self.builder.create_block();
        let cap_loop_body = self.builder.create_block();
        let cap_done = self.builder.create_block();

        self.builder.append_block_param(cap_loop_header, types::I64);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(cap_loop_header, &[zero]);

        self.builder.switch_to_block(cap_loop_header);
        let ci = self.builder.block_params(cap_loop_header)[0];
        let cap_cond =
            self.builder
                .ins()
                .icmp(IntCC::UnsignedLessThan, ci, preloaded.captures_count);
        self.builder
            .ins()
            .brif(cap_cond, cap_loop_body, &[], cap_done, &[]);

        self.builder.switch_to_block(cap_loop_body);
        self.builder.seal_block(cap_loop_body);

        // Load capture[ci]
        let cap_off = self.builder.ins().imul(ci, eight);
        let cap_addr = self.builder.ins().iadd(preloaded.captures_ptr, cap_off);
        let cap_val = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), cap_addr, 0);

        // Store at stack[base_sp + 1 + ci]
        let one = self.builder.ins().iconst(types::I64, 1);
        let dest_idx_base = self.builder.ins().iconst(types::I64, base_sp as i64);
        let dest_idx = self.builder.ins().iadd(dest_idx_base, one);
        let dest_idx = self.builder.ins().iadd(dest_idx, ci);
        let dest_off = self.builder.ins().imul(dest_idx, eight);
        let dest_addr = self.builder.ins().iadd(stack_base, dest_off);
        self.builder
            .ins()
            .store(MemFlags::trusted(), cap_val, dest_addr, 0);

        let next_ci = self.builder.ins().iadd(ci, one);
        self.builder.ins().jump(cap_loop_header, &[next_ci]);

        self.builder.switch_to_block(cap_done);
        self.builder.seal_block(cap_done);
        self.builder.seal_block(cap_loop_header);

        // Store args after captures: stack[base_sp + 1 + captures_count + j]
        for (j, &arg) in args.iter().enumerate() {
            let arg_idx_base = self
                .builder
                .ins()
                .iconst(types::I64, (base_sp + 1 + j) as i64);
            let arg_idx = self
                .builder
                .ins()
                .iadd(arg_idx_base, preloaded.captures_count);
            let arg_off = self.builder.ins().imul(arg_idx, eight);
            let arg_addr = self.builder.ins().iadd(stack_base, arg_off);
            self.builder
                .ins()
                .store(MemFlags::trusted(), arg, arg_addr, 0);
        }

        // Store arg_count = captures_count + args.len()
        let args_len = self.builder.ins().iconst(types::I64, args.len() as i64);
        let total_arg_count = self.builder.ins().iadd(preloaded.captures_count, args_len);
        let arg_count_f64 = self
            .builder
            .ins()
            .fcvt_from_sint(types::F64, total_arg_count);
        let arg_count_boxed = self.f64_to_i64(arg_count_f64);

        // arg_count goes at stack[base_sp + 1 + captures_count + args.len()]
        let ac_idx_base = self
            .builder
            .ins()
            .iconst(types::I64, (base_sp + 1 + args.len()) as i64);
        let ac_idx = self
            .builder
            .ins()
            .iadd(ac_idx_base, preloaded.captures_count);
        let ac_off = self.builder.ins().imul(ac_idx, eight);
        let ac_addr = self.builder.ins().iadd(stack_base, ac_off);
        self.builder
            .ins()
            .store(MemFlags::trusted(), arg_count_boxed, ac_addr, 0);

        // Update stack_ptr = base_sp + 1 + captures_count + args.len() + 1
        let final_sp_base = self
            .builder
            .ins()
            .iconst(types::I64, (base_sp + 2 + args.len()) as i64);
        let final_sp = self
            .builder
            .ins()
            .iadd(final_sp_base, preloaded.captures_count);
        self.builder.ins().store(
            MemFlags::trusted(),
            final_sp,
            self.ctx_ptr,
            STACK_PTR_OFFSET,
        );

        // Call jit_call_value
        let inst = self
            .builder
            .ins()
            .call(self.ffi.call_value, &[self.ctx_ptr]);
        let result = self.builder.inst_results(inst)[0];

        // Restore stack_ptr
        let restore_sp = self.builder.ins().iconst(types::I64, base_sp as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            restore_sp,
            self.ctx_ptr,
            STACK_PTR_OFFSET,
        );

        result
    }

    /// Emit an optimized callback call: uses pre-loaded closure data when available,
    /// otherwise falls back to the generic emit_callback_call.
    fn emit_callback_call_opt(
        &mut self,
        callback: Value,
        preloaded: &Option<PreloadedClosure>,
        element: Value,
        index: Value,
    ) -> Value {
        match preloaded {
            Some(pl) => {
                // Check at runtime if captures_count > 0 (i.e., this is actually a closure)
                let has_caps =
                    self.builder
                        .ins()
                        .icmp_imm(IntCC::UnsignedGreaterThan, pl.captures_count, 0);

                let fast_block = self.builder.create_block();
                let slow_block = self.builder.create_block();
                let merge = self.builder.create_block();
                self.builder.append_block_param(merge, types::I64);

                self.builder
                    .ins()
                    .brif(has_caps, fast_block, &[], slow_block, &[]);

                // Fast: closure with captures — use pre-loaded data
                self.builder.switch_to_block(fast_block);
                self.builder.seal_block(fast_block);
                let fast_result = self.emit_closure_fast_call(pl, &[element, index]);
                self.builder.ins().jump(merge, &[fast_result]);

                // Slow: non-closure or zero captures — generic path
                self.builder.switch_to_block(slow_block);
                self.builder.seal_block(slow_block);
                let slow_result = self.emit_callback_call(callback, element, index);
                self.builder.ins().jump(merge, &[slow_result]);

                self.builder.switch_to_block(merge);
                self.builder.seal_block(merge);
                self.builder.block_params(merge)[0]
            }
            None => self.emit_callback_call(callback, element, index),
        }
    }

    /// Emit an optimized 3-arg callback call for reduce.
    fn emit_callback_call_3_opt(
        &mut self,
        callback: Value,
        preloaded: &Option<PreloadedClosure>,
        acc: Value,
        element: Value,
        index: Value,
    ) -> Value {
        match preloaded {
            Some(pl) => {
                let has_caps =
                    self.builder
                        .ins()
                        .icmp_imm(IntCC::UnsignedGreaterThan, pl.captures_count, 0);

                let fast_block = self.builder.create_block();
                let slow_block = self.builder.create_block();
                let merge = self.builder.create_block();
                self.builder.append_block_param(merge, types::I64);

                self.builder
                    .ins()
                    .brif(has_caps, fast_block, &[], slow_block, &[]);

                self.builder.switch_to_block(fast_block);
                self.builder.seal_block(fast_block);
                let fast_result = self.emit_closure_fast_call(pl, &[acc, element, index]);
                self.builder.ins().jump(merge, &[fast_result]);

                self.builder.switch_to_block(slow_block);
                self.builder.seal_block(slow_block);
                let slow_result = self.emit_callback_call_3(callback, acc, element, index);
                self.builder.ins().jump(merge, &[slow_result]);

                self.builder.switch_to_block(merge);
                self.builder.seal_block(merge);
                self.builder.block_params(merge)[0]
            }
            None => self.emit_callback_call_3(callback, acc, element, index),
        }
    }

    // =========================================================================
    // A5 Optimization: Numeric element check
    // =========================================================================

    /// Emit a check for whether a NaN-boxed value is a plain f64 number.
    /// Numbers are not tagged (sign bit = 0 in the tag region), so
    /// `(bits & TAG_BASE) != TAG_BASE` means it's a number (or canonical NaN).
    fn emit_is_number(&mut self, val: Value) -> Value {
        let tag_base = self
            .builder
            .ins()
            .iconst(types::I64, shape_value::tags::TAG_BASE as i64);
        let masked = self.builder.ins().band(val, tag_base);
        let is_tagged = self.builder.ins().icmp(IntCC::Equal, masked, tag_base);
        // Number = NOT tagged
        let one = self.builder.ins().iconst(types::I8, 1);
        self.builder.ins().bxor(is_tagged, one)
    }

    // =========================================================================
    // A5 Optimization: Reduce strength reduction analysis
    // =========================================================================

    /// Detect if a callback function is a simple binary arithmetic op on its
    /// two parameters (e.g., `(a, b) => a + b`). Returns the op if detected.
    fn detect_simple_reduce_op(&self, site: &crate::optimizer::HofInlineSite) -> Option<ReduceOp> {
        let fn_id = site.callback_fn_id?;

        // Look up the function entry in the bytecode program
        let func = self.program.functions.get(fn_id as usize)?;

        // Must be a 2-parameter function (or 3 for (acc, elem, idx) but we
        // only care about (acc, elem) patterns — idx is ignored)
        if func.arity < 2 {
            return None;
        }

        // Scan the function body for the pattern:
        // LoadLocal 0, LoadLocal 1, Add/Mul, ReturnValue
        // (optionally with captures offset if it's a closure)
        let entry = func.entry_point;
        let instrs = &self.program.instructions;

        // Expect 4 instructions: load, load, op, return
        if entry + 3 >= instrs.len() {
            return None;
        }

        let captures_offset = func.captures_count;

        let i0 = &instrs[entry];
        let i1 = &instrs[entry + 1];
        let i2 = &instrs[entry + 2];
        let i3 = &instrs[entry + 3];

        // Check load patterns: LoadLocal for params 0 and 1 (offset by captures)
        let is_load_param = |instr: &shape_vm::bytecode::Instruction, param: u16| -> bool {
            instr.opcode == OpCode::LoadLocal
                && matches!(instr.operand, Some(shape_vm::bytecode::Operand::Local(idx)) if idx == captures_offset + param)
        };

        if !is_load_param(i0, 0) || !is_load_param(i1, 1) {
            return None;
        }

        // Check binary op
        let op = match i2.opcode {
            OpCode::Add | OpCode::AddNumber => Some(ReduceOp::Add),
            OpCode::Mul | OpCode::MulNumber => Some(ReduceOp::Mul),
            _ => None,
        }?;

        // Check return
        if i3.opcode != OpCode::ReturnValue {
            return None;
        }

        Some(op)
    }

    // =========================================================================
    // map: [a].map(fn) -> [fn(a[i], i)]
    // =========================================================================

    fn emit_hof_map(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        // Extract array data_ptr and length
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        // Allocate result array with same capacity
        let inst = self.builder.ins().call(self.ffi.hof_array_alloc, &[length]);
        let result_arr = self.builder.inst_results(inst)[0];

        // Loop: for i in 0..length
        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        // PHI: loop index
        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_header, types::I64); // result_arr (may be updated by push)

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero, result_arr]);

        // Loop header: check i < length
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cur_result = self.builder.block_params(loop_header)[1];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], loop_exit, &[]);

        // Loop body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        // Load element: data_ptr[i]
        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        // Box index as number
        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);

        // Push result to result array
        let push_inst = self
            .builder
            .ins()
            .call(self.ffi.hof_array_push, &[cur_result, cb_result]);
        let updated_result = self.builder.inst_results(push_inst)[0];

        // i++
        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder
            .ins()
            .jump(loop_header, &[next_i, updated_result]);

        // Loop exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);

        Ok(cur_result)
    }

    // =========================================================================
    // filter: [a].filter(fn) -> [a[i] where fn(a[i], i) is truthy]
    // =========================================================================

    fn emit_hof_filter(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        // Allocate result array (capacity = length, may be smaller)
        let inst = self.builder.ins().call(self.ffi.hof_array_alloc, &[length]);
        let result_arr = self.builder.inst_results(inst)[0];

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let push_block = self.builder.create_block();
        let skip_block = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_header, types::I64); // result_arr

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero, result_arr]);

        // Header
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cur_result = self.builder.block_params(loop_header)[1];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], loop_exit, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);
        let is_truthy = self.emit_is_truthy(cb_result);
        self.builder
            .ins()
            .brif(is_truthy, push_block, &[], skip_block, &[]);

        // Push block: element passes filter
        self.builder.switch_to_block(push_block);
        self.builder.seal_block(push_block);
        let push_inst = self
            .builder
            .ins()
            .call(self.ffi.hof_array_push, &[cur_result, element]);
        let updated = self.builder.inst_results(push_inst)[0];
        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i, updated]);

        // Skip block: element doesn't pass
        self.builder.switch_to_block(skip_block);
        self.builder.seal_block(skip_block);
        let one2 = self.builder.ins().iconst(types::I64, 1);
        let next_i2 = self.builder.ins().iadd(i, one2);
        self.builder.ins().jump(loop_header, &[next_i2, cur_result]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);

        Ok(cur_result)
    }

    // =========================================================================
    // reduce: [a].reduce(fn, init) -> fn(fn(init, a[0], 0), a[1], 1) ...
    // =========================================================================

    fn emit_hof_reduce(
        &mut self,
        receiver: Value,
        callback: Value,
        initial: Value,
        site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Detect simple reduce pattern for strength reduction
        if let Some(reduce_op) = self.detect_simple_reduce_op(site) {
            return self.emit_hof_reduce_strength(data_ptr, length, initial, reduce_op);
        }

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_header, types::I64); // accumulator

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero, initial]);

        // Header
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let acc = self.builder.block_params(loop_header)[1];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], loop_exit, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let new_acc = self.emit_callback_call_3_opt(callback, &preloaded, acc, element, i_boxed);

        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i, new_acc]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);

        Ok(acc)
    }

    // =========================================================================
    // A5 Optimization: Reduce strength reduction
    // =========================================================================

    /// Emit a strength-reduced reduce loop for simple binary arithmetic callbacks.
    /// Uses an unboxed f64 accumulator throughout the loop, only boxing at exit.
    /// For `(a, b) => a + b`: `acc_f64 = acc_f64 + unbox(element)`
    /// For `(a, b) => a * b`: `acc_f64 = acc_f64 * unbox(element)`
    fn emit_hof_reduce_strength(
        &mut self,
        data_ptr: Value,
        length: Value,
        initial: Value,
        op: ReduceOp,
    ) -> Result<Value, String> {
        // Unbox initial value to f64
        let initial_f64 = self
            .builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), initial);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_header, types::F64); // acc_f64

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero, initial_f64]);

        // Header
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let acc_f64 = self.builder.block_params(loop_header)[1];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], loop_exit, &[]);

        // Body: load element, unbox to f64, apply op
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let elem_bits = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        // A5 Numeric fast-path: check if element is a number
        let is_num = self.emit_is_number(elem_bits);

        let num_block = self.builder.create_block();
        let non_num_block = self.builder.create_block();
        let op_merge = self.builder.create_block();
        self.builder.append_block_param(op_merge, types::F64);

        self.builder
            .ins()
            .brif(is_num, num_block, &[], non_num_block, &[]);

        // Numeric path: direct f64 operation (no box/unbox per iteration)
        self.builder.switch_to_block(num_block);
        self.builder.seal_block(num_block);
        let elem_f64 = self
            .builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), elem_bits);
        let new_acc_num = match op {
            ReduceOp::Add => self.builder.ins().fadd(acc_f64, elem_f64),
            ReduceOp::Mul => self.builder.ins().fmul(acc_f64, elem_f64),
        };
        self.builder.ins().jump(op_merge, &[new_acc_num]);

        // Non-numeric path: treat as 0.0 (fallback for non-number elements)
        self.builder.switch_to_block(non_num_block);
        self.builder.seal_block(non_num_block);
        let fallback_acc = match op {
            ReduceOp::Add => acc_f64, // Adding 0 is identity
            ReduceOp::Mul => {
                let zero_f64 = self.builder.ins().f64const(0.0);
                self.builder.ins().fmul(acc_f64, zero_f64) // Mul by 0
            }
        };
        self.builder.ins().jump(op_merge, &[fallback_acc]);

        self.builder.switch_to_block(op_merge);
        self.builder.seal_block(op_merge);
        let new_acc = self.builder.block_params(op_merge)[0];

        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i, new_acc]);

        // Exit: box the f64 result back to NaN-boxed u64
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);

        let result_bits = self.f64_to_i64(acc_f64);
        Ok(result_bits)
    }

    // =========================================================================
    // find: [a].find(fn) -> first a[i] where fn(a[i], i) is truthy, or null
    // =========================================================================

    fn emit_hof_find(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let found_block = self.builder.create_block();
        let not_found_block = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_exit, types::I64); // result

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero]);

        // Header: branch to not_found (not loop_exit directly) when done
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], not_found_block, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);
        let is_truthy = self.emit_is_truthy(cb_result);

        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_truthy, found_block, &[], continue_block, &[]);

        // Found: jump to exit with element
        self.builder.switch_to_block(found_block);
        self.builder.seal_block(found_block);
        self.builder.ins().jump(loop_exit, &[element]);

        // Continue
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i]);

        // Not found: return TAG_NULL
        self.builder.switch_to_block(not_found_block);
        self.builder.seal_block(not_found_block);
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.builder.ins().jump(loop_exit, &[null_val]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);
        let result = self.builder.block_params(loop_exit)[0];
        Ok(result)
    }

    // =========================================================================
    // findIndex: [a].findIndex(fn) -> index of first truthy, or -1
    // =========================================================================

    fn emit_hof_find_index(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let found_block = self.builder.create_block();
        let not_found_block = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_exit, types::I64); // result

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero]);

        // Header: branch to not_found_block (not loop_exit) when done
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], not_found_block, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);
        let is_truthy = self.emit_is_truthy(cb_result);

        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_truthy, found_block, &[], continue_block, &[]);

        // Found: return boxed index
        self.builder.switch_to_block(found_block);
        self.builder.seal_block(found_block);
        self.builder.ins().jump(loop_exit, &[i_boxed]);

        // Continue
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i]);

        // Not found: return -1
        self.builder.switch_to_block(not_found_block);
        self.builder.seal_block(not_found_block);
        let neg_one = self
            .builder
            .ins()
            .iconst(types::I64, box_number(-1.0) as i64);
        self.builder.ins().jump(loop_exit, &[neg_one]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);
        let result = self.builder.block_params(loop_exit)[0];
        Ok(result)
    }

    // =========================================================================
    // some: [a].some(fn) -> true if any fn(a[i], i) is truthy
    // =========================================================================

    fn emit_hof_some(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let found_block = self.builder.create_block();
        let none_found_block = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_exit, types::I64); // result

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero]);

        // Header: branch to none_found_block (not loop_exit) when done
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], none_found_block, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);
        let is_truthy = self.emit_is_truthy(cb_result);

        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_truthy, found_block, &[], continue_block, &[]);

        // Found: return true
        self.builder.switch_to_block(found_block);
        self.builder.seal_block(found_block);
        let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
        self.builder.ins().jump(loop_exit, &[true_val]);

        // Continue
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i]);

        // None found: return false
        self.builder.switch_to_block(none_found_block);
        self.builder.seal_block(none_found_block);
        let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
        self.builder.ins().jump(loop_exit, &[false_val]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);
        let result = self.builder.block_params(loop_exit)[0];
        Ok(result)
    }

    // =========================================================================
    // every: [a].every(fn) -> true if all fn(a[i], i) are truthy
    // =========================================================================

    fn emit_hof_every(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let fail_block = self.builder.create_block();
        let all_passed_block = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i
        self.builder.append_block_param(loop_exit, types::I64); // result

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero]);

        // Header: branch to all_passed_block (not loop_exit) when done
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], all_passed_block, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data
        let cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);
        let is_truthy = self.emit_is_truthy(cb_result);

        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_truthy, continue_block, &[], fail_block, &[]);

        // Fail: found falsy -> return false
        self.builder.switch_to_block(fail_block);
        self.builder.seal_block(fail_block);
        let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
        self.builder.ins().jump(loop_exit, &[false_val]);

        // Continue
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i]);

        // All passed: return true
        self.builder.switch_to_block(all_passed_block);
        self.builder.seal_block(all_passed_block);
        let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
        self.builder.ins().jump(loop_exit, &[true_val]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);
        let result = self.builder.block_params(loop_exit)[0];
        Ok(result)
    }

    // =========================================================================
    // forEach: [a].forEach(fn) -> null (side effects only)
    // =========================================================================

    fn emit_hof_foreach(
        &mut self,
        receiver: Value,
        callback: Value,
        _site: &crate::optimizer::HofInlineSite,
    ) -> Result<Value, String> {
        let (data_ptr, length) = self.emit_array_data_ptr(receiver);

        // A5: Pre-load closure captures before entering loop
        let preloaded = self.try_preload_closure(callback);

        let loop_header = self.builder.create_block();
        let loop_body = self.builder.create_block();
        let loop_exit = self.builder.create_block();

        self.builder.append_block_param(loop_header, types::I64); // i

        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().jump(loop_header, &[zero]);

        // Header
        self.builder.switch_to_block(loop_header);
        let i = self.builder.block_params(loop_header)[0];
        let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
        self.builder
            .ins()
            .brif(cond, loop_body, &[], loop_exit, &[]);

        // Body
        self.builder.switch_to_block(loop_body);
        self.builder.seal_block(loop_body);

        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(i, eight);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let element = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), elem_addr, 0);

        let i_f64 = self.builder.ins().fcvt_from_sint(types::F64, i);
        let i_boxed = self.f64_to_i64(i_f64);

        // A5: Use optimized callback call with pre-loaded closure data (discard result)
        let _cb_result = self.emit_callback_call_opt(callback, &preloaded, element, i_boxed);

        let one = self.builder.ins().iconst(types::I64, 1);
        let next_i = self.builder.ins().iadd(i, one);
        self.builder.ins().jump(loop_header, &[next_i]);

        // Exit
        self.builder.switch_to_block(loop_exit);
        self.builder.seal_block(loop_exit);
        self.builder.seal_block(loop_header);

        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        Ok(null_val)
    }
}
