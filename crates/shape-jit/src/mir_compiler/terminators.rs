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
                            // ── Carrier-mismatch surface-and-stop ──────
                            // ADR-006 §2.7.5 carrier-shape audit
                            // (W12-jit-result-carrier-unification, the
                            // cluster-1 candidate Round 6A surfaced as
                            // site (a)): the operand's `NativeKind` is
                            // stamped (`NativeKind::String` for string
                            // literals via `MirConstant::Str`,
                            // `NativeKind::Ptr(HeapKind::TypedObject)`
                            // for struct locals), but the JIT-side
                            // producer stores the bits in the legacy
                            // NaN-box UnifiedValue carrier shape
                            // (`box_string` returns `unified_box(HK_
                            // STRING, Arc<String>)` per `value_ffi.rs:
                            // 535`; `box_typed_object` returns
                            // `unified_box(HK_TYPED_OBJECT, *const u8)`
                            // per `value_ffi.rs:516`). The §2.7.5
                            // carrier contract for those kind labels is
                            // raw `Arc::into_raw(Arc<T>) as u64`
                            // pointers — NOT NaN-box-wrapped pointers.
                            // Dispatching to `jit_print_str` /
                            // `jit_print_typed_object` (which read
                            // `*const String` / `*const TypedObject
                            // Storage` directly per §2.7.17) would
                            // dereference NaN-box header bits as a
                            // payload pointer and segfault.
                            //
                            // The Round 7A trinity (§2.7.17 Result/Option
                            // Arc-shape producers) is the matching
                            // pattern: it MIGRATED the JIT-side producer
                            // off `box_ok` / `box_some` (legacy NaN-box)
                            // to `jit_v2_make_result_ok` /
                            // `jit_v2_make_option_some` (Arc-shape).
                            // The String / TypedObject producers
                            // (`MirConstant::Str` lowering, struct
                            // `Aggregate` lowering) need the same
                            // producer-side migration — that's the
                            // cluster-1 `W12-jit-result-carrier-
                            // unification` scope (generalized to all
                            // §2.7.5 heap carriers).
                            //
                            // Until that migration lands, surface-and-
                            // stop is the only acceptable response per
                            // §2.7.7 #4 / #7 forbidden list — the
                            // deleted W-series tag_bits dispatch and
                            // the W-series Bool-default rationalization
                            // both refused on sight per CLAUDE.md.
                            Some(NativeKind::String)
                            | Some(NativeKind::Ptr(HeapKind::TypedObject)) => {
                                if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                                    eprintln!(
                                        "[jit-mir] print: SURFACE §2.7.5 carrier-\
                                         mismatch — operand NativeKind {:?} is \
                                         stamped, but the JIT-side producer for \
                                         this kind label stores the bits in the \
                                         legacy NaN-box UnifiedValue carrier \
                                         (`box_string` / `box_typed_object`), \
                                         NOT the §2.7.5 `Arc::into_raw(Arc<T>) as \
                                         u64` carrier the matching kinded print \
                                         body expects. Same defect class as \
                                         Round 6A site (a) for Result/Option, \
                                         which Round 7A's trinity resolved by \
                                         migrating the JIT producers to Arc-\
                                         shape (`jit_v2_make_result_ok` / \
                                         `jit_v2_make_option_some`). The \
                                         String / TypedObject producer \
                                         migration is the cluster-1 \
                                         `W12-jit-result-carrier-unification` \
                                         scope (generalized to all §2.7.5 heap \
                                         carriers).",
                                        kind_hint,
                                    );
                                }
                                return Err(format!(
                                    "Route A surface-and-stop: SURFACE §2.7.5 \
                                     carrier-mismatch — `print` Call-terminator \
                                     operand NativeKind is {:?}, kind label \
                                     stamped correctly but the JIT-side \
                                     producer stores the bits in the legacy \
                                     NaN-box UnifiedValue carrier shape (per \
                                     `value_ffi.rs::box_string` / \
                                     `box_typed_object`), NOT the §2.7.5 / \
                                     §2.7.17 `Arc::into_raw(Arc<T>) as u64` \
                                     carrier the matching kinded print body \
                                     (`jit_print_str` / `jit_print_typed_\
                                     object`) reads. Cluster-1 candidate \
                                     `W12-jit-result-carrier-unification` \
                                     (Round 6A site (a), generalized): \
                                     migrate `MirConstant::Str` and struct \
                                     Aggregate lowering to Arc-shape \
                                     producers, then enable the dispatch \
                                     arm. Same migration shape as Round 7A's \
                                     `jit_v2_make_result_ok` / \
                                     `jit_v2_make_option_some` for §2.7.17. \
                                     Per CLAUDE.md \"Forbidden \
                                     rationalizations\" + §2.7.7 #4 / #7, \
                                     the deleted W-series tag_bits \
                                     dispatch and the W-series Bool-\
                                     default rationalization both \
                                     refused on sight at the FFI \
                                     boundary.",
                                    kind_hint,
                                ));
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
                                if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                                    eprintln!(
                                        "[jit-mir] print: SURFACE §2.7.5 \
                                         — operand NativeKind not proven \
                                         ({:?}) or unwired heap arm. \
                                         ADR-006 §2.7.5 / §2.7.7 #4 / \
                                         #7 — extend producer-site \
                                         classification at the upstream \
                                         MIR shape (the §2.7.5 conduit's \
                                         producing-site walk) or wire \
                                         the kinded FFI body for the \
                                         heap kind. No kind-blind \
                                         fallback per CLAUDE.md \
                                         \"Forbidden rationalizations\".",
                                        kind_hint,
                                    );
                                }
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
}
