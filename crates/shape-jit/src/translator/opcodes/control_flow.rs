//! Control flow: jumps, loops, iterators, exception handling

use crate::nan_boxing::*;
use cranelift::prelude::*;
use shape_vm::bytecode::{Instruction, Operand};

use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    fn read_local_for_unroll_cmp(&mut self, local_slot: u16) -> Value {
        if self.unboxed_int_locals.contains(&local_slot) {
            let var = self.get_or_create_local(local_slot);
            return self.builder.use_var(var);
        }

        if self.unboxed_f64_locals.contains(&local_slot)
            && let Some(&f64_var) = self.f64_local_vars.get(&local_slot)
        {
            let f64_val = self.builder.use_var(f64_var);
            return self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val);
        }

        let var = self.get_or_create_local(local_slot);
        let boxed = self.builder.use_var(var);
        let as_f64 = self.i64_to_f64(boxed);
        self.builder.ins().fcvt_to_sint_sat(types::I64, as_f64)
    }

    // Jump operations
    pub(crate) fn compile_jump(&mut self, instr: &Instruction, idx: usize) -> Result<(), String> {
        if let Some(Operand::Offset(offset)) = &instr.operand {
            let target_idx = ((idx as i32) + 1 + *offset) as usize;
            let prev = self
                .block_stack_depth
                .entry(target_idx)
                .or_insert(self.stack_depth);
            debug_assert_eq!(
                *prev, self.stack_depth,
                "Block {} depth mismatch: first predecessor={}, this predecessor={}",
                target_idx, *prev, self.stack_depth
            );

            // Check for unrollable back-edge: backward jump with pending unroll info
            if target_idx < idx {
                if let Some(unroll) = self.pending_unroll.take() {
                    if let Some(&target_block) = self.blocks.get(&target_idx) {
                        let end_block = self.loop_stack.last().map(|ctx| ctx.end_block).unwrap();

                        // Emit additional body copies based on planned unroll factor.
                        // Between copies: check if IV still in bounds
                        for _ in 0..(unroll.factor.saturating_sub(1) as usize) {
                            let iv_val = self.read_local_for_unroll_cmp(unroll.iv_slot);
                            let bound_val = self.read_local_for_unroll_cmp(unroll.bound_slot);
                            let cmp = self.builder.ins().icmp(unroll.bound_cmp, iv_val, bound_val);

                            let copy_block = self.builder.create_block();
                            self.builder
                                .ins()
                                .brif(cmp, copy_block, &[], end_block, &[]);
                            self.builder.switch_to_block(copy_block);
                            self.builder.seal_block(copy_block);

                            // Re-compile body instructions for this unrolled copy
                            let program = self.program;
                            for body_idx in unroll.body_start..unroll.body_end {
                                let body_instr = &program.instructions[body_idx];
                                self.current_instr_idx = body_idx;
                                self.compile_instruction(body_instr, body_idx)?;
                            }
                        }

                        // After cloned copies, jump to header for next group.
                        self.emit_jump_to_target(target_idx, target_block);
                    }
                    return Ok(());
                }
            }

            if let Some(&target_block) = self.blocks.get(&target_idx) {
                self.emit_jump_to_target(target_idx, target_block);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_jump_if_false(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        if let Some(Operand::Offset(offset)) = &instr.operand {
            // Check if the top of stack is a known Bool type (from a comparison).
            // If so, we can use a single comparison instead of the 3-way falsy check.
            use shape_vm::type_tracking::StorageHint;
            let is_known_bool = self.peek_stack_type() == StorageHint::Bool;

            // Check for raw comparison result from typed_comparison (fused cmp-branch)
            let raw_cmp_opt = self.typed_stack.peek().and_then(|tv| tv.raw_cmp);

            if let Some(cond_boxed) = self.stack_pop() {
                let target_idx = ((idx as i32) + 1 + *offset) as usize;
                let next_idx = idx + 1;

                let prev = self
                    .block_stack_depth
                    .entry(target_idx)
                    .or_insert(self.stack_depth);
                debug_assert_eq!(
                    *prev, self.stack_depth,
                    "Block {} depth mismatch: first predecessor={}, this predecessor={}",
                    target_idx, *prev, self.stack_depth
                );
                let prev = self
                    .block_stack_depth
                    .entry(next_idx)
                    .or_insert(self.stack_depth);
                debug_assert_eq!(
                    *prev, self.stack_depth,
                    "Block {} depth mismatch: first predecessor={}, this predecessor={}",
                    next_idx, *prev, self.stack_depth
                );

                if let (Some(&target_block), Some(&next_block)) =
                    (self.blocks.get(&target_idx), self.blocks.get(&next_idx))
                {
                    let target_is_merge = self.merge_blocks.contains(&target_idx);
                    let next_is_merge = self.merge_blocks.contains(&next_idx);

                    if let Some(raw_cmp) = raw_cmp_opt {
                        // Fused comparison-branch: use raw i1 fcmp directly
                        // raw_cmp is true when condition holds
                        // JumpIfFalse: jump to target when condition is FALSE
                        // So: if raw_cmp is true -> next_block, if false -> target_block
                        if target_is_merge || next_is_merge {
                            let val = self.stack_peek().unwrap_or_else(|| {
                                self.builder.ins().iconst(types::I64, TAG_NULL as i64)
                            });
                            let target_args = if target_is_merge { vec![val] } else { vec![] };
                            let next_args = if next_is_merge { vec![val] } else { vec![] };
                            self.builder.ins().brif(
                                raw_cmp,
                                next_block,
                                &next_args,
                                target_block,
                                &target_args,
                            );
                        } else {
                            self.builder
                                .ins()
                                .brif(raw_cmp, next_block, &[], target_block, &[]);
                        }
                    } else {
                        // Existing path: compute is_falsy from boxed boolean
                        let is_falsy = if is_known_bool {
                            let false_tag =
                                self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                            self.builder.ins().icmp(IntCC::Equal, cond_boxed, false_tag)
                        } else {
                            let false_tag =
                                self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                            let null_tag = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                            let zero_bits = self
                                .builder
                                .ins()
                                .iconst(types::I64, box_number(0.0) as i64);

                            let is_bool_false =
                                self.builder.ins().icmp(IntCC::Equal, cond_boxed, false_tag);
                            let is_null =
                                self.builder.ins().icmp(IntCC::Equal, cond_boxed, null_tag);
                            let is_zero =
                                self.builder.ins().icmp(IntCC::Equal, cond_boxed, zero_bits);

                            let falsy1 = self.builder.ins().bor(is_bool_false, is_null);
                            self.builder.ins().bor(falsy1, is_zero)
                        };

                        if target_is_merge || next_is_merge {
                            let val = self.stack_peek().unwrap_or_else(|| {
                                self.builder.ins().iconst(types::I64, TAG_NULL as i64)
                            });
                            let target_args = if target_is_merge { vec![val] } else { vec![] };
                            let next_args = if next_is_merge { vec![val] } else { vec![] };
                            self.builder.ins().brif(
                                is_falsy,
                                target_block,
                                &target_args,
                                next_block,
                                &next_args,
                            );
                        } else {
                            self.builder
                                .ins()
                                .brif(is_falsy, target_block, &[], next_block, &[]);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_jump_if_true(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        if let Some(Operand::Offset(offset)) = &instr.operand {
            // Check if the top of stack is a known Bool type (from a comparison).
            use shape_vm::type_tracking::StorageHint;
            let is_known_bool = self.peek_stack_type() == StorageHint::Bool;

            // Check for raw comparison result from typed_comparison (fused cmp-branch)
            let raw_cmp_opt = self.typed_stack.peek().and_then(|tv| tv.raw_cmp);

            if let Some(cond_boxed) = self.stack_pop() {
                let target_idx = ((idx as i32) + 1 + *offset) as usize;
                let next_idx = idx + 1;

                let prev = self
                    .block_stack_depth
                    .entry(target_idx)
                    .or_insert(self.stack_depth);
                debug_assert_eq!(
                    *prev, self.stack_depth,
                    "Block {} depth mismatch: first predecessor={}, this predecessor={}",
                    target_idx, *prev, self.stack_depth
                );
                let prev = self
                    .block_stack_depth
                    .entry(next_idx)
                    .or_insert(self.stack_depth);
                debug_assert_eq!(
                    *prev, self.stack_depth,
                    "Block {} depth mismatch: first predecessor={}, this predecessor={}",
                    next_idx, *prev, self.stack_depth
                );

                if let (Some(&target_block), Some(&next_block)) =
                    (self.blocks.get(&target_idx), self.blocks.get(&next_idx))
                {
                    let target_is_merge = self.merge_blocks.contains(&target_idx);
                    let next_is_merge = self.merge_blocks.contains(&next_idx);

                    if let Some(raw_cmp) = raw_cmp_opt {
                        // Fused comparison-branch: use raw i1 fcmp directly
                        // JumpIfTrue: jump to target when condition is TRUE
                        // raw_cmp is true when condition holds -> jump to target_block
                        if target_is_merge || next_is_merge {
                            let val = self.stack_peek().unwrap_or_else(|| {
                                self.builder.ins().iconst(types::I64, TAG_NULL as i64)
                            });
                            let target_args = if target_is_merge { vec![val] } else { vec![] };
                            let next_args = if next_is_merge { vec![val] } else { vec![] };
                            self.builder.ins().brif(
                                raw_cmp,
                                target_block,
                                &target_args,
                                next_block,
                                &next_args,
                            );
                        } else {
                            self.builder
                                .ins()
                                .brif(raw_cmp, target_block, &[], next_block, &[]);
                        }
                    } else {
                        // Existing path: compute is_falsy from boxed boolean
                        let is_falsy = if is_known_bool {
                            let false_tag =
                                self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                            self.builder.ins().icmp(IntCC::Equal, cond_boxed, false_tag)
                        } else {
                            let false_tag =
                                self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                            let null_tag = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                            let zero_bits = self
                                .builder
                                .ins()
                                .iconst(types::I64, box_number(0.0) as i64);

                            let is_bool_false =
                                self.builder.ins().icmp(IntCC::Equal, cond_boxed, false_tag);
                            let is_null =
                                self.builder.ins().icmp(IntCC::Equal, cond_boxed, null_tag);
                            let is_zero =
                                self.builder.ins().icmp(IntCC::Equal, cond_boxed, zero_bits);

                            let falsy1 = self.builder.ins().bor(is_bool_false, is_null);
                            self.builder.ins().bor(falsy1, is_zero)
                        };

                        if target_is_merge || next_is_merge {
                            let val = self.stack_peek().unwrap_or_else(|| {
                                self.builder.ins().iconst(types::I64, TAG_NULL as i64)
                            });
                            let target_args = if target_is_merge { vec![val] } else { vec![] };
                            let next_args = if next_is_merge { vec![val] } else { vec![] };
                            self.builder.ins().brif(
                                is_falsy,
                                next_block,
                                &next_args,
                                target_block,
                                &target_args,
                            );
                        } else {
                            self.builder
                                .ins()
                                .brif(is_falsy, next_block, &[], target_block, &[]);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // Exception handling
    pub(crate) fn compile_setup_try(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        if let Some(Operand::Offset(offset)) = &instr.operand {
            let catch_idx = (idx as i32 + 1 + *offset) as usize;
            self.exception_handlers.push(catch_idx);
        }
        Ok(())
    }

    pub(crate) fn compile_pop_handler(&mut self) -> Result<(), String> {
        self.exception_handlers.pop();
        Ok(())
    }

    pub(crate) fn compile_throw(&mut self) -> Result<(), String> {
        use crate::context::{STACK_OFFSET, STACK_PTR_OFFSET};

        if let Some(error_val) = self.stack_pop() {
            // Peek at current handler WITHOUT popping - let PopHandler handle the pop
            // This keeps compile-time handlers in sync with runtime behavior
            if let Some(&catch_idx) = self.exception_handlers.last() {
                self.stack_push(error_val);
                self.materialize_to_stack(1);

                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.format_error, &[self.ctx_ptr]);
                let formatted_error = self.builder.inst_results(inst)[0];
                self.update_sp_after_ffi(1, 0);

                // Store formatted error on context stack for catch block
                let sp_addr = self
                    .builder
                    .ins()
                    .iadd_imm(self.ctx_ptr, STACK_PTR_OFFSET as i64);
                let sp_val = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::trusted(), sp_addr, 0);

                let stack_base = self
                    .builder
                    .ins()
                    .iadd_imm(self.ctx_ptr, STACK_OFFSET as i64);
                let elem_offset = self.builder.ins().ishl_imm(sp_val, 3);
                let write_addr = self.builder.ins().iadd(stack_base, elem_offset);

                self.builder
                    .ins()
                    .store(MemFlags::trusted(), formatted_error, write_addr, 0);

                let new_sp = self.builder.ins().iadd_imm(sp_val, 1);
                self.builder
                    .ins()
                    .store(MemFlags::trusted(), new_sp, sp_addr, 0);

                if let Some(&catch_block) = self.blocks.get(&catch_idx) {
                    self.block_stack_depth.insert(catch_idx, 0);
                    self.builder.ins().jump(catch_block, &[formatted_error]);
                } else if let Some(exit_block) = self.exit_block {
                    self.builder.ins().jump(exit_block, &[formatted_error]);
                }

                // After throw, subsequent instructions are dead code until we reach catch block
                // Create unreachable block to continue compilation without affecting real control flow
                let unreachable_block = self.builder.create_block();
                self.builder.switch_to_block(unreachable_block);
                self.builder.seal_block(unreachable_block);
            } else if let Some(exit_block) = self.exit_block {
                // No handler - format error and return it (not null!)
                self.stack_push(error_val);
                self.materialize_to_stack(1);

                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.format_error, &[self.ctx_ptr]);
                let formatted_error = self.builder.inst_results(inst)[0];
                self.update_sp_after_ffi(1, 0);

                self.builder.ins().jump(exit_block, &[formatted_error]);

                let unreachable_block = self.builder.create_block();
                self.builder.switch_to_block(unreachable_block);
                self.builder.seal_block(unreachable_block);
            }
        } else if let Some(exit_block) = self.exit_block {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.ins().jump(exit_block, &[null_val]);

            let unreachable_block = self.builder.create_block();
            self.builder.switch_to_block(unreachable_block);
            self.builder.seal_block(unreachable_block);
        }
        Ok(())
    }
}
