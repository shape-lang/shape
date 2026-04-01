//! MIR Terminator → Cranelift IR compilation.
//!
//! Terminators end basic blocks: Goto (jump), SwitchBool (branch),
//! Call (function call), Return, Unreachable (trap).

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile a MIR terminator.
    pub(crate) fn compile_terminator(
        &mut self,
        terminator: &Terminator,
    ) -> Result<(), String> {
        match &terminator.kind {
            TerminatorKind::Goto(target) => {
                let target_block = self.block_map.get(target).ok_or_else(|| {
                    format!("MirToIR: unknown block target {}", target)
                })?;
                self.builder.ins().jump(*target_block, &[]);
                Ok(())
            }

            TerminatorKind::SwitchBool {
                operand,
                true_bb,
                false_bb,
            } => {
                let cond_val = self.compile_operand(operand)?;

                let true_block = self.block_map.get(true_bb).ok_or_else(|| {
                    format!("MirToIR: unknown true block {}", true_bb)
                })?;
                let false_block = self.block_map.get(false_bb).ok_or_else(|| {
                    format!("MirToIR: unknown false block {}", false_bb)
                })?;

                // NaN-boxed booleans: TAG_TRUE vs anything else.
                // Compare against TAG_TRUE to get a boolean condition.
                let tag_true = self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_BOOL_TRUE as i64);
                let is_true =
                    self.builder
                        .ins()
                        .icmp(IntCC::Equal, cond_val, tag_true);

                self.builder
                    .ins()
                    .brif(is_true, *true_block, &[], *false_block, &[]);
                Ok(())
            }

            TerminatorKind::Call {
                func,
                args,
                destination,
                next,
            } => {
                // Resolve function ID from the func operand.
                // Direct calls use MirConstant::Function(name) → look up index.
                // Indirect calls (closures/first-class functions) fall back to
                // jit_call_value which reads the callee from the stack.
                let func_id: Option<u16> = match func {
                    Operand::Constant(MirConstant::Function(name)) => {
                        self.function_indices.get(name.as_str()).copied()
                    }
                    _ => None,
                };

                // 1. Compile and store each argument to ctx.stack[stack_ptr + i]
                let stack_base_offset = crate::context::STACK_OFFSET as i32;
                let sp_offset = crate::context::STACK_PTR_OFFSET as i32;

                // Load current stack_ptr
                let old_sp = self.builder.ins().load(
                    types::I64,
                    MemFlags::new(),
                    self.ctx_ptr,
                    sp_offset,
                );

                for (i, arg) in args.iter().enumerate() {
                    let val = self.compile_operand(arg)?;
                    // ctx.stack[stack_ptr + i]: byte offset = STACK_OFFSET + (sp + i) * 8
                    let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
                    let byte_off = self.builder.ins().ishl_imm(slot_idx, 3); // * 8
                    let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base_offset as i64);
                    // Store to ctx + abs_off
                    let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                    self.builder.ins().store(MemFlags::new(), val, store_addr, 0);
                }

                // 2. Update ctx.stack_ptr += arg_count
                let new_sp = self.builder.ins().iadd_imm(old_sp, args.len() as i64);
                self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                // 3. Call the function
                let result = if let Some(fid) = func_id {
                    // Direct call: jit_call_function(ctx, function_id, null, arg_count)
                    let func_id_val = self.builder.ins().iconst(types::I16, fid as i64);
                    let null_ptr = self.builder.ins().iconst(types::I64, 0);
                    let argc = self.builder.ins().iconst(types::I64, args.len() as i64);
                    let inst = self.builder.ins().call(
                        self.ffi.call_function,
                        &[self.ctx_ptr, func_id_val, null_ptr, argc],
                    );
                    self.builder.inst_results(inst)[0]
                } else {
                    // Indirect call: push callee value onto stack, then call_value
                    let callee_val = self.compile_operand(func)?;
                    // Store callee at stack[new_sp] (after the args)
                    let callee_slot = self.builder.ins().ishl_imm(new_sp, 3);
                    let callee_off = self.builder.ins().iadd_imm(callee_slot, stack_base_offset as i64);
                    let callee_addr = self.builder.ins().iadd(self.ctx_ptr, callee_off);
                    self.builder.ins().store(MemFlags::new(), callee_val, callee_addr, 0);
                    // Update stack_ptr to include callee
                    let sp_with_callee = self.builder.ins().iadd_imm(new_sp, 1);
                    self.builder.ins().store(MemFlags::new(), sp_with_callee, self.ctx_ptr, sp_offset);
                    // call_value reads callee + args from stack
                    let inst = self.builder.ins().call(
                        self.ffi.call_value,
                        &[self.ctx_ptr],
                    );
                    self.builder.inst_results(inst)[0]
                };

                // 4. Store result to destination
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result)?;

                // 4b. Reload locals that may have been mutated via references
                self.reload_referenced_locals();

                // 5. Jump to continuation block
                let next_block = self.block_map.get(next).ok_or_else(|| {
                    format!("MirToIR: unknown call continuation block {}", next)
                })?;
                self.builder.ins().jump(*next_block, &[]);
                Ok(())
            }

            TerminatorKind::Return => {
                // MIR's Return is preceded by Drop statements for all live locals
                // (the MIR lowering pass already inserts these). We just need to
                // emit the actual return instruction.
                //
                // The return value is in the dedicated return slot (slot 0 by convention).
                // Write it to ctx.stack[0] for the JIT calling convention.
                let return_slot = SlotId(0);
                if let Some(&var) = self.locals.get(&return_slot) {
                    let ret_val = self.builder.use_var(var);
                    // Store to ctx.stack[0]
                    let stack_offset = crate::context::STACK_OFFSET as i32;
                    self.builder.ins().store(
                        MemFlags::new(),
                        ret_val,
                        self.ctx_ptr,
                        stack_offset,
                    );
                    // Set stack_ptr to 1
                    let one = self.builder.ins().iconst(types::I64, 1);
                    let sp_offset = crate::context::STACK_PTR_OFFSET as i32;
                    self.builder
                        .ins()
                        .store(MemFlags::new(), one, self.ctx_ptr, sp_offset);
                }

                let signal = self.builder.ins().iconst(types::I32, 0);
                self.builder.ins().return_(&[signal]);
                Ok(())
            }

            TerminatorKind::Unreachable => {
                self.builder.ins().trap(TrapCode::User(0));
                Ok(())
            }
        }
    }
}
