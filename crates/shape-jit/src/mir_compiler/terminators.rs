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

                // Check if the condition is a native I8 bool or NaN-boxed I64.
                let cond_type = self.builder.func.dfg.value_type(cond_val);
                let is_true = if cond_type == types::I8 {
                    // Native bool: I8 value, 0 = false, nonzero = true.
                    cond_val
                } else {
                    // NaN-boxed truthiness: a value is falsy if it is
                    // TAG_NULL, TAG_NONE, TAG_BOOL_FALSE, or 0.
                    // Everything else (numbers, strings, arrays, objects,
                    // TAG_BOOL_TRUE) is truthy.
                    //
                    // We check: value != TAG_NULL && value != TAG_NONE
                    //        && value != TAG_BOOL_FALSE && value != 0
                    let tag_null = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    let tag_none = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NONE as i64);
                    let tag_false = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_BOOL_FALSE as i64);
                    let zero = self.builder.ins().iconst(types::I64, 0i64);
                    let not_null = self
                        .builder
                        .ins()
                        .icmp(IntCC::NotEqual, cond_val, tag_null);
                    let not_none = self
                        .builder
                        .ins()
                        .icmp(IntCC::NotEqual, cond_val, tag_none);
                    let not_false = self
                        .builder
                        .ins()
                        .icmp(IntCC::NotEqual, cond_val, tag_false);
                    let not_zero = self
                        .builder
                        .ins()
                        .icmp(IntCC::NotEqual, cond_val, zero);
                    let t1 = self.builder.ins().band(not_null, not_none);
                    let t2 = self.builder.ins().band(t1, not_false);
                    self.builder.ins().band(t2, not_zero)
                };

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
                // ── METHOD CALL PATH ─────────────────────────────────────
                // Method calls use MirConstant::Method(name). The MIR args
                // are [receiver, arg0, arg1, ...]. We need to push them to
                // ctx.stack in the format jit_call_method expects:
                //   [receiver, arg0, ..., method_name_string, arg_count_number]
                // then call jit_call_method(ctx, total_count).
                if let Operand::Constant(MirConstant::Method(method_name)) = func {
                    let stack_base_offset = crate::context::STACK_OFFSET as i32;
                    let sp_offset = crate::context::STACK_PTR_OFFSET as i32;

                    let old_sp = self.builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        self.ctx_ptr,
                        sp_offset,
                    );

                    // args[0] = receiver, args[1..] = actual method arguments
                    // Push all args (receiver first, then method args) NaN-boxed
                    for (i, arg) in args.iter().enumerate() {
                        let val = self.compile_operand(arg)?;
                        let boxed = self.ensure_nanboxed(val);
                        let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
                        let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                        let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base_offset as i64);
                        let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                        self.builder.ins().store(MemFlags::new(), boxed, store_addr, 0);
                    }

                    // Push method_name as NaN-boxed string
                    let method_str_bits = crate::nan_boxing::box_string(method_name.clone());
                    let method_val = self.builder.ins().iconst(types::I64, method_str_bits as i64);
                    let method_slot_idx = self.builder.ins().iadd_imm(old_sp, args.len() as i64);
                    let method_byte_off = self.builder.ins().ishl_imm(method_slot_idx, 3);
                    let method_abs_off = self.builder.ins().iadd_imm(method_byte_off, stack_base_offset as i64);
                    let method_addr = self.builder.ins().iadd(self.ctx_ptr, method_abs_off);
                    self.builder.ins().store(MemFlags::new(), method_val, method_addr, 0);

                    // Push arg_count as NaN-boxed number (count of actual args, NOT receiver)
                    let actual_arg_count = if args.is_empty() { 0 } else { args.len() - 1 };
                    let argc_bits = crate::nan_boxing::box_number(actual_arg_count as f64);
                    let argc_val = self.builder.ins().iconst(types::I64, argc_bits as i64);
                    let argc_slot_idx = self.builder.ins().iadd_imm(old_sp, (args.len() + 1) as i64);
                    let argc_byte_off = self.builder.ins().ishl_imm(argc_slot_idx, 3);
                    let argc_abs_off = self.builder.ins().iadd_imm(argc_byte_off, stack_base_offset as i64);
                    let argc_addr = self.builder.ins().iadd(self.ctx_ptr, argc_abs_off);
                    self.builder.ins().store(MemFlags::new(), argc_val, argc_addr, 0);

                    // Update stack_ptr: receiver + args + method_name + arg_count
                    let total_items = args.len() + 2; // args (including receiver) + method_name + arg_count
                    let new_sp = self.builder.ins().iadd_imm(old_sp, total_items as i64);
                    self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                    // Call jit_call_method(ctx, total_count)
                    let count_val = self.builder.ins().iconst(types::I64, total_items as i64);
                    let inst = self.builder.ins().call(
                        self.ffi.call_method,
                        &[self.ctx_ptr, count_val],
                    );
                    let result = self.builder.inst_results(inst)[0];

                    // Restore stack_ptr to old value
                    self.builder.ins().store(MemFlags::new(), old_sp, self.ctx_ptr, sp_offset);

                    // Store result to destination
                    self.release_old_value_if_heap(destination)?;
                    self.write_place(destination, result)?;

                    // Reload locals that may have been mutated via references
                    self.reload_referenced_locals();

                    // Jump to continuation block
                    let next_block = self.block_map.get(next).ok_or_else(|| {
                        format!("MirToIR: unknown call continuation block {}", next)
                    })?;
                    self.builder.ins().jump(*next_block, &[]);
                    return Ok(());
                }

                // ── BUILTIN FUNCTION PATH ─────────────────────────────
                // Known builtins like "print" that aren't user functions.
                // Dispatch directly to the FFI implementation.
                if let Operand::Constant(MirConstant::Function(name)) = func {
                    if name == "print" && self.function_indices.get(name.as_str()).is_none() {
                        // print(value) → jit_print(nanboxed_value)
                        let val = if args.is_empty() {
                            self.builder.ins().iconst(types::I64, crate::nan_boxing::TAG_NULL as i64)
                        } else {
                            let raw = self.compile_operand(&args[0])?;
                            self.ensure_nanboxed(raw)
                        };
                        self.builder.ins().call(self.ffi.print, &[val]);

                        // print returns None
                        let none_val = self.builder.ins().iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                        self.release_old_value_if_heap(destination)?;
                        self.write_place(destination, none_val)?;
                        self.reload_referenced_locals();
                        let next_block = self.block_map.get(next).ok_or_else(|| {
                            format!("MirToIR: unknown call continuation block {}", next)
                        })?;
                        self.builder.ins().jump(*next_block, &[]);
                        return Ok(());
                    }
                }

                // ── ENUM CONSTRUCTOR PATH ─────────────────────────────
                // Qualified function calls like "Shape::Circle" that don't
                // exist in the function index are enum variant constructors.
                // Enums are represented as arrays in the JIT, so we just
                // create an array from the constructor arguments.
                if let Operand::Constant(MirConstant::Function(name)) = func {
                    if name.contains("::") && self.function_indices.get(name.as_str()).is_none() {
                        // Create array from args (enum payload)
                        let zero = self.builder.ins().iconst(types::I64, 0i64);
                        let inst = self.builder.ins().call(
                            self.ffi.new_array,
                            &[self.ctx_ptr, zero],
                        );
                        let mut arr = self.builder.inst_results(inst)[0];

                        for arg in args.iter() {
                            let raw = self.compile_operand(arg)?;
                            let val = self.ensure_nanboxed(raw);
                            let inst = self.builder.ins().call(
                                self.ffi.array_push_elem,
                                &[arr, val],
                            );
                            arr = self.builder.inst_results(inst)[0];
                        }

                        // Store result to destination
                        self.release_old_value_if_heap(destination)?;
                        self.write_place(destination, arr)?;

                        // Jump to continuation block
                        let next_block = self.block_map.get(next).ok_or_else(|| {
                            format!("MirToIR: unknown call continuation block {}", next)
                        })?;
                        self.builder.ins().jump(*next_block, &[]);
                        return Ok(());
                    }
                }

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

                // Check if we have a direct FuncRef for the callee.
                let func_ref = func_id.and_then(|fid| self.user_func_refs.get(&fid).copied());

                let result = if let Some(func_ref) = func_ref {
                    // ── DIRECT CALL PATH ──────────────────────────────────
                    // Compile args as SSA values and pass as native Cranelift params.
                    // ABI: fn(ctx_ptr, arg0, arg1, ..., argN) -> i32
                    // The callee stores its return value to ctx.stack[0].
                    // Args are NaN-boxed since the callee ABI uses I64 params.
                    let mut arg_vals = Vec::with_capacity(args.len() + 1);
                    arg_vals.push(self.ctx_ptr);
                    for arg in args.iter() {
                        let val = self.compile_operand(arg)?;
                        let boxed = self.ensure_nanboxed(val);
                        arg_vals.push(boxed);
                    }

                    let inst = self.builder.ins().call(func_ref, &arg_vals);
                    let signal = self.builder.inst_results(inst)[0];

                    // Deopt: if signal < 0, the callee encountered an error.
                    // Propagate by returning the negative signal immediately.
                    let zero = self.builder.ins().iconst(types::I32, 0);
                    let is_error =
                        self.builder
                            .ins()
                            .icmp(IntCC::SignedLessThan, signal, zero);
                    let deopt_block = self.builder.create_block();
                    let continue_block = self.builder.create_block();
                    self.builder
                        .ins()
                        .brif(is_error, deopt_block, &[], continue_block, &[]);

                    // Deopt block: return the error signal.
                    self.builder.switch_to_block(deopt_block);
                    self.builder.seal_block(deopt_block);
                    self.builder.ins().return_(&[signal]);

                    // Continue block: read return value from ctx.stack[0].
                    self.builder.switch_to_block(continue_block);
                    self.builder.seal_block(continue_block);
                    let stack_offset = crate::context::STACK_OFFSET as i32;
                    self.builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        self.ctx_ptr,
                        stack_offset,
                    )
                } else {
                    // ── INDIRECT CALL (closures/first-class functions) ────
                    let stack_base_offset = crate::context::STACK_OFFSET as i32;
                    let sp_offset = crate::context::STACK_PTR_OFFSET as i32;

                    let old_sp = self.builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        self.ctx_ptr,
                        sp_offset,
                    );

                    // Push callee onto stack first (NaN-boxed)
                    let callee_val = self.compile_operand(func)?;
                    let callee_boxed = self.ensure_nanboxed(callee_val);
                    let callee_slot_idx = old_sp;
                    let callee_byte_off = self.builder.ins().ishl_imm(callee_slot_idx, 3);
                    let callee_abs_off = self.builder.ins().iadd_imm(callee_byte_off, stack_base_offset as i64);
                    let callee_addr = self.builder.ins().iadd(self.ctx_ptr, callee_abs_off);
                    self.builder.ins().store(MemFlags::new(), callee_boxed, callee_addr, 0);

                    // Push args after callee (NaN-boxed)
                    for (i, arg) in args.iter().enumerate() {
                        let val = self.compile_operand(arg)?;
                        let boxed = self.ensure_nanboxed(val);
                        let slot_idx = self.builder.ins().iadd_imm(old_sp, (i + 1) as i64);
                        let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                        let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base_offset as i64);
                        let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                        self.builder.ins().store(MemFlags::new(), boxed, store_addr, 0);
                    }

                    // Push arg_count as NaN-boxed number
                    let total_items = 1 + args.len() + 1; // callee + args + arg_count
                    let argc_slot_idx = self.builder.ins().iadd_imm(old_sp, (1 + args.len()) as i64);
                    let argc_byte_off = self.builder.ins().ishl_imm(argc_slot_idx, 3);
                    let argc_abs_off = self.builder.ins().iadd_imm(argc_byte_off, stack_base_offset as i64);
                    let argc_addr = self.builder.ins().iadd(self.ctx_ptr, argc_abs_off);
                    let argc_val = self.builder.ins().iconst(types::I64,
                        crate::nan_boxing::box_number(args.len() as f64) as i64);
                    self.builder.ins().store(MemFlags::new(), argc_val, argc_addr, 0);

                    // Update stack_ptr
                    let new_sp = self.builder.ins().iadd_imm(old_sp, total_items as i64);
                    self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                    // jit_call_value reads callee + args + arg_count from stack
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
                // ctx.stack stores NaN-boxed I64 values, so box if native-typed.
                let return_slot = SlotId(0);
                if let Some(&var) = self.locals.get(&return_slot) {
                    let ret_val = self.builder.use_var(var);
                    // Ensure the return value is NaN-boxed I64 for ctx.stack.
                    let boxed = self.ensure_nanboxed(ret_val);
                    // Store to ctx.stack[0]
                    let stack_offset = crate::context::STACK_OFFSET as i32;
                    self.builder.ins().store(
                        MemFlags::new(),
                        boxed,
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
