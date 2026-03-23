//! Function calls and returns: Call, CallValue, CallMethod, Return

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use std::collections::HashMap;

use crate::context::{STACK_OFFSET, STACK_PTR_OFFSET};
use crate::nan_boxing::*;
use shape_vm::bytecode::{Instruction, OpCode, Operand};
use shape_vm::type_tracking::StorageHint;

use crate::translator::types::{BytecodeToIR, InlineCandidate, InlineFrameContext};
use shape_vm::type_tracking::SlotKind;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    pub(crate) fn compile_call(&mut self, instr: &Instruction, idx: usize) -> Result<(), String> {
        if let Some(Operand::Function(fn_id)) = &instr.operand {
            let _arg_count_ssa = self.stack_pop();
            let arg_count = self.get_arg_count_from_prev_instruction(idx);
            let discard_result = self
                .program
                .instructions
                .get(idx + 1)
                .map(|next| next.opcode == OpCode::Pop)
                .unwrap_or(false);
            let direct_arity_ok = self
                .user_func_arities
                .get(&fn_id.0)
                .map(|&arity| arity as usize == arg_count)
                .unwrap_or(false);
            let inline_depth_limit = self.optimization_plan.call_path.inline_depth_limit;
            let prefer_direct = self
                .optimization_plan
                .call_path
                .prefer_direct_call_sites
                .contains(&idx);

            if prefer_direct {
                if direct_arity_ok {
                    if let Some(&func_ref) = self.user_funcs.get(&fn_id.0) {
                        self.compile_direct_call(func_ref, arg_count, discard_result)?;
                        self.reload_referenced_locals();
                        return Ok(());
                    }
                } else {
                    self.compile_ffi_call(fn_id.0, arg_count, discard_result)?;
                    self.reload_referenced_locals();
                    return Ok(());
                }
            }

            // Check for inline candidate (small functions → zero call overhead)
            if self.inline_depth < inline_depth_limit {
                if let Some(candidate) = self.inline_candidates.get(&fn_id.0).cloned() {
                    if candidate.arity as usize == arg_count {
                        return self.compile_inline_call(fn_id.0, candidate, arg_count, idx);
                    }
                }
            }

            // Check if we have a direct reference to this function (bypasses FFI)
            if direct_arity_ok {
                if let Some(&func_ref) = self.user_funcs.get(&fn_id.0) {
                    // DIRECT CALL PATH - no FFI overhead!
                    self.compile_direct_call(func_ref, arg_count, discard_result)?;
                } else {
                    // FALLBACK: FFI call (for dynamic/closure calls or when user_funcs is empty)
                    self.compile_ffi_call(fn_id.0, arg_count, discard_result)?;
                }
            } else {
                // Arity mismatch at compile site: preserve VM semantics via FFI.
                self.compile_ffi_call(fn_id.0, arg_count, discard_result)?;
            }
        } else if let Some(Operand::Count(arg_count)) = &instr.operand {
            // This path doesn't have a fn_id, use FFI
            let arg_count_usize = *arg_count as usize;
            let discard_result = self
                .program
                .instructions
                .get(idx + 1)
                .map(|next| next.opcode == OpCode::Pop)
                .unwrap_or(false);
            self.compile_ffi_call(0, arg_count_usize, discard_result)?;
        }
        // After any call, reload locals that were MakeRef'd — the callee
        // may have modified them through the reference pointer.
        self.reload_referenced_locals();
        Ok(())
    }

    /// Compile a direct JIT call to a user-defined function.
    /// This bypasses FFI and emits a native call using the internal
    /// signature: `fn(ctx_ptr, arg0, arg1, ..., argN) -> i32`.
    fn compile_direct_call(
        &mut self,
        func_ref: FuncRef,
        arg_count: usize,
        discard_result: bool,
    ) -> Result<(), String> {
        // 1. Pop args from SSA stack.
        let mut arg_vals = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if let Some(val) = self.stack_pop() {
                arg_vals.push(val);
            } else {
                break;
            }
        }

        // If we didn't get all args from SSA stack, use materialize path.
        if arg_vals.len() < arg_count {
            for val in arg_vals.into_iter().rev() {
                self.stack_push(val);
            }
            self.materialize_to_stack(arg_count);

            let stack_base_offset = STACK_OFFSET;
            let stack_ptr_offset = STACK_PTR_OFFSET;

            let stack_ptr = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                self.ctx_ptr,
                stack_ptr_offset,
            );

            let mut materialized_args = Vec::with_capacity(arg_count);
            for i in 0..arg_count {
                let offset_i64 = self
                    .builder
                    .ins()
                    .iconst(types::I64, (arg_count - 1 - i) as i64);
                let stack_idx = self.builder.ins().isub(stack_ptr, offset_i64);
                let one = self.builder.ins().iconst(types::I64, 1);
                let stack_idx = self.builder.ins().isub(stack_idx, one);
                let eight = self.builder.ins().iconst(types::I64, 8);
                let byte_offset = self.builder.ins().imul(stack_idx, eight);
                let stack_addr = self
                    .builder
                    .ins()
                    .iadd_imm(self.ctx_ptr, stack_base_offset as i64);
                let addr = self.builder.ins().iadd(stack_addr, byte_offset);
                let arg_val = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::trusted(), addr, 0);
                materialized_args.push(arg_val);
            }

            // Decrement stack_ptr by arg_count
            let arg_count_val = self.builder.ins().iconst(types::I64, arg_count as i64);
            let new_stack_ptr = self.builder.ins().isub(stack_ptr, arg_count_val);
            self.builder.ins().store(
                MemFlags::trusted(),
                new_stack_ptr,
                self.ctx_ptr,
                stack_ptr_offset,
            );

            let mut call_args = Vec::with_capacity(arg_count + 1);
            call_args.push(self.ctx_ptr);
            call_args.extend(materialized_args);

            let inst = self.builder.ins().call(func_ref, &call_args);
            let signal = self.builder.inst_results(inst)[0];
            self.deopt_if_negative_signal(signal);
        } else {
            // Fast path: got all args from SSA stack.
            arg_vals.reverse(); // Restore original parameter order.
            let mut call_args = Vec::with_capacity(arg_count + 1);
            call_args.push(self.ctx_ptr);
            call_args.extend(arg_vals);

            let inst = self.builder.ins().call(func_ref, &call_args);
            let signal = self.builder.inst_results(inst)[0];
            self.deopt_if_negative_signal(signal);
        }

        // Push call result unless it is immediately discarded by Pop.
        if discard_result {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        } else {
            let result = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                self.ctx_ptr,
                STACK_OFFSET,
            );
            self.stack_push(result);
        }

        Ok(())
    }

    /// Compile an FFI call to jit_call_function (fallback path)
    fn compile_ffi_call(
        &mut self,
        fn_id: u16,
        arg_count: usize,
        discard_result: bool,
    ) -> Result<(), String> {
        if arg_count > 0 {
            self.materialize_to_stack(arg_count);
        }

        let fn_id_val = self.builder.ins().iconst(types::I16, fn_id as i64);
        let null_ptr = self.builder.ins().iconst(types::I64, 0);
        let arg_count_val = self.builder.ins().iconst(types::I64, arg_count as i64);

        let inst = self.builder.ins().call(
            self.ffi.call_function,
            &[self.ctx_ptr, fn_id_val, null_ptr, arg_count_val],
        );
        let result = self.builder.inst_results(inst)[0];
        self.update_sp_after_ffi(arg_count, 0);
        if discard_result {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        } else {
            self.stack_push(result);
        }
        Ok(())
    }

    pub(crate) fn compile_call_value(&mut self, idx: usize) -> Result<(), String> {
        let arg_count = self.get_arg_count_from_prev_instruction(idx);
        let items_needed = arg_count + 2;

        // Speculative fast path: if feedback indicates a monomorphic call target,
        // emit a guarded direct call instead of going through the generic FFI.
        if self.has_feedback() {
            if let Some(target_fn_id) = self.speculative_call_target(idx) {
                if let Some(&func_ref) = self.user_funcs.get(&target_fn_id) {
                    // Self-recursive or same-module call: we have a FuncRef, emit direct call.
                    let arity_ok = self
                        .user_func_arities
                        .get(&target_fn_id)
                        .map(|&arity| arity as usize == arg_count)
                        .unwrap_or(false);

                    if arity_ok && self.stack_len() >= items_needed {
                        let callee_depth = items_needed;
                        if let Some(callee_val) = self.stack_peek_at(callee_depth - 1) {
                            self.emit_speculative_call(target_fn_id, callee_val, idx);

                            // Guard passed: emit a direct call
                            let _arg_count_ssa = self.stack_pop();
                            let _callee = self.stack_pop();

                            let discard_result = self
                                .program
                                .instructions
                                .get(idx + 1)
                                .map(|next| next.opcode == OpCode::Pop)
                                .unwrap_or(false);

                            self.compile_direct_call(func_ref, arg_count, discard_result)?;
                            self.reload_referenced_locals();
                            return Ok(());
                        }
                    }
                } else if self.stack_len() >= items_needed {
                    // Cross-function call without a FuncRef (callee not yet Tier-2
                    // compiled). Emit a callee identity guard; on success, fall
                    // through to the generic FFI path (known-monomorphic). On
                    // guard failure, deopt.
                    let callee_depth = items_needed;
                    if let Some(callee_val) = self.stack_peek_at(callee_depth - 1) {
                        self.emit_speculative_call(target_fn_id, callee_val, idx);
                    }
                }
            }
        }

        if self.stack_len() >= items_needed {
            self.materialize_to_stack(items_needed);

            let inst = self
                .builder
                .ins()
                .call(self.ffi.call_value, &[self.ctx_ptr]);
            let result = self.builder.inst_results(inst)[0];

            self.update_sp_after_ffi(items_needed, 0);
            self.stack_push(result);
        } else if self.stack_len() >= 2 {
            let count = self.stack_len();
            self.materialize_to_stack(count);

            let inst = self
                .builder
                .ins()
                .call(self.ffi.call_value, &[self.ctx_ptr]);
            let result = self.builder.inst_results(inst)[0];

            self.update_sp_after_ffi(count, 0);
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        self.reload_referenced_locals();
        Ok(())
    }

    pub(crate) fn compile_call_foreign(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        let _arg_count_ssa = self.stack_pop();
        let arg_count = self.get_arg_count_from_prev_instruction(idx);
        let foreign_idx = match instr.operand {
            Some(Operand::ForeignFunction(idx)) => idx as u32,
            _ => return Err("CallForeign missing ForeignFunction operand".to_string()),
        };
        let foreign_is_native = self
            .program
            .foreign_functions
            .get(foreign_idx as usize)
            .map(|entry| entry.native_abi.is_some())
            .unwrap_or(false);

        if foreign_is_native
            && let Some(result) =
                self.try_compile_call_foreign_native_direct(foreign_idx, arg_count)
        {
            self.stack_push(result);
            self.reload_referenced_locals();
            return Ok(());
        }

        if arg_count > 0 {
            self.materialize_to_stack(arg_count);
        }

        let foreign_idx_val = self.builder.ins().iconst(types::I32, foreign_idx as i64);
        let arg_count_val = self.builder.ins().iconst(types::I64, arg_count as i64);
        let foreign_call = if foreign_is_native {
            self.ffi.call_foreign_native
        } else {
            self.ffi.call_foreign_dynamic
        };
        let inst = self.builder.ins().call(
            foreign_call,
            &[self.ctx_ptr, foreign_idx_val, arg_count_val],
        );
        let result = self.builder.inst_results(inst)[0];
        self.update_sp_after_ffi(arg_count, 0);
        self.stack_push(result);
        self.reload_referenced_locals();
        Ok(())
    }

    fn try_compile_call_foreign_native_direct(
        &mut self,
        foreign_idx: u32,
        arg_count: usize,
    ) -> Option<Value> {
        if arg_count > 8 || self.stack_len() < arg_count {
            return None;
        }

        let mut args: Vec<Value> = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(self.stack_pop_boxed()?);
        }
        args.reverse();

        let foreign_idx_val = self.builder.ins().iconst(types::I32, foreign_idx as i64);
        let inst = match arg_count {
            0 => self.builder.ins().call(
                self.ffi.call_foreign_native_0,
                &[self.ctx_ptr, foreign_idx_val],
            ),
            1 => self.builder.ins().call(
                self.ffi.call_foreign_native_1,
                &[self.ctx_ptr, foreign_idx_val, args[0]],
            ),
            2 => self.builder.ins().call(
                self.ffi.call_foreign_native_2,
                &[self.ctx_ptr, foreign_idx_val, args[0], args[1]],
            ),
            3 => self.builder.ins().call(
                self.ffi.call_foreign_native_3,
                &[self.ctx_ptr, foreign_idx_val, args[0], args[1], args[2]],
            ),
            4 => self.builder.ins().call(
                self.ffi.call_foreign_native_4,
                &[
                    self.ctx_ptr,
                    foreign_idx_val,
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                ],
            ),
            5 => self.builder.ins().call(
                self.ffi.call_foreign_native_5,
                &[
                    self.ctx_ptr,
                    foreign_idx_val,
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                    args[4],
                ],
            ),
            6 => self.builder.ins().call(
                self.ffi.call_foreign_native_6,
                &[
                    self.ctx_ptr,
                    foreign_idx_val,
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                    args[4],
                    args[5],
                ],
            ),
            7 => self.builder.ins().call(
                self.ffi.call_foreign_native_7,
                &[
                    self.ctx_ptr,
                    foreign_idx_val,
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                    args[4],
                    args[5],
                    args[6],
                ],
            ),
            8 => self.builder.ins().call(
                self.ffi.call_foreign_native_8,
                &[
                    self.ctx_ptr,
                    foreign_idx_val,
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                    args[4],
                    args[5],
                    args[6],
                    args[7],
                ],
            ),
            _ => return None,
        };
        Some(self.builder.inst_results(inst)[0])
    }

    pub(crate) fn compile_call_method(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        // Extract method_id and arg_count from TypedMethodCall operand if available.
        // This enables inline fast paths for known methods without FFI overhead.
        let (method_id, operand_arg_count) = match &instr.operand {
            Some(Operand::TypedMethodCall {
                method_id,
                arg_count,
                ..
            }) => (Some(*method_id), *arg_count as usize),
            _ => (None, self.get_arg_count_from_prev_instruction(idx)),
        };

        // Try inline fast path for known methods on known types.
        // These avoid materializing to the context stack and the FFI call entirely.
        if let Some(mid) = method_id {
            if operand_arg_count == 0 {
                if let Some(()) = self.try_inline_method(mid)? {
                    self.reload_referenced_locals();
                    return Ok(());
                }
            }
            if operand_arg_count == 1 {
                if let Some(()) = self.try_inline_method_1arg(mid)? {
                    self.reload_referenced_locals();
                    return Ok(());
                }
            }
            if self.should_direct_dispatch_typed_method(idx, mid) {
                if let Some(()) = self.try_direct_typed_method_dispatch(mid, operand_arg_count)? {
                    self.reload_referenced_locals();
                    return Ok(());
                }
            }

            // Try HOF inlining (map/filter/reduce/find/some/every/forEach/findIndex)
            if let Some(()) = self.try_inline_hof_method(mid, operand_arg_count, idx)? {
                return Ok(());
            }
        }

        // Fallback: full FFI dispatch through emit_method_ffi_fallback
        // TypedMethodCall: stack has [receiver, arg0, arg1, ...] — no overhead values
        let arg_count = operand_arg_count;
        let needed_values = arg_count + 1; // receiver + args

        if self.stack_len() >= needed_values {
            // Pop args and receiver from SSA stack
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                if let Some(arg) = self.stack_pop() {
                    args.push(arg);
                }
            }
            args.reverse();
            let receiver = match self.stack_pop() {
                Some(v) => v,
                None => {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                    self.reload_referenced_locals();
                    return Ok(());
                }
            };

            let result = self.emit_method_ffi_fallback(
                receiver,
                method_id.unwrap_or(0),
                &args,
            );
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        self.reload_referenced_locals();
        Ok(())
    }

    /// Try to inline a zero-argument method call.
    ///
    /// For known method IDs with zero arguments, we can emit inline IR that:
    /// 1. Pops the receiver from the SSA stack
    /// 2. Emits a runtime type check on the receiver tag
    /// 3. Emits the inlined operation (e.g., array length read, f64 abs/floor/ceil)
    /// 4. Falls back to FFI if the receiver type doesn't match
    ///
    /// Returns `Some(())` if the method was inlined, `None` if it should use FFI.
    fn try_inline_method(&mut self, method_id: u16) -> Result<Option<()>, String> {
        use shape_value::MethodId;

        // Stack layout (TypedMethodCall): [..., receiver]
        // method_id and arg_count are in the operand, not on the stack
        if self.stack_len() < 1 {
            return Ok(None);
        }

        match method_id {
            // arr.length / arr.len -> inline array length read
            id if id == MethodId::LENGTH.0 || id == MethodId::LEN.0 => {
                let receiver = self.stack_pop().unwrap();

                // Runtime heap kind check: is receiver an array?
                let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_array, inline_block, &[], ffi_block, &[]);

                // Inline path: read JitArray.len directly
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let result = self.inline_array_length(receiver);
                self.builder.ins().jump(merge_block, &[result]);

                // FFI fallback for non-array receivers (string.length, etc.)
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // num.abs() -> inline fabs
            id if id == MethodId::ABS.0 => {
                let receiver = self.stack_pop().unwrap();

                // Runtime check: is receiver a number?
                let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
                let masked = self.builder.ins().band(receiver, nan_base);
                let is_num = self.builder.ins().icmp(IntCC::NotEqual, masked, nan_base);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_num, inline_block, &[], ffi_block, &[]);

                // Inline: fabs
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let f64_val = self.i64_to_f64(receiver);
                let abs_val = self.builder.ins().fabs(f64_val);
                let result = self.f64_to_i64(abs_val);
                self.builder.ins().jump(merge_block, &[result]);

                // FFI fallback for non-numeric receivers
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // num.floor() -> inline floor
            id if id == MethodId::FLOOR.0 => {
                self.inline_number_unary_method(|b, v| b.ins().floor(v))
            }

            // num.ceil() -> inline ceil
            id if id == MethodId::CEIL.0 => self.inline_number_unary_method(|b, v| b.ins().ceil(v)),

            // num.round() -> inline nearest (round to nearest even)
            id if id == MethodId::ROUND.0 => {
                self.inline_number_unary_method(|b, v| b.ins().nearest(v))
            }

            // arr.isEmpty -> inline: length == 0 ? true : false
            id if id == MethodId::IS_EMPTY.0 => {
                let receiver = self.stack_pop().unwrap();

                let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_array, inline_block, &[], ffi_block, &[]);

                // Inline path: check if len == 0
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let (_, length) = self.emit_array_data_ptr(receiver);
                let zero = self.builder.ins().iconst(types::I64, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, length, zero);
                let result = self.emit_boxed_bool_from_i1(is_zero);
                self.builder.ins().jump(merge_block, &[result]);

                // FFI fallback
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // arr.first -> inline: len > 0 ? data[0] : null
            id if id == MethodId::FIRST.0 => {
                let receiver = self.stack_pop().unwrap();

                let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_array, inline_block, &[], ffi_block, &[]);

                // Inline path: branch on len > 0 to avoid null-ptr dereference
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let (data_ptr, length) = self.emit_array_data_ptr(receiver);
                let zero = self.builder.ins().iconst(types::I64, 0);
                let has_elements =
                    self.builder
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThan, length, zero);

                let load_block = self.builder.create_block();
                let empty_block = self.builder.create_block();
                self.builder
                    .ins()
                    .brif(has_elements, load_block, &[], empty_block, &[]);

                // Non-empty: load data[0]
                self.builder.switch_to_block(load_block);
                self.builder.seal_block(load_block);
                let first_elem = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::new(), data_ptr, 0);
                self.builder.ins().jump(merge_block, &[first_elem]);

                // Empty: return null
                self.builder.switch_to_block(empty_block);
                self.builder.seal_block(empty_block);
                let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                self.builder.ins().jump(merge_block, &[null_val]);

                // FFI fallback
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // arr.last -> inline: len > 0 ? data[len-1] : null
            id if id == MethodId::LAST.0 => {
                let receiver = self.stack_pop().unwrap();

                let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_array, inline_block, &[], ffi_block, &[]);

                // Inline path: branch on len > 0 to avoid null-ptr dereference
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let (data_ptr, length) = self.emit_array_data_ptr(receiver);
                let zero = self.builder.ins().iconst(types::I64, 0);
                let has_elements =
                    self.builder
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThan, length, zero);

                let load_block = self.builder.create_block();
                let empty_block = self.builder.create_block();
                self.builder
                    .ins()
                    .brif(has_elements, load_block, &[], empty_block, &[]);

                // Non-empty: load data[(len-1)*8]
                self.builder.switch_to_block(load_block);
                self.builder.seal_block(load_block);
                let one = self.builder.ins().iconst(types::I64, 1);
                let last_idx = self.builder.ins().isub(length, one);
                let eight = self.builder.ins().iconst(types::I64, 8);
                let byte_offset = self.builder.ins().imul(last_idx, eight);
                let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
                let last_elem = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::new(), elem_addr, 0);
                self.builder.ins().jump(merge_block, &[last_elem]);

                // Empty: return null
                self.builder.switch_to_block(empty_block);
                self.builder.seal_block(empty_block);
                let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                self.builder.ins().jump(merge_block, &[null_val]);

                // FFI fallback
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // arr.pop() -> FFI to jit_array_pop
            id if id == MethodId::POP.0 => {
                let receiver = self.stack_pop().unwrap();

                let inst = self.builder.ins().call(self.ffi.array_pop, &[receiver]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // arr.reverse() -> FFI to jit_array_reverse
            id if id == MethodId::REVERSE.0 => {
                let receiver = self.stack_pop().unwrap();

                let inst = self.builder.ins().call(self.ffi.array_reverse, &[receiver]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            _ => Ok(None),
        }
    }

    /// Try to inline a one-argument method call.
    ///
    /// For known method IDs with one argument, we can emit inline IR that:
    /// 1. Pops the single argument
    /// 2. Pops the receiver
    /// 3. Emits a runtime type check on the receiver tag
    /// 4. Emits the inlined operation or an FFI helper call
    /// 5. Falls back to FFI if the receiver type doesn't match
    ///
    /// Returns `Some(())` if the method was inlined, `None` if it should use FFI.
    fn try_inline_method_1arg(&mut self, method_id: u16) -> Result<Option<()>, String> {
        use shape_value::MethodId;

        // Stack layout (TypedMethodCall): [..., receiver, arg0]
        // method_id and arg_count are in the operand, not on the stack
        if self.stack_len() < 2 {
            return Ok(None);
        }

        match method_id {
            // arr.includes(value) -> inline linear scan with bitwise equality
            id if id == MethodId::INCLUDES.0 => {
                let arg0 = self.stack_pop().unwrap();
                let receiver = self.stack_pop().unwrap();

                let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_array, inline_block, &[], ffi_block, &[]);

                // Inline path: linear scan loop
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let (data_ptr, length) = self.emit_array_data_ptr(receiver);

                // Loop: for i in 0..len { if data[i] == needle { return TRUE } }
                let loop_header = self.builder.create_block();
                let loop_body = self.builder.create_block();
                let found_block = self.builder.create_block();
                let not_found_block = self.builder.create_block();
                self.builder.append_block_param(loop_header, types::I64); // i

                let zero = self.builder.ins().iconst(types::I64, 0);
                self.builder.ins().jump(loop_header, &[zero]);

                // Loop header: check i < len
                self.builder.switch_to_block(loop_header);
                let i = self.builder.block_params(loop_header)[0];
                let in_bounds = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
                self.builder
                    .ins()
                    .brif(in_bounds, loop_body, &[], not_found_block, &[]);

                // Loop body: load data[i], compare with needle
                self.builder.switch_to_block(loop_body);
                let eight = self.builder.ins().iconst(types::I64, 8);
                let byte_offset = self.builder.ins().imul(i, eight);
                let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
                let elem = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::new(), elem_addr, 0);
                let matches = self.builder.ins().icmp(IntCC::Equal, elem, arg0);
                let one = self.builder.ins().iconst(types::I64, 1);
                let next_i = self.builder.ins().iadd(i, one);
                self.builder
                    .ins()
                    .brif(matches, found_block, &[], loop_header, &[next_i]);

                self.builder.seal_block(loop_header);
                self.builder.seal_block(loop_body);

                // Found
                self.builder.switch_to_block(found_block);
                self.builder.seal_block(found_block);
                let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
                self.builder.ins().jump(merge_block, &[true_val]);

                // Not found
                self.builder.switch_to_block(not_found_block);
                self.builder.seal_block(not_found_block);
                let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                self.builder.ins().jump(merge_block, &[false_val]);

                // FFI fallback for non-array receivers
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[arg0]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // arr.indexOf(needle) -> inline linear scan, return index or -1
            id if id == MethodId::INDEX_OF.0 => {
                let arg0 = self.stack_pop().unwrap();
                let receiver = self.stack_pop().unwrap();

                let is_array = self.emit_is_heap_kind(receiver, HK_ARRAY);

                let inline_block = self.builder.create_block();
                let ffi_block = self.builder.create_block();
                let merge_block = self.builder.create_block();
                self.builder.append_block_param(merge_block, types::I64);

                self.builder
                    .ins()
                    .brif(is_array, inline_block, &[], ffi_block, &[]);

                // Inline path: linear scan loop
                self.builder.switch_to_block(inline_block);
                self.builder.seal_block(inline_block);
                let (data_ptr, length) = self.emit_array_data_ptr(receiver);

                let loop_header = self.builder.create_block();
                let loop_body = self.builder.create_block();
                let found_block = self.builder.create_block();
                self.builder.append_block_param(found_block, types::I64); // found index
                let not_found_block = self.builder.create_block();
                self.builder.append_block_param(loop_header, types::I64); // i

                let zero = self.builder.ins().iconst(types::I64, 0);
                self.builder.ins().jump(loop_header, &[zero]);

                // Loop header
                self.builder.switch_to_block(loop_header);
                let i = self.builder.block_params(loop_header)[0];
                let in_bounds = self.builder.ins().icmp(IntCC::UnsignedLessThan, i, length);
                self.builder
                    .ins()
                    .brif(in_bounds, loop_body, &[], not_found_block, &[]);

                // Loop body
                self.builder.switch_to_block(loop_body);
                let eight = self.builder.ins().iconst(types::I64, 8);
                let byte_offset = self.builder.ins().imul(i, eight);
                let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
                let elem = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::new(), elem_addr, 0);
                let matches = self.builder.ins().icmp(IntCC::Equal, elem, arg0);
                let one = self.builder.ins().iconst(types::I64, 1);
                let next_i = self.builder.ins().iadd(i, one);
                self.builder
                    .ins()
                    .brif(matches, found_block, &[i], loop_header, &[next_i]);

                self.builder.seal_block(loop_header);
                self.builder.seal_block(loop_body);

                // Found: box index as number
                self.builder.switch_to_block(found_block);
                self.builder.seal_block(found_block);
                let found_idx = self.builder.block_params(found_block)[0];
                let idx_f64 = self.builder.ins().fcvt_from_sint(types::F64, found_idx);
                let result = self.f64_to_i64(idx_f64);
                self.builder.ins().jump(merge_block, &[result]);

                // Not found: return -1 as NaN-boxed number
                self.builder.switch_to_block(not_found_block);
                self.builder.seal_block(not_found_block);
                let neg_one_bits = crate::nan_boxing::box_number(-1.0);
                let neg_one = self.builder.ins().iconst(types::I64, neg_one_bits as i64);
                self.builder.ins().jump(merge_block, &[neg_one]);

                // FFI fallback
                self.builder.switch_to_block(ffi_block);
                self.builder.seal_block(ffi_block);
                let ffi_result = self.emit_method_ffi_fallback(receiver, method_id, &[arg0]);
                self.builder.ins().jump(merge_block, &[ffi_result]);

                self.builder.switch_to_block(merge_block);
                self.builder.seal_block(merge_block);
                let result = self.builder.block_params(merge_block)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            // arr.push(element) -> FFI to jit_array_push_element
            id if id == MethodId::PUSH.0 => {
                let arg0 = self.stack_pop().unwrap();
                let receiver = self.stack_pop().unwrap();

                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.array_push_element, &[receiver, arg0]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
                Ok(Some(()))
            }

            _ => Ok(None),
        }
    }

    fn should_direct_dispatch_typed_method(&self, idx: usize, method_id: u16) -> bool {
        let plan = &self.optimization_plan.table_queryable;
        plan.filter_sites.contains(&idx)
            || plan.map_sites.contains(&idx)
            || plan.count_sites.contains(&idx)
            || plan.limit_sites.contains(&idx)
            || plan.order_by_sites.contains(&idx)
            || plan.typed_column_load_sites.contains(&idx)
            || method_id == shape_value::MethodId::COUNT.0
            || method_id == shape_value::MethodId::FILTER.0
            || method_id == shape_value::MethodId::MAP.0
            || method_id == shape_value::MethodId::LIMIT.0
            || method_id == shape_value::MethodId::TAKE.0
            || method_id == shape_value::MethodId::ORDER_BY.0
    }

    fn try_direct_typed_method_dispatch(
        &mut self,
        method_id: u16,
        arg_count: usize,
    ) -> Result<Option<()>, String> {
        let needed = arg_count + 1; // receiver + args
        if self.stack_len() < needed {
            return Ok(None);
        }

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
        let receiver = match self.stack_pop() {
            Some(v) => v,
            None => return Ok(None),
        };

        let result = self.emit_method_ffi_fallback(receiver, method_id, &args);
        self.stack_push(result);
        Ok(Some(()))
    }

    /// Helper: inline a unary f64 method on a number receiver (floor, ceil, round).
    /// Pops receiver from SSA stack, emits type check + inline IR.
    fn inline_number_unary_method<F>(&mut self, op: F) -> Result<Option<()>, String>
    where
        F: FnOnce(&mut FunctionBuilder, Value) -> Value,
    {
        let receiver = self.stack_pop().unwrap();

        // Runtime check: is receiver a number?
        let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
        let masked = self.builder.ins().band(receiver, nan_base);
        let is_num = self.builder.ins().icmp(IntCC::NotEqual, masked, nan_base);

        let inline_block = self.builder.create_block();
        let else_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);

        self.builder
            .ins()
            .brif(is_num, inline_block, &[], else_block, &[]);

        // Inline: apply f64 operation
        self.builder.switch_to_block(inline_block);
        self.builder.seal_block(inline_block);
        let f64_val = self.i64_to_f64(receiver);
        let result_f64 = op(self.builder, f64_val);
        let result = self.f64_to_i64(result_f64);
        self.builder.ins().jump(merge_block, &[result]);

        // Non-numeric: return TAG_NULL (these methods don't apply to non-numbers)
        self.builder.switch_to_block(else_block);
        self.builder.seal_block(else_block);
        let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
        self.builder.ins().jump(merge_block, &[null_val]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let result = self.builder.block_params(merge_block)[0];
        self.stack_push(result);
        Ok(Some(()))
    }

    /// Emit an FFI fallback call for a method that we tried to inline but the
    /// receiver didn't match the expected type. Rebuilds the context stack with
    /// receiver + args + method_name + arg_count and calls jit_call_method.
    pub(crate) fn emit_method_ffi_fallback(
        &mut self,
        receiver: Value,
        method_id: u16,
        args: &[Value],
    ) -> Value {
        use crate::context::{STACK_OFFSET, STACK_PTR_OFFSET};

        // Reconstruct the method name string constant from method_id
        let method_name_str = shape_value::MethodId(method_id).name().unwrap_or("unknown");
        let method_str = method_name_str.to_string();
        let str_bits = jit_box(HK_STRING, method_str);
        let method_val = self.builder.ins().iconst(types::I64, str_bits as i64);

        let arg_count = args.len();
        let arg_count_bits = crate::nan_boxing::box_number(arg_count as f64);
        let arg_count_val = self.builder.ins().iconst(types::I64, arg_count_bits as i64);

        // Store to ctx.stack: [receiver, ...args, method_name, arg_count]
        let total = 2 + arg_count + 1; // receiver + args + method + arg_count
        let base_sp = self.compile_time_sp;
        self.builder.ins().store(
            MemFlags::trusted(),
            receiver,
            self.ctx_ptr,
            STACK_OFFSET + (base_sp as i32) * 8,
        );
        for (i, arg) in args.iter().enumerate() {
            self.builder.ins().store(
                MemFlags::trusted(),
                *arg,
                self.ctx_ptr,
                STACK_OFFSET + ((base_sp + 1 + i) as i32) * 8,
            );
        }
        self.builder.ins().store(
            MemFlags::trusted(),
            method_val,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 1 + arg_count) as i32) * 8,
        );
        self.builder.ins().store(
            MemFlags::trusted(),
            arg_count_val,
            self.ctx_ptr,
            STACK_OFFSET + ((base_sp + 2 + arg_count) as i32) * 8,
        );

        // Update ctx.stack_ptr
        let new_sp = self
            .builder
            .ins()
            .iconst(types::I64, (base_sp + total) as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_sp, self.ctx_ptr, STACK_PTR_OFFSET);

        // Call jit_call_method
        let count_val = self.builder.ins().iconst(types::I64, total as i64);
        let inst = self
            .builder
            .ins()
            .call(self.ffi.call_method, &[self.ctx_ptr, count_val]);

        // Restore ctx.stack_ptr to base
        let restore_sp = self.builder.ins().iconst(types::I64, base_sp as i64);
        self.builder.ins().store(
            MemFlags::trusted(),
            restore_sp,
            self.ctx_ptr,
            STACK_PTR_OFFSET,
        );

        self.builder.inst_results(inst)[0]
    }

    /// Inline a small leaf function at the call site.
    ///
    /// Instead of emitting a `call` instruction, we compile the callee's
    /// bytecode directly into the caller's IR with remapped local variables.
    /// This eliminates:
    /// - ctx.locals save/restore overhead
    /// - The call instruction itself (pipeline flush, branch predictor miss)
    /// - Result load from ctx.stack[0]
    ///
    /// Only used for straight-line leaf functions (no calls, no branches).
    fn compile_inline_call(
        &mut self,
        callee_fn_id: u16,
        candidate: InlineCandidate,
        arg_count: usize,
        call_site_idx: usize,
    ) -> Result<(), String> {
        // 1. Pop args from SSA stack (they come in reverse order)
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if let Some(val) = self.stack_pop() {
                args.push(val);
            }
        }
        args.reverse(); // Restore original parameter order

        // 2. Snapshot caller's state for multi-frame deopt before changing inline_depth.
        // Capture all current-frame locals (at current inline_local_base).
        let caller_locals_snapshot: Vec<(u16, Variable)> = self
            .locals
            .iter()
            .filter(|(idx, _)| {
                if self.inline_local_base > 0 {
                    **idx >= self.inline_local_base && **idx < self.inline_local_base + 128
                } else {
                    **idx < 128
                }
            })
            .map(|(idx, var)| {
                let bc_idx = idx.wrapping_sub(self.inline_local_base);
                (bc_idx, *var)
            })
            .collect();
        let caller_local_kinds: Vec<SlotKind> = caller_locals_snapshot
            .iter()
            .map(|&(bc_idx, _)| {
                if self.unboxed_int_locals.contains(&bc_idx) {
                    SlotKind::Int64
                } else if self.unboxed_f64_locals.contains(&bc_idx) {
                    SlotKind::Float64
                } else {
                    SlotKind::NanBoxed
                }
            })
            .collect();
        // The caller's function ID: at depth 0 it's the physical function being compiled;
        // at deeper depths it's the previous inlined callee (now acting as caller).
        let caller_function_id = if self.inline_depth == 0 {
            self.compiling_function_id
        } else {
            // The last frame on the stack was pushed when entering the current inline level.
            // Its callee_fn_id (which was the current function) is the caller for the next level.
            self.inline_frame_stack
                .last()
                .map(|ctx| ctx.callee_fn_id)
                .unwrap_or(self.compiling_function_id)
        };
        self.inline_frame_stack.push(InlineFrameContext {
            function_id: caller_function_id,
            callee_fn_id,
            call_site_ip: call_site_idx,
            locals_snapshot: caller_locals_snapshot,
            local_kinds: caller_local_kinds,
            stack_depth: self.stack_depth,
            f64_locals: self.unboxed_f64_locals.clone(),
            int_locals: self.unboxed_int_locals.clone(),
        });

        // 3. Set up inline local namespace to avoid caller/callee collisions
        let prev_local_base = self.inline_local_base;
        self.inline_local_base = prev_local_base.wrapping_add(10_000);
        self.inline_depth += 1;

        // 3. Save caller's local_types that might be overwritten
        let mut saved_local_types: HashMap<u16, StorageHint> = HashMap::new();
        for i in 0..candidate.locals_count {
            if let Some(&hint) = self.local_types.get(&i) {
                saved_local_types.insert(i, hint);
            }
        }

        // 4. Define callee args as locals in the inline namespace
        for (i, arg_val) in args.into_iter().enumerate() {
            let var = self.get_or_create_local(i as u16);
            self.builder.def_var(var, arg_val);
        }

        // Initialize remaining locals to null
        for i in arg_count as u16..candidate.locals_count {
            let var = self.get_or_create_local(i);
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.def_var(var, null_val);
        }

        // 5. Create inline return merge block
        //    Return/ReturnValue in the callee will jump here instead of the caller's exit
        let inline_return_block = self.builder.create_block();
        self.builder
            .append_block_param(inline_return_block, types::I64);
        let saved_exit = self.exit_block;
        self.exit_block = Some(inline_return_block);

        // 6. Compile callee instructions inline (straight-line, no blocks needed)
        let start = candidate.entry_point;
        let end = start + candidate.instruction_count;
        let callee_instrs: Vec<_> = self.program.instructions[start..end].to_vec();

        let mut had_return = false;
        for (rel_i, instr) in callee_instrs.iter().enumerate() {
            self.current_instr_idx = start + rel_i;
            self.compile_instruction(instr, start + rel_i)?;
            // ReturnValue/Return will jump to our inline_return_block
            if matches!(instr.opcode, OpCode::Return | OpCode::ReturnValue) {
                had_return = true;
                break;
            }
        }

        // If callee body didn't have Return/ReturnValue, the current block
        // is unterminated. Add an implicit return null to prevent Cranelift
        // panic on switch_to_block.
        if !had_return {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.ins().jump(inline_return_block, &[null_val]);
        }

        // 7. Restore compiler state
        self.exit_block = saved_exit;
        self.inline_local_base = prev_local_base;
        self.inline_depth -= 1;
        self.inline_frame_stack.pop();

        // Restore caller's local_types
        for i in 0..candidate.locals_count {
            self.local_types.remove(&i);
        }
        for (idx, hint) in saved_local_types {
            self.local_types.insert(idx, hint);
        }

        // 8. Switch to the return block and push the inlined result
        self.builder.switch_to_block(inline_return_block);
        self.builder.seal_block(inline_return_block);
        let result = self.builder.block_params(inline_return_block)[0];
        self.stack_push(result);

        Ok(())
    }

    pub(crate) fn compile_return(&mut self) -> Result<(), String> {
        if let Some(exit_block) = self.exit_block {
            let default_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.ins().jump(exit_block, &[default_val]);
        }
        Ok(())
    }

    pub(crate) fn compile_return_value(&mut self) -> Result<(), String> {
        if let Some(exit_block) = self.exit_block {
            let ret_val = self
                .stack_pop_boxed()
                .unwrap_or_else(|| self.builder.ins().iconst(types::I64, TAG_NULL as i64));
            self.builder.ins().jump(exit_block, &[ret_val]);
        }
        Ok(())
    }
}
