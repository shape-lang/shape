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

                // Convert condition to I8 bool based on its Cranelift type.
                let cond_type = self.builder.func.dfg.value_type(cond_val);
                let is_true = if cond_type == types::I8 {
                    // Native bool: 0 = false, nonzero = true.
                    cond_val
                } else if cond_type == types::F64 {
                    // Native F64: truthy if != 0.0 and not NaN
                    let zero = self.builder.ins().f64const(0.0);
                    // fcmp ordered-not-equal: false for NaN and 0.0
                    self.builder
                        .ins()
                        .fcmp(FloatCC::OrderedNotEqual, cond_val, zero)
                } else if cond_type == types::I32 {
                    // Native I32: truthy if != 0
                    let zero = self.builder.ins().iconst(types::I32, 0);
                    self.builder.ins().icmp(IntCC::NotEqual, cond_val, zero)
                } else {
                    // v2-boundary: I64 (NaN-boxed) truthiness check uses NaN-box tags
                    let tag_null = self
                        .builder
                        .ins()
                        .iconst(types::I64, 0i64);
                    let tag_none = self
                        .builder
                        .ins()
                        .iconst(types::I64, 0i64);
                    let tag_false = self
                        .builder
                        .ins()
                        .iconst(types::I64, 0i64);
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
                // ── v2 TYPED-ARRAY METHOD FAST PATH ──────────────────────
                // Intercept `arr.length()` / `arr.len()` / `arr.push(v)`
                // when the receiver is a v2 typed-array slot. Bypass
                // `jit_call_method` and emit inline FFI calls.
                if let Operand::Constant(MirConstant::Method(method_name)) = func {
                    if let Some(receiver_arg) = args.first() {
                        if let Some(receiver_place) = match receiver_arg {
                            Operand::Copy(p)
                            | Operand::Move(p)
                            | Operand::MoveExplicit(p) => Some(p.clone()),
                            _ => None,
                        } {
                            if let Some(elem_kind) =
                                self.v2_typed_array_elem_kind(&receiver_place)
                            {
                                if let Some(()) = self.try_emit_v2_array_method(
                                    method_name,
                                    &receiver_place,
                                    &args[1..],
                                    destination,
                                    elem_kind,
                                )? {
                                    let next_block = self.block_map.get(next).ok_or_else(
                                        || format!("MirToIR: unknown call continuation block {}", next),
                                    )?;
                                    self.builder.ins().jump(*next_block, &[]);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                // ── v2 TYPED HASHMAP<string, V> METHOD FAST PATH ─────────
                // Intercept `m.get(k)` / `m.set(k, v)` / `m.has(k)` /
                // `m.length()` when the receiver is a `HashMap<string, I64|F64>`
                // slot. Bypasses the generic method-dispatch trampoline.
                if let Operand::Constant(MirConstant::Method(method_name)) = func {
                    if let Some(receiver_arg) = args.first() {
                        if let Some(receiver_place) = match receiver_arg {
                            Operand::Copy(p)
                            | Operand::Move(p)
                            | Operand::MoveExplicit(p) => Some(p.clone()),
                            _ => None,
                        } {
                            if let Some(kinds) =
                                self.v2_typed_str_map_kinds(&receiver_place)
                            {
                                if let Some(()) = self.try_emit_v2_typed_map_method(
                                    method_name,
                                    &receiver_place,
                                    &args[1..],
                                    destination,
                                    kinds,
                                )? {
                                    let next_block = self.block_map.get(next).ok_or_else(
                                        || format!("MirToIR: unknown call continuation block {}", next),
                                    )?;
                                    self.builder.ins().jump(*next_block, &[]);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                // ── V3-S6c MONOMORPHIZED METHOD-CALL ROUTING ─────────────
                // PATH α-prime per supervisor 2026-05-15 ratification. If
                // this Method-call site was specialized at bytecode-compile
                // time (V3-S6b side-table populated by
                // `try_monomorphize_method_call` success — both the
                // type-only and closure-aware mirrors), bypass the
                // `jit_call_method` trampoline + the `handle_int_map`
                // ckpt3_surface path entirely. Emit a direct Cranelift
                // FuncRef call to `user_func_refs[specialized_idx]`,
                // mirroring the direct-Function-call codegen at lines
                // ~807-867 below.
                //
                // ABI alignment is structurally guaranteed by
                // `crates/shape-vm/src/compiler/monomorphization/substitution.rs:2247`
                // (`inline_closure_body_into_specialization` doc-comment):
                // the specialized function preserves the original
                // parameter list verbatim — closure params are kept (the
                // closure pointer sits unused after body inlining); no
                // capture hoisting (Phase D/E territory). So Method-call
                // args `[receiver, closure_obj, ...]` map 1:1 to the
                // specialized function's `(self, closure_param, ...)`
                // signature.
                //
                // ADR-006 §2.7.5 stamp-at-compile-time: the side-table
                // lookup at JIT codegen is COMPILE TIME (this terminator
                // pass), NEVER runtime — the routing decision is encoded
                // into emitted Cranelift IR, not a runtime tag-byte read.
                //
                // Bypasses the V3-S6b dual-consumer SIGSEGV class
                // (`concrete_types[slot] = Array(I64)` activates both
                // `parametric_method_return_kind_from_receiver` AND
                // `v2_typed_array_elem_kind` → handle_int_map ckpt3_surface)
                // because no consume-via-stamp is performed; the routing
                // is structural at the Call terminator.
                if let Operand::Constant(MirConstant::Method(_)) = func {
                    let key = (terminator.span, self.caller_function_id);
                    if let Some(&specialized_idx) =
                        self.monomorphized_method_call_sites.get(&key)
                    {
                        let func_ref_opt = self
                            .user_func_refs
                            .get(&(specialized_idx as u16))
                            .copied();
                        if let Some(func_ref) = func_ref_opt {
                            // Mirror of the direct-Function-call codegen at
                            // terminators.rs:807-867 below. ABI =
                            // fn(ctx_ptr, arg0, ..., argN) -> i32 deopt
                            // signal. Args widened to I64 uniformly per
                            // R4.2E.
                            let mut arg_vals = Vec::with_capacity(args.len() + 1);
                            arg_vals.push(self.ctx_ptr);
                            for arg in args.iter() {
                                let val = self.compile_operand(arg)?;
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let boxed = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder.ins().bitcast(
                                        types::I64,
                                        MemFlags::new(),
                                        val,
                                    )
                                } else if val_ty == types::I32 {
                                    self.builder.ins().sextend(types::I64, val)
                                } else if val_ty == types::I8 {
                                    self.builder.ins().uextend(types::I64, val)
                                } else if val_ty == types::I16 {
                                    self.builder.ins().sextend(types::I64, val)
                                } else {
                                    val
                                };
                                arg_vals.push(boxed);
                            }

                            let inst =
                                self.builder.ins().call(func_ref, &arg_vals);
                            let signal = self.builder.inst_results(inst)[0];

                            // Deopt: signal < 0 propagates by immediate
                            // return of the negative signal.
                            let zero =
                                self.builder.ins().iconst(types::I32, 0);
                            let is_error = self.builder.ins().icmp(
                                IntCC::SignedLessThan,
                                signal,
                                zero,
                            );
                            let deopt_block = self.builder.create_block();
                            let continue_block = self.builder.create_block();
                            self.builder.ins().brif(
                                is_error,
                                deopt_block,
                                &[],
                                continue_block,
                                &[],
                            );

                            self.builder.switch_to_block(deopt_block);
                            self.builder.seal_block(deopt_block);
                            self.builder.ins().return_(&[signal]);

                            self.builder.switch_to_block(continue_block);
                            self.builder.seal_block(continue_block);
                            let stack_offset =
                                crate::context::STACK_OFFSET as i32;
                            let result = self.builder.ins().load(
                                types::I64,
                                MemFlags::new(),
                                self.ctx_ptr,
                                stack_offset,
                            );

                            self.release_old_value_if_heap(destination)?;
                            self.write_place(destination, result)?;
                            self.reload_referenced_locals();

                            let next_block =
                                self.block_map.get(next).ok_or_else(|| {
                                    format!(
                                        "MirToIR: unknown call continuation \
                                         block {}",
                                        next
                                    )
                                })?;
                            self.builder.ins().jump(*next_block, &[]);
                            return Ok(());
                        }
                        // FuncRef miss: side-table had specialized_idx but
                        // user_func_refs lacks it (declaration race or
                        // sub_program-rebased index space). Fall through to
                        // the existing jit_call_method trampoline path
                        // below — preserves V3-S6b baseline behaviour.
                    }
                    // Side-table miss (not a monomorphized site, or this
                    // caller-function didn't specialize this call). Fall
                    // through to the existing jit_call_method trampoline
                    // path below.
                }

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

                    // args[0] = receiver, args[1..] = actual method arguments.
                    // ADR-006 §2.7.7 / Q9: every push into ctx.stack writes
                    // the producing-site kind into `stack_kinds` in lockstep.
                    // `jit_call_method` doesn't currently consume kinds (it
                    // dispatches by method name + receiver type via the
                    // §2.7.10/Q11 path), but the lockstep invariant
                    // requires the writes; the kind track at these slots
                    // surfaces a kind-source gap correctly if some future
                    // FFI reads them.
                    //
                    // R4.2E: VM-stack slots are 8-byte-wide I64 bit-patterns.
                    // Widen narrow Cranelift types inline (sextend/uextend/
                    // bitcast) so the method-dispatch trampoline reads a
                    // uniform u64 from ctx.stack[sp+i]. No NaN-boxing tagging
                    // is applied — raw bit-patterns only.
                    for (i, arg) in args.iter().enumerate() {
                        // Source kind for the parallel-kind track,
                        // falling back to the §2.7.5 carrier kind
                        // (`UInt64`) for opaque-source operands —
                        // NOT a Bool-default fallback.
                        let _ = i;
                        let arg_kind = self.operand_slot_kind_or_carrier(arg);

                        let val = self.compile_operand(arg)?;
                        let val_ty = self.builder.func.dfg.value_type(val);
                        let boxed = if val_ty == types::I64 {
                            val
                        } else if val_ty == types::F64 {
                            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
                        } else if val_ty == types::I32 {
                            self.builder.ins().sextend(types::I64, val)
                        } else if val_ty == types::I8 {
                            self.builder.ins().uextend(types::I64, val)
                        } else if val_ty == types::I16 {
                            self.builder.ins().sextend(types::I64, val)
                        } else {
                            val
                        };
                        let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
                        let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                        let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base_offset as i64);
                        let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                        self.builder.ins().store(MemFlags::new(), boxed, store_addr, 0);
                        // §2.7.7 / Q9 lockstep parallel-kind write.
                        self.emit_kind_track_write(slot_idx, arg_kind);
                    }

                    // v2-boundary: method name pushed as NaN-boxed string to ctx.stack
                    let method_str_bits = crate::ffi::value_ffi::box_string(method_name.clone());
                    let method_val = self.builder.ins().iconst(types::I64, method_str_bits as i64);
                    let method_slot_idx = self.builder.ins().iadd_imm(old_sp, args.len() as i64);
                    let method_byte_off = self.builder.ins().ishl_imm(method_slot_idx, 3);
                    let method_abs_off = self.builder.ins().iadd_imm(method_byte_off, stack_base_offset as i64);
                    let method_addr = self.builder.ins().iadd(self.ctx_ptr, method_abs_off);
                    self.builder.ins().store(MemFlags::new(), method_val, method_addr, 0);
                    // Method name is a heap String — kind = `NativeKind::String`.
                    self.emit_kind_track_write(
                        method_slot_idx,
                        shape_value::NativeKind::String,
                    );

                    // v2-boundary: arg_count pushed as raw i64 to ctx.stack.
                    // jit_call_method decodes this via direct `as usize` — no NaN-box.
                    let actual_arg_count = if args.is_empty() { 0 } else { args.len() - 1 };
                    let argc_bits = actual_arg_count as i64;
                    let argc_val = self.builder.ins().iconst(types::I64, argc_bits as i64);
                    let argc_slot_idx = self.builder.ins().iadd_imm(old_sp, (args.len() + 1) as i64);
                    let argc_byte_off = self.builder.ins().ishl_imm(argc_slot_idx, 3);
                    let argc_abs_off = self.builder.ins().iadd_imm(argc_byte_off, stack_base_offset as i64);
                    let argc_addr = self.builder.ins().iadd(self.ctx_ptr, argc_abs_off);
                    self.builder.ins().store(MemFlags::new(), argc_val, argc_addr, 0);
                    // arg_count sentinel slot — UInt64 carrier kind per §2.7.5.
                    self.emit_kind_track_write(
                        argc_slot_idx,
                        shape_value::NativeKind::UInt64,
                    );

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
                        // W11-jit-new-array (ADR-006 §2.7.5 stamp-at-compile-
                        // time): when the operand's `NativeKind` is proven
                        // (Int64 / Float64 / Bool), dispatch to the matching
                        // kinded entry point so the FFI sees the raw native
                        // value rather than a deleted-ValueWord-shape u64.
                        // The kind-blind `jit_print` falls through for
                        // operands whose kind the MIR could not prove
                        // (heap arms remain a §2.7.5 follow-up).
                        let kind_hint = if args.is_empty() {
                            None
                        } else {
                            self.operand_slot_kind(&args[0])
                        };
                        let val = if args.is_empty() {
                            self.builder.ins().iconst(types::I64, 0i64)
                        } else {
                            self.compile_operand_raw(&args[0])?
                        };
                        use shape_value::heap_value::HeapKind;
                        use shape_vm::type_tracking::NativeKind;
                        match kind_hint {
                            Some(
                                NativeKind::Int64
                                | NativeKind::UInt64
                                | NativeKind::IntSize
                                | NativeKind::UIntSize,
                            ) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::I32 {
                                    self.builder.ins().sextend(types::I64, val)
                                } else if val_ty == types::I8 {
                                    self.builder.ins().uextend(types::I64, val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(self.ffi.print_i64, &[widened]);
                            }
                            Some(NativeKind::Float64) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let coerced = if val_ty == types::F64 {
                                    val
                                } else if val_ty == types::I64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::F64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(self.ffi.print_f64, &[coerced]);
                            }
                            Some(NativeKind::Bool) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let coerced = if val_ty == types::I8 {
                                    val
                                } else if val_ty == types::I64
                                    || val_ty == types::I32
                                {
                                    self.builder.ins().ireduce(types::I8, val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(self.ffi.print_bool, &[coerced]);
                            }
                            // ── Heap-arm kinded dispatch (§2.7.5 carriers) ──
                            // W12-jit-print-heap-arm-classification
                            // (Phase 3 cluster-0 Round 8A, 2026-05-13):
                            // ADR-006 §2.7.5 stamp-at-compile-time. The
                            // operand's `NativeKind` is proven at MIR-
                            // emit time (via `operand_slot_kind`'s
                            // producing-site classification); route to
                            // the matching kinded FFI entry that reads
                            // the typed `Arc<T>` payload directly. The
                            // FFI bodies take `(ctx_ptr, bits)` and
                            // delegate to the canonical VM-side
                            // `ValueFormatter::format_kinded` for
                            // VM == JIT identical output. No NaN-box tag
                            // decode, no `is_heap_kind` probe — kind IS
                            // the discriminator (§2.7.7 #4 / #7).
                            //
                            // The Option / Result arms below match the
                            // Round 7A `jit_v2_make_option_*` /
                            // `_make_result_*` producer-site Arc-shape
                            // carrier per §2.7.17 — `Arc::into_raw(Arc<
                            // OptionData|ResultData>) as u64` with the
                            // matching `Ptr(HeapKind::Option|Result)`
                            // kind label.
                            Some(NativeKind::Ptr(HeapKind::Option)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_option,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Result)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_result,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            // ── §2.7.5 String carrier arm ───────────────
                            // W12-jit-string-carrier-unification (Phase 3
                            // cluster-0 Round 12 T2/T3, 2026-05-13). The
                            // `MirConstant::Str` / `MirConstant::StringId`
                            // sites in `mir_compiler/ownership.rs` now emit
                            // `arc_string_constant(s)` → `Arc::into_raw(
                            // Arc<String>) as u64` per ADR-006 §2.7.5.
                            // `jit_print_str` reads `&String` via
                            // `ValueSlot::from_raw(bits) + KindedSlot::new(
                            // ..., NativeKind::String)` and delegates to the
                            // canonical `ValueFormatter::format_kinded` — VM
                            // == JIT identical output.
                            //
                            // The String EnumPayload extractor at Round 8A
                            // (`infer_enum_payload_kind` switched to the
                            // full `native_kind_from_concrete_type` mapping)
                            // also produces this same §2.7.5 carrier per
                            // §2.7.17 receiver-recovery soundness, so
                            // `print(Err("x"))` reaches this arm with the
                            // matching carrier shape.
                            Some(NativeKind::String) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_str,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            // ── TypedObject SURFACE-and-stop (residual) ──
                            // The JIT-internal TypedObject path
                            // (`box_typed_object` at `value_ffi.rs:516-518`
                            // returning `unified_box(HK_TYPED_OBJECT, *const
                            // u8)` over a JIT-owned `TypedObject` struct, NOT
                            // the VM-side `Arc<TypedObjectStorage>`) cannot
                            // be reused unchanged for `jit_print_typed_object`
                            // — that body expects an `Arc::into_raw(Arc<
                            // TypedObjectStorage>) as u64` carrier (a
                            // different Rust type with different layout).
                            // Migrating the JIT-side TypedObject to the VM-
                            // side `Arc<TypedObjectStorage>` is a larger
                            // surgery (W11 TypedArray family invariant + 17+
                            // JIT-internal consumers in `typed_object/`,
                            // `data.rs`, `property_access.rs`, etc.).
                            //
                            // Per W12-jit-string-carrier-unification surface-
                            // and-stop discipline (round dispatch §"Surface-
                            // and-stop expected"): "If the TypedObject
                            // migration scope exceeds the budget OR breaks
                            // the W11-jit-new-array TypedArray<T> shape ...
                            // STOP and surface to disambiguate." This arm
                            // stays SURFACE; cluster-1 absorbs the
                            // TypedObject producer migration via a separate
                            // sub-cluster.
                            Some(NativeKind::Ptr(HeapKind::TypedObject)) => {
                                tracing::debug!(
                                    target: "shape_jit",
                                    "jit-mir print: SURFACE \u{a7}2.7.5 \
                                     carrier-mismatch \u{2014} operand \
                                     NativeKind Ptr(TypedObject) stamped but \
                                     the JIT-side `box_typed_object` producer \
                                     at `value_ffi.rs:516-518` emits a \
                                     JIT-internal `TypedObject` struct under \
                                     NaN-box wrap, NOT the VM-side \
                                     `Arc<TypedObjectStorage>` the \
                                     `jit_print_typed_object` body expects. \
                                     Migrating the JIT TypedObject to \
                                     `Arc<TypedObjectStorage>` is out of W12 \
                                     T2/T3 scope per the round's surface-and-\
                                     stop discipline (round dispatch text). \
                                     Cluster-1 follow-up: W17 jit-typed-object-\
                                     arc-storage-migration.",
                                );
                                return Err(format!(
                                    "Route A surface-and-stop: SURFACE \
                                     §2.7.5 carrier-mismatch — `print` \
                                     Call-terminator operand NativeKind \
                                     Ptr(TypedObject); the JIT-side \
                                     `box_typed_object` at \
                                     `ffi/value_ffi.rs:516-518` emits a \
                                     JIT-internal `TypedObject` struct under \
                                     NaN-box wrap (a JIT-owned type, NOT the \
                                     VM-side `Arc<TypedObjectStorage>` the \
                                     `jit_print_typed_object` body reads). \
                                     Migrating the JIT TypedObject to \
                                     `Arc<TypedObjectStorage>` is out of \
                                     W12 T2/T3 scope per the round's \
                                     surface-and-stop discipline. Tracked \
                                     for cluster-1 follow-up \
                                     W17-jit-typed-object-arc-storage-\
                                     migration. kind_hint={:?}",
                                    kind_hint,
                                ));
                            }
                            // ── Phase 3 cluster-2 Round 3 cw-D-fam12
                            //    Scalar Char (Family 1) arm ─────────────
                            //
                            // ADR-006 §2.7.5 amendment (Round 19 S1.5
                            // W12-nativekind-scalar-additions, 2026-05-14):
                            // Char is a 4-byte scalar carrier (codepoint
                            // inline in low 32 bits of `ValueSlot`, no Arc
                            // wrapping). Both the post-amendment scalar
                            // label `NativeKind::Char` and the
                            // pre-amendment heap arm
                            // `NativeKind::Ptr(HeapKind::Char)` recognize
                            // the same carrier shape (per
                            // `KindedSlot::as_char` accessor accepting
                            // both labels at `kinded_slot.rs:594-597`).
                            // Both route to `jit_print_char(u32)` which
                            // takes the codepoint directly — mirror of
                            // `jit_print_i64` / `jit_print_f64` /
                            // `jit_print_bool` (scalar-by-value FFI shape,
                            // no ctx_ptr threading needed).
                            Some(NativeKind::Char)
                            | Some(NativeKind::Ptr(HeapKind::Char)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                // Narrow to I32: ValueSlot::from_char stores
                                // `c as u64` (zero-extended); the low 32 bits
                                // carry the codepoint per `ValueSlot::as_char`
                                // (`self.0 as u32`). Cranelift FFI passes I32
                                // directly.
                                let narrowed = if val_ty == types::I32 {
                                    val
                                } else if val_ty == types::I64
                                    || val_ty == types::I8
                                {
                                    // I64: truncate to low 32 bits.
                                    // I8: zero-extend then narrow (covers
                                    // any upstream code that materializes a
                                    // char as a byte).
                                    if val_ty == types::I64 {
                                        self.builder.ins().ireduce(types::I32, val)
                                    } else {
                                        self.builder.ins().uextend(types::I32, val)
                                    }
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_char,
                                    &[narrowed],
                                );
                            }
                            // ── Phase 3 cluster-2 Round 3 cw-D-fam12
                            //    Concurrency-primitive family (Family 2):
                            //    Mutex / Atomic / Lazy / Channel arms ──
                            //
                            // ADR-006 §2.7.25 concurrency-primitive
                            // rebuild trio (Mutex / Atomic / Lazy) + the
                            // §2.7.20 Channel precedent share the same
                            // `Arc::into_raw(Arc<XData>) as u64` carrier
                            // shape and the same `<X:state>` opaque-tag
                            // print format (per `printing.rs:451-464`
                            // Channel + `:534-551` Mutex/Atomic/Lazy).
                            // Each delegates to `print_kinded_inner` so
                            // VM == JIT identical output is preserved
                            // (no NaN-box tag decode, no `is_heap_kind`
                            // probe — kind IS the discriminator per
                            // §2.7.7 #4 / #7).
                            Some(NativeKind::Ptr(HeapKind::Mutex)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_mutex,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Atomic)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_atomic,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Lazy)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_lazy,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Channel)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_channel,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            // ── Phase 3 cluster-2 Round 4 cw-D-fam3
                            //    Collection family (Family 3) arms ────
                            //
                            // ADR-006 §2.7.5.B per-HeapKind-family kinded
                            // jit_print dispatch arms (Family 3 amendment
                            // extension, 2026-05-16). Per cluster-2-
                            // inventory §E.5 the Collection family is
                            // HashMap (ord 17), HashSet (21), Deque (23),
                            // PriorityQueue (25), Range (26), Iterator (22)
                            // — all `Arc<XData>` heap-arm carriers per
                            // §2.7.5 stamp-at-compile-time (HashMap is
                            // `Arc<HashMapKindedRef>` per Wave 2 Round 3b
                            // C2-joint ckpt-2 Q25.B SUPERSEDED; the inner
                            // per-V monomorphization dispatches at the
                            // carrier's variant tag, transparent to the
                            // FFI body which only forwards the outer
                            // typed-Arc bits + the §2.7.5 kind label to
                            // `print_kinded_inner`). Each delegates to
                            // `print_kinded_inner` so VM == JIT identical
                            // output is preserved (no NaN-box tag decode,
                            // no `is_heap_kind` probe — kind IS the
                            // discriminator per §2.7.7 #4 / #7).
                            //
                            // Per inventory §E.5 categorization the
                            // Iterator HeapKind is part of the Collection
                            // family (NOT the pure-discriminator family);
                            // `HeapValue::Iterator(Arc<IteratorState>)`
                            // participates in the §2.3 typed-Arc payload
                            // pattern per ADR-006 §2.7.16 / Q17
                            // W13-iterator-state, and the dispatch arm in
                            // `format_heap_kind` at `printing.rs:430`
                            // reads the bits as `*const IteratorState`.
                            // ADR-006 §2.7.5.B 2026-05-16
                            Some(NativeKind::Ptr(HeapKind::HashMap)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_hashmap,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::HashSet)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_hashset,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Deque)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_deque,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::PriorityQueue)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_priority_queue,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Range)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_range,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            Some(NativeKind::Ptr(HeapKind::Iterator)) => {
                                let val_ty = self.builder.func.dfg.value_type(val);
                                let widened = if val_ty == types::I64 {
                                    val
                                } else if val_ty == types::F64 {
                                    self.builder
                                        .ins()
                                        .bitcast(types::I64, MemFlags::new(), val)
                                } else {
                                    val
                                };
                                self.builder.ins().call(
                                    self.ffi.print_iterator,
                                    &[self.ctx_ptr, widened],
                                );
                            }
                            // ── NotImplemented(SURFACE): unproven kind /
                            //    unwired heap arm ─────────────────────
                            //
                            // ADR-006 §2.7.5 / §2.7.7 #4 / #7. The kind-
                            // blind `jit_print` fallback (which routed
                            // through the deleted-W-series
                            // `format_value_word` per `ffi/conversion.
                            // rs` lines 200-217) was retired in Round 8A
                            // verification (2026-05-13). Round 7A's
                            // smoke 1.5 close depended on this fallback
                            // because `infer_enum_payload_kind` used
                            // the scalar-only `elem_slot_kind_for_
                            // concrete` classifier, leaving `Err(String)`
                            // / `Some(typed_object)` payload slots
                            // without a kind stamp. Round 8A
                            // verification extended that classifier to
                            // the full `native_kind_from_concrete_type`
                            // mapping (per §2.7.17 receiver-recovery
                            // soundness — `jit_arc_*_payload` returns
                            // the inner `KindedSlot.slot.raw()`
                            // verbatim, preserving the §2.7.5 carrier
                            // shape for every NativeKind variant). With
                            // that extension the kinded arms above
                            // catch every EnumPayload-derived print
                            // operand on Smoke 1.5 — both Ok(I64) and
                            // Err(String) reach their matching kinded
                            // entry.
                            //
                            // Remaining `_`-arm operands are either:
                            // - kind-source gaps upstream of the print
                            //   site (the §2.7.5 conduit doesn't yet
                            //   stamp every MIR shape — e.g. closure-
                            //   return locals without `concrete_types`
                            //   propagation), or
                            // - heap arms beyond {Option, Result,
                            //   String, TypedObject} not yet wired with
                            //   per-kind FFI bodies (TypedArray,
                            //   HashMap, HashSet, ...).
                            //
                            // Both are honest surface-and-stop cases.
                            // No tag-decode, no Bool-default fallback
                            // per CLAUDE.md "Forbidden rationalizations"
                            // + "Renames to refuse on sight". The
                            // pre-Round-8A "preserved baseline"
                            // rationalization was itself the W-series
                            // walk-back the supervisor refuses on
                            // sight (CLAUDE.md "Just a small fallback
                            // for this one edge case" / "Mark this as
                            // a follow-up for a later phase").
                            _ => {
                                tracing::debug!(
                                    target: "shape_jit",
                                    kind_hint = ?kind_hint,
                                    "jit-mir print: SURFACE \u{a7}2.7.5 \u{2014} \
                                     operand NativeKind not proven or unwired \
                                     heap arm. ADR-006 \u{a7}2.7.5 / \u{a7}2.7.7 #4 / #7 \
                                     \u{2014} extend producer-site classification at \
                                     the upstream MIR shape (the \u{a7}2.7.5 \
                                     conduit's producing-site walk) or wire \
                                     the kinded FFI body for the heap kind. \
                                     No kind-blind fallback per CLAUDE.md \
                                     \"Forbidden rationalizations\".",
                                );
                                return Err(format!(
                                    "Route A surface-and-stop: \
                                     NotImplemented(SURFACE) — \
                                     `print` Call-terminator operand \
                                     NativeKind is {:?}; either the \
                                     §2.7.5 producer-site \
                                     classification conduit doesn't \
                                     stamp this operand's kind at the \
                                     upstream MIR shape (extend \
                                     `infer_*_kind` for that shape per \
                                     §2.7.5 / §2.7.7 #4), or the heap \
                                     arm isn't wired with a per-kind \
                                     `jit_print_<heap_kind>` FFI body. \
                                     The pre-Round-8A kind-blind \
                                     `jit_print` fallback was retired \
                                     in Round 8A verification per \
                                     CLAUDE.md \"Forbidden \
                                     rationalizations\" (\"just a small \
                                     fallback for this one edge case\" \
                                     refused on sight).",
                                    kind_hint,
                                ));
                            }
                        }

                        // v2-boundary: None represented as TAG_NULL in NaN-boxed ABI
                        let none_val = self.builder.ins().iconst(types::I64, 0i64);
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
                // The legacy path lowered the payload as a heterogeneous
                // array via the kind-blind `jit_new_array` +
                // `jit_array_push_elem` ABI (the deleted ValueWord-shape
                // path).
                //
                // Route A (ADR-006 §2.7.14 / W11-jit-new-array close) does
                // not yet provide a heterogeneous-element carrier — that's
                // the same `op_new_array` surface the VM-side handler
                // returns (`shape-vm executor/objects/object_creation.rs:
                // 316`). Per §2.7.14 forbidden list ("Bool-default fallback
                // for unknown element kinds") surface-and-stop instead of
                // fabricating a kind. Unit variants (empty args) still
                // route through the normal call dispatch below.
                if let Operand::Constant(MirConstant::Function(name)) = func {
                    if name.contains("::")
                        && self.function_indices.get(name.as_str()).is_none()
                    {
                        return Err(format!(
                            "Route A surface-and-stop: SURFACE — enum \
                             constructor `{}` depends on a heterogeneous- \
                             element-array carrier that Route A does not \
                             yet supply (parallel to op_new_array's VM-side \
                             surface in shape-vm `object_creation.rs:316`). \
                             Tracked as W11-jit-new-array follow-up per \
                             ADR-006 §2.7.4. ADR-006 §2.7.14 / §2.7.5.",
                            name
                        ));
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

                // ── Session 2: STACK-CLOSURE DIRECT-DISPATCH FAST PATH ──
                // When the callee operand loads from a slot that was
                // populated by `emit_stack_closure`, the stored value is a
                // raw `StackClosure*` — not NaN-boxed, not `HK_CLOSURE` —
                // so `jit_call_value` can't dispatch it. Instead, use the
                // slot's side-table (`stack_closure_call_info`) to look up
                // the function_id and per-capture offsets, load captures
                // from the `StackSlot` at their native width, and call the
                // corresponding user_func_ref with captures prepended to the
                // user arg list. This matches the calling convention of
                // closure bodies (first N params are captures).
                let stack_closure_info: Option<(
                    super::StackClosureCallInfo,
                    cranelift::codegen::ir::StackSlot,
                )> = match func {
                    Operand::Copy(Place::Local(slot))
                    | Operand::Move(Place::Local(slot))
                    | Operand::MoveExplicit(Place::Local(slot)) => self
                        .stack_closure_call_info
                        .get(slot)
                        .cloned()
                        .and_then(|info| {
                            self.stack_closure_slots
                                .get(slot)
                                .copied()
                                .map(|ss| (info, ss))
                        }),
                    _ => None,
                };

                if let Some((info, stack_slot)) = stack_closure_info {
                    if let Some(func_ref) =
                        self.user_func_refs.get(&info.function_id).copied()
                    {
                        // Prepend captures as args. Each capture is loaded
                        // from the stack slot at its recorded offset with
                        // its recorded native type, then widened to I64 for
                        // the uniform callee ABI.
                        let mut arg_vals = Vec::with_capacity(
                            info.capture_offsets.len() + args.len() + 1,
                        );
                        arg_vals.push(self.ctx_ptr);
                        for (off, ty) in info
                            .capture_offsets
                            .iter()
                            .zip(info.capture_types.iter())
                        {
                            let raw = self
                                .builder
                                .ins()
                                .stack_load(*ty, stack_slot, *off);
                            let widened = if *ty == types::I64 {
                                raw
                            } else if *ty == types::F64 {
                                self.builder
                                    .ins()
                                    .bitcast(types::I64, MemFlags::new(), raw)
                            } else if *ty == types::I32 || *ty == types::I16 {
                                self.builder.ins().sextend(types::I64, raw)
                            } else if *ty == types::I8 {
                                self.builder.ins().uextend(types::I64, raw)
                            } else {
                                raw
                            };
                            arg_vals.push(widened);
                        }
                        // User args follow, widened uniformly.
                        for arg in args.iter() {
                            let val = self.compile_operand(arg)?;
                            let val_ty = self.builder.func.dfg.value_type(val);
                            let boxed = if val_ty == types::I64 {
                                val
                            } else if val_ty == types::F64 {
                                self.builder
                                    .ins()
                                    .bitcast(types::I64, MemFlags::new(), val)
                            } else if val_ty == types::I32 || val_ty == types::I16 {
                                self.builder.ins().sextend(types::I64, val)
                            } else if val_ty == types::I8 {
                                self.builder.ins().uextend(types::I64, val)
                            } else {
                                val
                            };
                            arg_vals.push(boxed);
                        }

                        let inst = self.builder.ins().call(func_ref, &arg_vals);
                        let signal = self.builder.inst_results(inst)[0];

                        // Deopt: if signal < 0, propagate the error.
                        let zero = self.builder.ins().iconst(types::I32, 0);
                        let is_error = self.builder.ins().icmp(
                            IntCC::SignedLessThan,
                            signal,
                            zero,
                        );
                        let deopt_block = self.builder.create_block();
                        let continue_block = self.builder.create_block();
                        self.builder.ins().brif(
                            is_error,
                            deopt_block,
                            &[],
                            continue_block,
                            &[],
                        );
                        self.builder.switch_to_block(deopt_block);
                        self.builder.seal_block(deopt_block);
                        self.builder.ins().return_(&[signal]);

                        self.builder.switch_to_block(continue_block);
                        self.builder.seal_block(continue_block);
                        let stack_offset = crate::context::STACK_OFFSET as i32;
                        let result = self.builder.ins().load(
                            types::I64,
                            MemFlags::new(),
                            self.ctx_ptr,
                            stack_offset,
                        );

                        self.release_old_value_if_heap(destination)?;
                        self.write_place(destination, result)?;
                        self.reload_referenced_locals();

                        let next_block = self.block_map.get(next).ok_or_else(|| {
                            format!(
                                "MirToIR: unknown call continuation block {}",
                                next
                            )
                        })?;
                        self.builder.ins().jump(*next_block, &[]);
                        return Ok(());
                    }
                }

                // Check if we have a direct FuncRef for the callee.
                let func_ref = func_id.and_then(|fid| self.user_func_refs.get(&fid).copied());

                let result = if let Some(func_ref) = func_ref {
                    // ── DIRECT CALL PATH ──────────────────────────────────
                    // Compile args as SSA values and pass as native Cranelift params.
                    // ABI: fn(ctx_ptr, arg0, arg1, ..., argN) -> i32
                    // The callee stores its return value to ctx.stack[0].
                    // R4.2E: callee ABI uses uniform I64 params. Widen narrow
                    // Cranelift types inline to I64 bit-patterns (sextend /
                    // uextend / bitcast) — NOT NaN-boxing tagging.
                    let mut arg_vals = Vec::with_capacity(args.len() + 1);
                    arg_vals.push(self.ctx_ptr);
                    for arg in args.iter() {
                        let val = self.compile_operand(arg)?;
                        let val_ty = self.builder.func.dfg.value_type(val);
                        let boxed = if val_ty == types::I64 {
                            val
                        } else if val_ty == types::F64 {
                            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
                        } else if val_ty == types::I32 {
                            self.builder.ins().sextend(types::I64, val)
                        } else if val_ty == types::I8 {
                            self.builder.ins().uextend(types::I64, val)
                        } else if val_ty == types::I16 {
                            self.builder.ins().sextend(types::I64, val)
                        } else {
                            val
                        };
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
                    //
                    // ADR-006 §2.7.7 / Q9 + §2.7.11 / Q12: every push into
                    // `JITContext.stack` writes the producing-site kind into
                    // the parallel `stack_kinds` track in lockstep. The
                    // callee's kind classifies the dispatch shape inside
                    // `jit_call_value` (Closure → raw-Arc closure path,
                    // FunctionRef / UInt64 → function-id path); per-arg
                    // kinds flow into the trampoline VM's frame setup as
                    // §2.7.11/Q12 carriers.
                    let stack_base_offset = crate::context::STACK_OFFSET as i32;
                    let sp_offset = crate::context::STACK_PTR_OFFSET as i32;

                    let old_sp = self.builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        self.ctx_ptr,
                        sp_offset,
                    );

                    // Source the callee kind from the producing site.
                    // Precise kinds — `Ptr(HeapKind::Closure)` for
                    // closure-bearing slots seeded by `infer_slot_kinds::
                    // ClosureCapture`, `UInt64` for `MirConstant::Function`
                    // inline function refs — drive the §2.7.11/Q12 callee
                    // classification at `jit_call_value` exactly. For
                    // opaque-source slots whose inference left `None`,
                    // the documented §2.7.5 carrier kind `UInt64` flows in
                    // instead — `jit_call_value`'s UInt64 arm preserves
                    // the existing JIT-internal NaN-box bit-shape dispatch
                    // (cases 1 / 2 — inline function refs and legacy
                    // HK_CLOSURE callees). NOT a Bool-default fallback
                    // (§2.7.7 #9 forbidden); `UInt64` is the §2.7.5
                    // carrier kind for I64-wide raw bits without further
                    // classification.
                    let callee_kind = self.operand_slot_kind_or_carrier(func);

                    // R4.2E: indirect-call callee pushed to ctx.stack as raw
                    // I64 bit-pattern. Widen narrow types inline — closures
                    // are already I64 in practice but the code path is type-
                    // agnostic.
                    let callee_val = self.compile_operand(func)?;
                    let callee_ty = self.builder.func.dfg.value_type(callee_val);
                    let callee_boxed = if callee_ty == types::I64 {
                        callee_val
                    } else if callee_ty == types::F64 {
                        self.builder.ins().bitcast(types::I64, MemFlags::new(), callee_val)
                    } else if callee_ty == types::I32 {
                        self.builder.ins().sextend(types::I64, callee_val)
                    } else if callee_ty == types::I8 {
                        self.builder.ins().uextend(types::I64, callee_val)
                    } else if callee_ty == types::I16 {
                        self.builder.ins().sextend(types::I64, callee_val)
                    } else {
                        callee_val
                    };
                    let callee_slot_idx = old_sp;
                    let callee_byte_off = self.builder.ins().ishl_imm(callee_slot_idx, 3);
                    let callee_abs_off = self.builder.ins().iadd_imm(callee_byte_off, stack_base_offset as i64);
                    let callee_addr = self.builder.ins().iadd(self.ctx_ptr, callee_abs_off);
                    self.builder.ins().store(MemFlags::new(), callee_boxed, callee_addr, 0);
                    // Lockstep parallel-kind track write (§2.7.7 / Q9).
                    self.emit_kind_track_write(callee_slot_idx, callee_kind);

                    // R4.2E: indirect-call args pushed to ctx.stack as raw
                    // I64 bit-patterns. Widen narrow Cranelift types inline.
                    for (i, arg) in args.iter().enumerate() {
                        // Source the arg kind from the producing site for the
                        // parallel-kind track. Falls back to `UInt64`
                        // (the §2.7.5 carrier kind for I64-wide raw bits)
                        // when the producing-site inference is opaque —
                        // not a Bool-default fallback.
                        let _ = i;
                        let arg_kind = self.operand_slot_kind_or_carrier(arg);

                        let val = self.compile_operand(arg)?;
                        let val_ty = self.builder.func.dfg.value_type(val);
                        let boxed = if val_ty == types::I64 {
                            val
                        } else if val_ty == types::F64 {
                            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
                        } else if val_ty == types::I32 {
                            self.builder.ins().sextend(types::I64, val)
                        } else if val_ty == types::I8 {
                            self.builder.ins().uextend(types::I64, val)
                        } else if val_ty == types::I16 {
                            self.builder.ins().sextend(types::I64, val)
                        } else {
                            val
                        };
                        let slot_idx = self.builder.ins().iadd_imm(old_sp, (i + 1) as i64);
                        let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                        let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base_offset as i64);
                        let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                        self.builder.ins().store(MemFlags::new(), boxed, store_addr, 0);
                        // Lockstep parallel-kind track write (§2.7.7 / Q9).
                        self.emit_kind_track_write(slot_idx, arg_kind);
                    }

                    // v2-boundary: arg_count stored as a raw i64 on ctx.stack.
                    // jit_call_value decodes this via direct `as usize` — no NaN-box.
                    //
                    // ADR-006 §2.7.11/Q12 + §2.7.5: the arg_count sentinel
                    // slot's kind is `NativeKind::UInt64` — the documented
                    // "I64-wide raw bits carrier kind" / function-id-class
                    // kind already used for FFI-boundary scalar sentinels
                    // (cf. `dispatch_call_via_trampoline_vm` callee/arg
                    // kind companion). NOT a Bool-default fallback.
                    let total_items = 1 + args.len() + 1; // callee + args + arg_count
                    let argc_slot_idx = self.builder.ins().iadd_imm(old_sp, (1 + args.len()) as i64);
                    let argc_byte_off = self.builder.ins().ishl_imm(argc_slot_idx, 3);
                    let argc_abs_off = self.builder.ins().iadd_imm(argc_byte_off, stack_base_offset as i64);
                    let argc_addr = self.builder.ins().iadd(self.ctx_ptr, argc_abs_off);
                    let argc_val = self.builder.ins().iconst(types::I64,
                        args.len() as i64);
                    self.builder.ins().store(MemFlags::new(), argc_val, argc_addr, 0);
                    // Lockstep parallel-kind track write (§2.7.7 / Q9).
                    self.emit_kind_track_write(
                        argc_slot_idx,
                        shape_value::NativeKind::UInt64,
                    );

                    // Update stack_ptr
                    let new_sp = self.builder.ins().iadd_imm(old_sp, total_items as i64);
                    self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                    // jit_call_value reads callee + args + arg_count from stack
                    // AND the parallel-kind track for the §2.7.11/Q12
                    // callee-classification dispatch.
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
                // Write return value to ctx.stack[0] and set return_type_tag
                // for native type preservation (avoids NaN-boxing on return path).
                let return_slot = SlotId(0);
                // W11-jit-new-array: when SlotId(0) is not declared, the
                // program has no value-bearing return (e.g. top-level ending
                // with `print(x)`). Stamp UNIT so the executor's typed
                // dispatch maps it to `WireValue::Null` — pre-W11 this
                // arm left the tag at its zero default (NANBOXED) and the
                // executor.rs:267 SURFACE fired.
                if !self.locals.contains_key(&return_slot) {
                    let tag = self.builder.ins().iconst(
                        types::I8,
                        crate::context::RETURN_TAG_UNIT as i64,
                    );
                    let tag_offset = crate::context::RETURN_TYPE_TAG_OFFSET as i32;
                    self.builder
                        .ins()
                        .store(MemFlags::new(), tag, self.ctx_ptr, tag_offset);
                    let sp_zero = self.builder.ins().iconst(types::I64, 0);
                    let sp_offset = crate::context::STACK_PTR_OFFSET as i32;
                    self.builder
                        .ins()
                        .store(MemFlags::new(), sp_zero, self.ctx_ptr, sp_offset);
                    let signal = self.builder.ins().iconst(types::I32, 0);
                    self.builder.ins().return_(&[signal]);
                    return Ok(());
                }
                if let Some(&var) = self.locals.get(&return_slot) {
                    let ret_val = self.builder.use_var(var);
                    let val_type = self.builder.func.dfg.value_type(ret_val);
                    let stack_offset = crate::context::STACK_OFFSET as i32;

                    if val_type == types::F64 {
                        // Native f64: store raw bits, set tag=1
                        let as_bits = self.builder.ins().bitcast(types::I64, MemFlags::new(), ret_val);
                        self.builder.ins().store(MemFlags::new(), as_bits, self.ctx_ptr, stack_offset);
                        let tag = self.builder.ins().iconst(types::I8, crate::context::RETURN_TAG_F64 as i64);
                        let tag_offset = crate::context::RETURN_TYPE_TAG_OFFSET as i32;
                        self.builder.ins().store(MemFlags::new(), tag, self.ctx_ptr, tag_offset);
                    } else if val_type == types::I8 {
                        // Native bool: zero-extend to I64, set tag=4
                        let extended = self.builder.ins().uextend(types::I64, ret_val);
                        self.builder.ins().store(MemFlags::new(), extended, self.ctx_ptr, stack_offset);
                        let tag = self.builder.ins().iconst(types::I8, crate::context::RETURN_TAG_BOOL as i64);
                        let tag_offset = crate::context::RETURN_TYPE_TAG_OFFSET as i32;
                        self.builder.ins().store(MemFlags::new(), tag, self.ctx_ptr, tag_offset);
                    } else if val_type == types::I32 {
                        // Native i32 (NativeKind::Int32 / UInt32): sign-extend
                        // to I64 and stamp RETURN_TAG_I32 so the host marshals
                        // the value as an integer rather than a NaN-boxed
                        // ValueWord. Pre-W11 close this arm fell through to
                        // the `RETURN_TAG_NANBOXED` default — the §2.7.5
                        // kind-source gap the executor.rs:267 SURFACE
                        // documented (W11-jit-new-array unblocks).
                        let extended = self.builder.ins().sextend(types::I64, ret_val);
                        self.builder.ins().store(MemFlags::new(), extended, self.ctx_ptr, stack_offset);
                        let tag = self.builder.ins().iconst(types::I8, crate::context::RETURN_TAG_I32 as i64);
                        let tag_offset = crate::context::RETURN_TYPE_TAG_OFFSET as i32;
                        self.builder.ins().store(MemFlags::new(), tag, self.ctx_ptr, tag_offset);
                    } else {
                        // I64. Under strict typing the return-slot kind is
                        // statically known: an `Int64` slot is a raw native
                        // integer (RETURN_TAG_I64); a slot with no inferred
                        // kind is a `()` / no-value return (RETURN_TAG_UNIT);
                        // any other kind that happened to widen to I64 in
                        // Cranelift (Ptr to a v2 heap value, String, etc.)
                        // is still the deleted NaN-boxed ABI's residual —
                        // those paths hit the executor.rs:267 SURFACE.
                        //
                        // W11-jit-new-array stamps RETURN_TAG_I64 from the
                        // bytecode compiler's slot kind (§2.7.5 stamp-at-
                        // compile-time): when the return slot's
                        // `NativeKind` is `Int64`/`UInt64`/`IntSize`/
                        // `UIntSize` (and their nullable variants), the
                        // value is a raw native integer. `None` kind means
                        // the slot was never written with a value-bearing
                        // expression (typical for top-level programs ending
                        // in `print(x)`) — stamp UNIT. All other I64 arms
                        // fall through to RETURN_TAG_NANBOXED — the §2.7.5
                        // kind-source gap surfaced at `executor.rs:267`.
                        use shape_vm::type_tracking::NativeKind;
                        let return_kind = super::types::slot_kind_for_local(
                            &self.slot_kinds,
                            return_slot.0,
                        );
                        let raw_int = matches!(
                            return_kind,
                            Some(
                                NativeKind::Int64
                                    | NativeKind::UInt64
                                    | NativeKind::IntSize
                                    | NativeKind::UIntSize
                                    | NativeKind::NullableInt64
                                    | NativeKind::NullableUInt64
                                    | NativeKind::NullableIntSize
                                    | NativeKind::NullableUIntSize
                            )
                        );
                        self.builder.ins().store(MemFlags::new(), ret_val, self.ctx_ptr, stack_offset);
                        let tag_value = if raw_int {
                            crate::context::RETURN_TAG_I64
                        } else if return_kind.is_none() {
                            crate::context::RETURN_TAG_UNIT
                        } else {
                            crate::context::RETURN_TAG_NANBOXED
                        };
                        let tag = self.builder.ins().iconst(types::I8, tag_value as i64);
                        let tag_offset = crate::context::RETURN_TYPE_TAG_OFFSET as i32;
                        self.builder.ins().store(MemFlags::new(), tag, self.ctx_ptr, tag_offset);
                    }

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

    /// W10 jit-call-method-user-trait-fix (2026-05-17): emit a user-type
    /// operator-trait dispatch as a method-call equivalent, writing the
    /// result into `destination`.
    ///
    /// Mirror of the `MirConstant::Method` `TerminatorKind::Call` path at
    /// `compile_terminator` (this file, lines ~315-431), reused for the
    /// MIR `Rvalue::BinaryOp` / `Rvalue::UnaryOp` sites whose source span
    /// is recorded in `operator_trait_dispatch_sites` (populated at
    /// bytecode-compile time at `crates/shape-vm/src/compiler/expressions/
    /// binary_ops.rs::emit_operator_trait_call` and the parallel Neg/Not
    /// sites at `crates/shape-vm/src/compiler/expressions/unary_ops.rs`).
    ///
    /// `receiver_operands` is a single-element slice with the receiver
    /// (lhs for binary, operand for unary). `extra_args` is the remainder
    /// (rhs for binary; empty for unary). The split shape mirrors the
    /// bytecode-side `OpCode::CallMethod` ABI where the receiver is
    /// `args[0]` and any explicit method arguments follow.
    ///
    /// Does NOT emit a continuation jump (the caller is inside a
    /// `StatementKind::Assign` handler, not a Call terminator).
    pub(crate) fn emit_user_trait_method_call(
        &mut self,
        method_name: &str,
        receiver_operands: &[shape_vm::mir::types::Operand],
        extra_args: &[shape_vm::mir::types::Operand],
        destination: &shape_vm::mir::types::Place,
    ) -> Result<(), String> {
        let stack_base_offset = crate::context::STACK_OFFSET as i32;
        let sp_offset = crate::context::STACK_PTR_OFFSET as i32;

        let old_sp = self.builder.ins().load(
            types::I64,
            MemFlags::new(),
            self.ctx_ptr,
            sp_offset,
        );

        // ADR-006 §2.7.7 / Q9 lockstep: every data push stamps the parallel-
        // kind track in the same slot. Mirrors `compile_terminator`'s
        // `MirConstant::Method` arm: args[0] = receiver, args[1..] =
        // explicit method arguments. Each operand is widened to 8-byte I64
        // per the JIT-stack ABI; no NaN-box tagging.
        let combined: Vec<&shape_vm::mir::types::Operand> = receiver_operands
            .iter()
            .chain(extra_args.iter())
            .collect();
        for (i, arg) in combined.iter().enumerate() {
            let arg_kind = self.operand_slot_kind_or_carrier(arg);
            let val = self.compile_operand(arg)?;
            let val_ty = self.builder.func.dfg.value_type(val);
            let boxed = if val_ty == types::I64 {
                val
            } else if val_ty == types::F64 {
                self.builder
                    .ins()
                    .bitcast(types::I64, MemFlags::new(), val)
            } else if val_ty == types::I32 {
                self.builder.ins().sextend(types::I64, val)
            } else if val_ty == types::I8 {
                self.builder.ins().uextend(types::I64, val)
            } else if val_ty == types::I16 {
                self.builder.ins().sextend(types::I64, val)
            } else {
                val
            };
            let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
            let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
            let abs_off = self
                .builder
                .ins()
                .iadd_imm(byte_off, stack_base_offset as i64);
            let store_addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
            self.builder
                .ins()
                .store(MemFlags::new(), boxed, store_addr, 0);
            self.emit_kind_track_write(slot_idx, arg_kind);
        }

        // Push method name (heap String) — kind = NativeKind::String.
        let method_str_bits =
            crate::ffi::value_ffi::box_string(method_name.to_string());
        let method_val = self
            .builder
            .ins()
            .iconst(types::I64, method_str_bits as i64);
        let method_slot_idx = self
            .builder
            .ins()
            .iadd_imm(old_sp, combined.len() as i64);
        let method_byte_off = self.builder.ins().ishl_imm(method_slot_idx, 3);
        let method_abs_off = self
            .builder
            .ins()
            .iadd_imm(method_byte_off, stack_base_offset as i64);
        let method_addr = self.builder.ins().iadd(self.ctx_ptr, method_abs_off);
        self.builder
            .ins()
            .store(MemFlags::new(), method_val, method_addr, 0);
        self.emit_kind_track_write(
            method_slot_idx,
            shape_value::NativeKind::String,
        );

        // Push arg_count = explicit args (excludes receiver) — UInt64 carrier.
        let actual_arg_count = extra_args.len() as i64;
        let argc_val = self.builder.ins().iconst(types::I64, actual_arg_count);
        let argc_slot_idx = self
            .builder
            .ins()
            .iadd_imm(old_sp, (combined.len() + 1) as i64);
        let argc_byte_off = self.builder.ins().ishl_imm(argc_slot_idx, 3);
        let argc_abs_off = self
            .builder
            .ins()
            .iadd_imm(argc_byte_off, stack_base_offset as i64);
        let argc_addr = self.builder.ins().iadd(self.ctx_ptr, argc_abs_off);
        self.builder
            .ins()
            .store(MemFlags::new(), argc_val, argc_addr, 0);
        self.emit_kind_track_write(
            argc_slot_idx,
            shape_value::NativeKind::UInt64,
        );

        // Update stack_ptr: receiver(s) + extra_args + method_name + arg_count.
        let total_items = combined.len() + 2;
        let new_sp = self.builder.ins().iadd_imm(old_sp, total_items as i64);
        self.builder
            .ins()
            .store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

        // Call jit_call_method(ctx, total_count).
        let count_val = self.builder.ins().iconst(types::I64, total_items as i64);
        let inst = self.builder.ins().call(
            self.ffi.call_method,
            &[self.ctx_ptr, count_val],
        );
        let result = self.builder.inst_results(inst)[0];

        // Restore stack_ptr to old value.
        self.builder
            .ins()
            .store(MemFlags::new(), old_sp, self.ctx_ptr, sp_offset);

        // Write result to destination + reload referenced locals (per
        // the standard Call-terminator wind-down).
        self.write_place(destination, result)?;
        self.reload_referenced_locals();

        Ok(())
    }
}
