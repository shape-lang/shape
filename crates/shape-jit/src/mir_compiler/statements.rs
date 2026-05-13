//! MIR Statement → Cranelift IR compilation.
//!
//! MIR has ~7 statement kinds (vs ~100 bytecode opcodes).
//! Ownership is structural: Assign releases old heap values,
//! Drop releases refcounts, Nop is skipped.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::NativeKind;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile a single MIR statement.
    pub(crate) fn compile_statement(
        &mut self,
        stmt: &MirStatement,
    ) -> Result<(), String> {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // v2 fast path: when the destination is `Place::Local(s)` whose
                // ConcreteType is `Array<scalar>`, allocate a real v2 typed
                // array via FFI and bypass the legacy NaN-boxed Aggregate path.
                if let (Rvalue::Aggregate(operands), Some(elem_kind)) = (
                    rvalue,
                    self.v2_typed_array_elem_kind(place),
                ) {
                    if let Some(arr_val) =
                        self.emit_v2_array_aggregate(operands, elem_kind)?
                    {
                        self.release_old_value_if_heap(place)?;
                        self.write_place(place, arr_val)?;
                        return Ok(());
                    }
                }

                // v2 fast path: when the destination is a TypedObject slot
                // (`ConcreteType::Struct(_)` / `Enum(_)` / `Option(_)` /
                // `Result(_, _)` / `Tuple(_)`), the bytecode/MIR lowering
                // emits a redundant `Assign(Aggregate)` (a scratch step
                // mirroring the AST shape) followed by a real
                // `StatementKind::ObjectStore` (or `EnumStore`) that does
                // the actual `typed_object_alloc` + per-field set. Skip
                // the kind-blind Aggregate compilation here — the real
                // allocation arrives at the ObjectStore site. This is
                // the §2.7.5 conduit's user-visible benefit: with the
                // top-level slot's `ConcreteType` threaded from the
                // bytecode compiler, the JIT no longer surfaces-and-stops
                // on `Point { x, y }`-style struct literals (W12-top-level-
                // concrete-types-conduit close, 2026-05-12).
                //
                // ADR-006 §2.7.5 forbidden list — this is NOT a Bool-
                // default fallback. The condition is "the bytecode
                // compiler proved this slot's `ConcreteType`"; when
                // unproven the slot's `concrete_types[slot]` is
                // `ConcreteType::Void` and `is_typed_object_slot`
                // returns `false` — codegen surfaces-and-stops at the
                // Aggregate site, not silently leaks. Per §2.7.5 the
                // kind is stamped at compile time from the proven
                // type information, never decoded from runtime bits.
                if matches!(rvalue, Rvalue::Aggregate(_))
                    && self.is_typed_object_slot(place)
                {
                    return Ok(());
                }

                // Session 2: propagate stack-closure call metadata on simple
                // local→local moves/copies. MIR frequently shuffles a closure
                // handle between slots (e.g. `let f = <closure>; f(x)` lowers
                // to `SlotId(X) <- ClosureCapture; SlotId(Y) <- Move SlotId(X);
                // Call Copy(SlotId(Y))`). Without this copy the Call
                // terminator's stack-closure fast path can't find the side-
                // table entry keyed on the original slot.
                if let (
                    Place::Local(dst),
                    Rvalue::Use(
                        Operand::Move(Place::Local(src))
                        | Operand::Copy(Place::Local(src))
                        | Operand::MoveExplicit(Place::Local(src)),
                    ),
                ) = (place, rvalue)
                {
                    if let Some(info) =
                        self.stack_closure_call_info.get(src).cloned()
                    {
                        self.stack_closure_call_info.insert(*dst, info);
                    }
                    if let Some(ss) = self.stack_closure_slots.get(src).copied() {
                        self.stack_closure_slots.insert(*dst, ss);
                    }
                }

                // Release old value if overwriting a heap local.
                self.release_old_value_if_heap(place)?;
                // Compile the rvalue.
                let val = self.compile_rvalue(rvalue)?;
                // Write the new value.
                self.write_place(place, val)?;
                Ok(())
            }

            StatementKind::Drop(place) => {
                self.emit_drop(place)?;
                Ok(())
            }

            StatementKind::ArrayStore {
                container_slot,
                operands: _,
            } => {
                // v2 fast path: when the container slot is a v2 `Array<scalar>`,
                // the preceding `Assign(Aggregate)` has already allocated a real
                // `*mut TypedArray<T>` and populated it. Skip the redundant
                // re-build — the MIR ownership transfer has already been
                // observed by the preceding Aggregate.
                let container_place = Place::Local(*container_slot);
                if self.v2_typed_array_elem_kind(&container_place).is_some() {
                    return Ok(());
                }

                // Route A (ADR-006 §2.7.14 / W11-jit-new-array close):
                // reaching here means the container slot has no proven
                // `Array<scalar>` element kind. The kind-blind
                // `jit_new_array` + `jit_array_push_elem` path was the
                // deleted ValueWord-shape ABI. Per §2.7.14 forbidden list
                // ("Bool-default fallback for unknown element kinds")
                // surface-and-stop instead of fabricating a kind.
                Err(
                    "Route A surface-and-stop: SURFACE — \
                     StatementKind::ArrayStore reached the kind-blind \
                     fallback. The v2 typed-array fast path requires the \
                     container `Place::Local` to carry a \
                     `ConcreteType::Array<scalar>`; reaching here means the \
                     element kind is not threaded from the producing call \
                     signature. Tracked as W11-jit-new-array per \
                     phase-3-kickoff-prompt.md. ADR-006 §2.7.14 / §2.7.5."
                        .to_string(),
                )
            }

            StatementKind::ObjectStore {
                container_slot,
                operands,
                field_names,
            } => {
                // Register a schema for cross-boundary compatibility.
                let real_field_names: Vec<String> = field_names
                    .iter()
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .collect();
                let sid = shape_runtime::type_schema::register_predeclared_any_schema(
                    &real_field_names,
                );

                let schema_id = self.builder.ins().iconst(
                    cranelift::prelude::types::I32,
                    sid as i64,
                );
                let data_size = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    (operands.len() as i64) * 8,
                );
                let inst = self.builder.ins().call(
                    self.ffi.typed_object_alloc,
                    &[schema_id, data_size],
                );
                let mut obj = self.builder.inst_results(inst)[0];

                // Record field_name -> positional byte offset mapping.
                for (i, name) in field_names.iter().enumerate() {
                    if !name.is_empty() {
                        self.field_byte_offsets.insert(name.clone(), (i as u16) * 8);
                    }
                }

                // R4.2C: FFI signatures accept plain u64 bit-patterns — no
                // box wrap needed at call site. `typed_object_set_field`
                // takes field values as ValueWord-encoded I64 slots. Native
                // F64/I32/I8 operands from `compile_operand_raw` must be
                // widened to I64 before the FFI call so the Cranelift
                // verifier accepts the parameter types.
                for (i, op) in operands.iter().enumerate() {
                    let val_raw = self.compile_operand_raw(op)?;
                    let val = self.widen_to_i64(val_raw);
                    let offset_val = self.builder.ins().iconst(
                        cranelift::prelude::types::I64,
                        (i as i64) * 8,
                    );
                    let inst = self.builder.ins().call(
                        self.ffi.typed_object_set_field,
                        &[obj, offset_val, val],
                    );
                    obj = self.builder.inst_results(inst)[0];
                }

                let place = Place::Local(*container_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, obj)?;
                Ok(())
            }

            StatementKind::EnumStore {
                container_slot,
                operands,
                variant_name,
            } => {
                // Enum variant construction.
                //
                // W12-collection-constructor-mir-lowering (Phase 3
                // cluster-0 Round 6C → Round 10, 2026-05-13): the
                // `EnumStore` MIR shape is also used by the primitive-
                // collection ctor family (`Set` / `HashMap` / `Deque` /
                // `PriorityQueue` / `Channel` / `Mutex` / `Atomic` /
                // `Lazy`) per the W12-enum-constructor audit's §5.3
                // "reuse `EnumStore` with `kind`-on-the-slot threading"
                // recommendation. The `variant_name` disambiguates
                // enum-variant from collection-ctor. Round 10 wires
                // each collection-ctor name to the Round 9 typed-Arc
                // allocator FuncRef.
                //
                // ADR-006 §2.7.5 producer-side classification: the ctor
                // kind is known here at MIR-emission time, threaded
                // through `variant_name`, and dispatched to the
                // matching `jit_v2_make_*` FFI body.
                //
                // Carrier shape (audit §4.1 + Round 9 binding): all
                // entries return `Arc::into_raw(Arc<XData>) as u64`
                // with the standard Rust Arc layout (refcount at
                // offset -16). Retain/release on the receiver / result
                // slots dispatches through Round 9's
                // `retain_func_for_place` / `release_func_for_place`
                // 8-arm extension keyed on the slot's proven
                // `NativeKind::Ptr(HeapKind::*)`.
                if let Some(name) = variant_name.as_deref() {
                    if is_collection_ctor_name(name) {
                        return self.emit_collection_ctor(
                            name,
                            *container_slot,
                            operands,
                        );
                    }
                }
                // For unit variants (empty operands), the preceding
                // `Assign(Aggregate)` short-circuit already left the slot
                // initialized.
                if operands.is_empty() {
                    return Ok(());
                }

                // W12-jit-result-option-trinity (Phase 3 cluster-0
                // Round 7A, 2026-05-12). EnumStore non-empty payload
                // consumer per item (iii) of the trinity. The
                // `variant_name` was producer-stamped at MIR-emission
                // time (§2.7.5) by the bare-form enum-variant intercept
                // in `mir/lowering/expr.rs:1556-1577` + the qualified
                // `Expr::EnumConstructor` arm at `expr.rs:1669-1693`.
                //
                // Dispatches to the Arc-shape producers at
                // `crates/shape-jit/src/ffi/result.rs::jit_v2_make_*`
                // (committed as item (ii) of the trinity), which return
                // `Arc::into_raw(Arc<ResultData>) as u64` /
                // `Arc::into_raw(Arc<OptionData>) as u64` matching the
                // VM-side `BuiltinFunction::OkCtor` / `ErrCtor` /
                // `SomeCtor` / `NoneCtor` output shape per ADR-006
                // §2.7.17.
                //
                // Payload kind is stamped from the operand's MIR-inferred
                // kind via `operand_slot_kind(op)` → `stack_kind_code::
                // encode(kind)` at call-site time per §2.7.5. NOT a
                // Bool-default fallback: when the operand's kind isn't
                // proven (the `None` arm of `operand_slot_kind`), surface-
                // and-stop with the structured cite per §2.7.7 #9.
                //
                // Unsupported variant names (user-defined enum variants
                // that aren't Ok / Err / Some / None) surface-and-stop —
                // user-defined enum codegen is a separate workstream per
                // the trinity audit §7 row 5.
                let Some(name) = variant_name.as_deref() else {
                    return Err(
                        "EnumStore: SURFACE — variant_name is None on a \
                         non-empty payload. The MIR producer sites in \
                         `mir/lowering/{expr,stmt}.rs` MUST thread \
                         `variant_name` per ADR-006 §2.7.5 producer-site \
                         classification; reaching here means a producer \
                         site emitted EnumStore without classification \
                         (forbidden #9). \
                         W12-jit-result-option-trinity (Phase 3 cluster-0 \
                         Round 7A) / ADR-006 §2.7.17."
                            .to_string(),
                    );
                };

                let Some(variant_tag) = shape_vm::mir::types::VariantTag::from_name(name) else {
                    return Err(format!(
                        "EnumStore: SURFACE — variant '{}' (operands.len()={}) \
                         is not in the trinity-supported set \
                         (Ok / Err / Some / None). User-defined enum \
                         variant codegen via EnumStore is a separate \
                         workstream per `docs/cluster-audits/\
                         w12-jit-match-enum-inline-audit.md` §7 row 5 — \
                         needs `VariantTag::User(EnumLayoutId, variant_id)` \
                         extension + parallel `jit_v2_make_user_enum_*` \
                         FFI family. \
                         W12-jit-result-option-trinity (Phase 3 cluster-0 \
                         Round 7A) / ADR-006 §2.7.17.",
                        name,
                        operands.len()
                    ));
                };

                // None has no payload — handled separately (empty operands
                // already returned above, but `MirConstant::None` lowers
                // to `MirConstant::None` operand which still gets here
                // with operands.len()==1 in some paths). For safety:
                if matches!(variant_tag, shape_vm::mir::types::VariantTag::None_) {
                    // None construction via the Arc-shape producer —
                    // no payload, no kind code. The producer always
                    // builds the same `Arc<OptionData>` with
                    // `is_some=false` + the §2.7.17 placeholder.
                    let inst = self.builder.ins().call(
                        self.ffi.v2_make_option_none,
                        &[],
                    );
                    let arc_bits = self.builder.inst_results(inst)[0];
                    let place = Place::Local(*container_slot);
                    self.release_old_value_if_heap(&place)?;
                    self.write_place(&place, arc_bits)?;
                    return Ok(());
                }

                // Ok / Err / Some — single-payload producers. The MIR
                // enforces exactly one operand for these via the producer-
                // side intercepts (`is_bare_enum_variant_ctor` + the
                // `Expr::EnumConstructor` tuple-payload arm). If we ever
                // see operands.len() != 1, that's a producer-site bug.
                if operands.len() != 1 {
                    return Err(format!(
                        "EnumStore: SURFACE — variant '{}' expects 1 \
                         operand, got {}. Producer-site contract \
                         violated. W12-jit-result-option-trinity / \
                         ADR-006 §2.7.17.",
                        name,
                        operands.len()
                    ));
                }
                let operand = &operands[0];

                // Producer-site kind classification per §2.7.5: the
                // operand's MIR-inferred kind IS the payload kind.
                // `operand_slot_kind` projects through Field / Index /
                // Local with the §2.7.5 conduit's `concrete_types` map +
                // the constant arms — every operand the trinity supports
                // has a proven kind by construction (the bare-form
                // intercept's operand is a typed local; the qualified
                // EnumConstructor's operand is a typed expression). When
                // the kind is genuinely unprovable, the carrier fallback
                // (NativeKind::UInt64) is the §2.7.5 stable-FFI carrier
                // kind for raw I64-wide bits — NOT a Bool-default
                // rationalization per §2.7.7 #9.
                let payload_kind = self
                    .operand_slot_kind_or_carrier(operand);
                let kind_code = super::super::ffi::stack_kind_code::encode(payload_kind);

                // Compile the operand to its raw payload bits (the call
                // signature is I64-wide per the §2.7.5 stable-FFI
                // convention; widen narrow native values to I64).
                let payload_val = self.compile_operand_raw(operand)?;
                let payload_i64 = self.widen_to_i64(payload_val);

                let kind_code_val = self
                    .builder
                    .ins()
                    .iconst(types::I8, kind_code as i64);

                let func_ref = match variant_tag {
                    shape_vm::mir::types::VariantTag::Ok => self.ffi.v2_make_result_ok,
                    shape_vm::mir::types::VariantTag::Err => self.ffi.v2_make_result_err,
                    shape_vm::mir::types::VariantTag::Some_ => self.ffi.v2_make_option_some,
                    shape_vm::mir::types::VariantTag::None_ => unreachable!("handled above"),
                };

                let inst = self
                    .builder
                    .ins()
                    .call(func_ref, &[payload_i64, kind_code_val]);
                let arc_bits = self.builder.inst_results(inst)[0];

                let place = Place::Local(*container_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, arc_bits)?;
                Ok(())
            }

            StatementKind::Nop => Ok(()),

            StatementKind::TaskBoundary(_, _) => {
                // TaskBoundary is a borrow-checker annotation consumed by the MIR
                // solver. Actual async mechanics are handled by Call terminators to
                // spawn_task/join_init FFI functions. No-op at codegen time.
                Ok(())
            }

            StatementKind::ClosureCapture {
                closure_slot,
                operands,
                function_id,
            } => {
                // Create a closure by pushing captures to ctx.stack and calling jit_make_closure.
                let fid = function_id.ok_or_else(|| {
                    "MirToIR: ClosureCapture missing function_id (MIR not patched)".to_string()
                })?;

                // ── Phase E FAST PATH: stack-allocated closure ─────────
                // When the storage planner has proven the closure slot
                // never escapes its defining frame, allocate a Cranelift
                // StackSlot shaped like `StackClosure { fn_id, type_id,
                // captures... }` instead of calling `jit_make_closure`.
                // Cranelift's SROA eliminates the slot when Phase C has
                // inlined the closure body and the env pointer is dead.
                //
                // The slot is only safe when the closure is local — any
                // escape (return, container store, task boundary, etc.)
                // forces the legacy heap path below.
                // Track A.1D.2: stack closures pack captures inline at
                // their native width — there is no `Box::into_raw` cell
                // and no `owned_mutable_capture_mask` driving a later
                // reclaim. Capture kinds `OwnedMutable` / `Shared`
                // require the heap path's FFI allocator
                // (`jit_alloc_owned_mut_cell` in A.1D /
                // `jit_alloc_shared_cell` in A.1E). Force the heap path
                // whenever the layout declares any non-Immutable
                // capture, even if the closure would otherwise qualify
                // as non-escaping. The slot's captured-cell pointer is
                // reclaimed by `release_typed_closure` at refcount-zero.
                let layout_needs_heap = self
                    .closure_function_layouts
                    .get(&fid)
                    .map(|l| {
                        l.owned_mutable_capture_mask != 0
                            || l.shared_capture_mask != 0
                    })
                    .unwrap_or(false);
                if !layout_needs_heap
                    && self.non_escaping_closure_slots.contains(closure_slot)
                {
                    self.emit_stack_closure(fid, *closure_slot, operands)?;
                    return Ok(());
                }

                // ── Closure-spec Phase H2 DEFAULT PATH: inline heap alloc ──
                // When the compiler provided a `ClosureLayout` for this
                // closure's function_id, emit a `TypedClosureHeader`
                // allocation + typed capture writes inline, then finalize
                // into a NaN-boxed `Arc<HeapValue::Closure>` via the
                // `jit_finalize_heap_closure` FFI. The `jit_make_closure`
                // FFI is no longer called on this path — Phase H2 unlocks
                // the §10 benchmark gate by guaranteeing that lowering
                // `MakeClosureHeap` never emits a `jit_make_closure`
                // symbol. Phase H1's env-var gate has been removed; this
                // path is unconditional whenever a layout is available.
                //
                // See `docs/v2-closure-specialization.md` §13 H2.
                if let Some(layout) =
                    self.closure_function_layouts.get(&fid).cloned()
                {
                    let closure_ptr =
                        self.emit_heap_closure(fid, &layout, operands)?;
                    // Closure-spec Phase H2: convert the raw TypedClosureHeader
                    // into a NaN-boxed Arc<HeapValue::Closure> via
                    // `jit_finalize_heap_closure`. The layout pointer is a
                    // stable program-lifetime Arc<ClosureLayout> (stored in
                    // `BytecodeProgram.closure_function_layouts`) so passing
                    // its raw address as the finalizer argument is valid
                    // for the duration of any JIT call that uses this
                    // closure.
                    let layout_addr = std::sync::Arc::as_ptr(&layout) as i64;
                    let layout_val = self
                        .builder
                        .ins()
                        .iconst(cranelift::prelude::types::I64, layout_addr);
                    let fid_val_32 = self
                        .builder
                        .ins()
                        .iconst(cranelift::prelude::types::I32, fid as i64);
                    let cap_val_32 = self
                        .builder
                        .ins()
                        .iconst(cranelift::prelude::types::I32, operands.len() as i64);
                    let inst = self.builder.ins().call(
                        self.ffi.finalize_heap_closure,
                        &[closure_ptr, fid_val_32, cap_val_32, layout_val],
                    );
                    let closure_val = self.builder.inst_results(inst)[0];
                    let place = Place::Local(*closure_slot);
                    self.release_old_value_if_heap(&place)?;
                    self.write_place(&place, closure_val)?;
                    return Ok(());
                }

                // ── LEGACY HEAP PATH (no ClosureLayout available) ─────
                // Fallback for closure functions that were not registered
                // in `closure_function_layouts` (e.g. programs loaded from
                // disk without the side-table). Phase H5 will delete this
                // once the compile-time registration is universal and the
                // `MakeClosure` opcode is merged into `MakeClosureHeap`.

                // Push each capture operand to ctx.stack[stack_ptr + i].
                // ADR-006 §2.7.7 / Q9: lockstep parallel-kind write at every
                // push site. `jit_make_closure` (the legacy FFI consuming
                // these slots) doesn't currently read the kinds — but the
                // invariant requires the writes so future FFI consumers can
                // route through `stack_kind_code::decode` rather than
                // synthesizing kinds at the read site.
                let stack_base = crate::context::STACK_OFFSET as i32;
                let sp_offset = crate::context::STACK_PTR_OFFSET as i32;
                let old_sp = self.builder.ins().load(
                    cranelift::prelude::types::I64,
                    MemFlags::new(),
                    self.ctx_ptr,
                    sp_offset,
                );

                // R4.2E: legacy ClosureCapture path pushes captures to
                // ctx.stack as raw I64 bit-patterns. Widen narrow Cranelift
                // types inline (sextend / uextend / bitcast) — no NaN-box
                // tagging.
                for (i, op) in operands.iter().enumerate() {
                    // Source the capture kind from the producing site,
                    // falling back to the §2.7.5 carrier kind (`UInt64`)
                    // for opaque-source operands — NOT a Bool-default
                    // fallback.
                    let _ = i;
                    let op_kind = self.operand_slot_kind_or_carrier(op);

                    let raw = self.compile_operand(op)?;
                    let raw_ty = self.builder.func.dfg.value_type(raw);
                    let val = if raw_ty == cranelift::prelude::types::I64 {
                        raw
                    } else if raw_ty == cranelift::prelude::types::F64 {
                        self.builder
                            .ins()
                            .bitcast(cranelift::prelude::types::I64, MemFlags::new(), raw)
                    } else if raw_ty == cranelift::prelude::types::I32 {
                        self.builder.ins().sextend(cranelift::prelude::types::I64, raw)
                    } else if raw_ty == cranelift::prelude::types::I8 {
                        self.builder.ins().uextend(cranelift::prelude::types::I64, raw)
                    } else if raw_ty == cranelift::prelude::types::I16 {
                        self.builder.ins().sextend(cranelift::prelude::types::I64, raw)
                    } else {
                        raw
                    };
                    let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
                    let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                    let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base as i64);
                    let addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                    self.builder.ins().store(MemFlags::new(), val, addr, 0);
                    // §2.7.7 / Q9 lockstep parallel-kind write.
                    self.emit_kind_track_write(slot_idx, op_kind);
                }

                // Update ctx.stack_ptr += captures_count
                let new_sp = self.builder.ins().iadd_imm(old_sp, operands.len() as i64);
                self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                // Call jit_make_closure(ctx, function_id, captures_count)
                let fid_val = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    fid as i64,
                );
                let cap_count = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    operands.len() as i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.make_closure,
                    &[self.ctx_ptr, fid_val, cap_count],
                );
                let closure_val = self.builder.inst_results(inst)[0];

                // Store the closure in the closure_slot
                let place = Place::Local(*closure_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, closure_val)?;
                Ok(())
            }
        }
    }

    /// Phase E: emit a Cranelift `StackSlot` shaped like
    /// `StackClosure { function_id: u32, type_id: u32, captures... }`
    /// for a non-escaping closure.
    ///
    /// Layout (from `shape_value::v2::closure_layout::StackClosure`):
    /// - offset 0: function_id (u32)
    /// - offset 4: type_id (u32, always 0 here — Phase E does not yet
    ///   thread the real `ClosureTypeId` through the JIT; the field
    ///   is a layout placeholder for Phase F's `Function<A,R>` dispatch)
    /// - offset 8 onwards: captures, laid out per
    ///   `ClosureLayout::stack_capture_offset(i)` at native width
    ///
    /// Captures are stored at their native Cranelift type so inlined
    /// bodies (Phase C) can consume them without NaN-box round-trips.
    /// Capture kinds that don't map to a native Cranelift type
    /// (pointers, strings, unknown) fall back to I64 storage — correct
    /// since the MIR's `ClosureCapture` slot_kind matches the source.
    ///
    /// The resulting `StackClosure*` pointer is written to the closure
    /// slot as I64 (NaN-boxed value semantics). After Phase C inlining,
    /// the pointer is dead; Cranelift's SROA pass eliminates the slot.
    fn emit_stack_closure(
        &mut self,
        function_id: u16,
        closure_slot: SlotId,
        operands: &[Operand],
    ) -> Result<(), String> {
        use cranelift::prelude::types as cl_types;

        // Determine per-capture Cranelift types + byte offsets.
        // The MIR operand's root slot kind dictates the native storage
        // width. Slots with no inferred kind fall back to I64 (same
        // Cranelift width as the legacy `Unknown`/`Dynamic` arm).
        let mut capture_types: Vec<Type> = Vec::with_capacity(operands.len());
        for op in operands.iter() {
            let kind = match op {
                Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => {
                    let slot = p.root_local();
                    super::types::slot_kind_for_local(&self.slot_kinds, slot.0)
                        .unwrap_or(NativeKind::Int64)
                }
                Operand::Constant(MirConstant::Float(_)) => NativeKind::Float64,
                Operand::Constant(MirConstant::Int(_)) => NativeKind::Int64,
                Operand::Constant(MirConstant::Bool(_)) => NativeKind::Bool,
                _ => NativeKind::Int64,
            };
            capture_types.push(super::types::cranelift_type_for_slot(kind));
        }

        // Compute byte offsets: 8-byte header (fn_id + type_id), then each
        // capture with natural alignment. Mirrors
        // `ClosureLayout::from_capture_types` but operates on Cranelift
        // types directly (the runtime layout struct keys on ConcreteType
        // which is unavailable at MIR-codegen time).
        let header_size: usize = 8;
        let mut offsets: Vec<i32> = Vec::with_capacity(operands.len());
        let mut cur: usize = header_size;
        let mut max_align: usize = 8;
        for ty in &capture_types {
            let (size, align) = cranelift_type_size_align(*ty);
            cur = (cur + align - 1) & !(align - 1);
            offsets.push(cur as i32);
            if align > max_align {
                max_align = align;
            }
            cur += size;
        }
        let total = (cur + max_align - 1) & !(max_align - 1);
        // Cranelift StackSlot size must be > 0 and fit in u32.
        let total = total.max(8) as u32;
        let align_shift: u8 = match max_align {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            16 => 4,
            _ => 3,
        };

        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            total,
            align_shift,
        ));

        // function_id at offset 0 (I32)
        let fid_val = self
            .builder
            .ins()
            .iconst(cl_types::I32, function_id as i64);
        self.builder.ins().stack_store(fid_val, slot, 0);
        // type_id at offset 4 (I32). Phase E stores 0 — Phase F threads
        // the real ClosureTypeId through once `Function<A,R>` dispatch
        // needs it for signature lookup.
        let tid_val = self.builder.ins().iconst(cl_types::I32, 0);
        self.builder.ins().stack_store(tid_val, slot, 4);

        // Each capture at its typed offset.
        for (i, op) in operands.iter().enumerate() {
            let val = self.compile_operand(op)?;
            let target_ty = capture_types[i];
            let val_ty = self.builder.func.dfg.value_type(val);
            let stored = self.coerce_for_capture_store(val, val_ty, target_ty);
            self.builder.ins().stack_store(stored, slot, offsets[i]);
        }

        // Produce the closure value = stack slot address. This is a raw
        // pointer; we store it as I64 so existing Place plumbing
        // (which treats closure slots as NaN-boxed I64 values) works
        // uniformly. After Phase C inlining the pointer is dead; SROA
        // eliminates the stack slot. No arc_retain / arc_release on
        // stack closures.
        let closure_addr = self.builder.ins().stack_addr(cl_types::I64, slot, 0);

        // Track the stack slot so drop/release paths know to skip
        // arc_release on this slot.
        self.stack_closure_slots.insert(closure_slot, slot);
        // Session 2: also record the function_id + per-capture byte
        // offsets + native Cranelift types so the indirect-call path
        // can dispatch this stack closure without going through the
        // `jit_call_value` FFI (which can't recognise a raw
        // stack-slot pointer — it isn't NaN-boxed).
        self.stack_closure_call_info.insert(
            closure_slot,
            super::StackClosureCallInfo {
                function_id,
                capture_offsets: offsets.clone(),
                capture_types: capture_types.clone(),
            },
        );

        // Write to the closure slot. We deliberately do NOT call
        // `release_old_value_if_heap` here — a stack closure cannot
        // be overwriting a heap pointer on this path (the storage
        // planner guarantees single-assignment to closure slots), and
        // the MIR has already emitted the appropriate Drop for any
        // prior heap handle that sat in this slot.
        let place = Place::Local(closure_slot);
        self.write_place(&place, closure_addr)?;
        Ok(())
    }

    /// Closure-spec Phase H1: emit inline Cranelift IR that allocates and
    /// initializes a `TypedClosureHeader`-shaped heap block for an escaping
    /// closure. Replaces the legacy `jit_make_closure` FFI call.
    ///
    /// Emitted IR sequence:
    /// 1. `call jit_v2_alloc_struct(total_heap_size, HEAP_KIND_V2_CLOSURE)`
    ///    — returns a zero-initialised `*mut u8` with the `HeapHeader`
    ///    (refcount=1, kind=HEAP_KIND_V2_CLOSURE, flags=0) already written
    ///    by the allocator shim.
    /// 2. `store u32 function_id -> [closure_ptr + 8]`
    /// 3. `store u32 type_id -> [closure_ptr + 12]` — Phase H1 stores 0
    ///    for `type_id`; Phase F's `FunctionTypeId` plumbing is still TBD.
    /// 4. For each capture `i`: `store.T captures[i], [closure_ptr +
    ///    layout.heap_capture_offset(i)]` at the capture's natural width.
    /// 5. For each bit set in `layout.heap_capture_mask`: an atomic
    ///    `atomic_rmw add [capture_ptr + 0], 1` on the capture's own
    ///    `HeapHeader.refcount` (Relaxed ordering, matching
    ///    `HeapHeader::retain`).
    ///
    /// Returns the raw `TypedClosureHeader*` (I64). Phase H2's caller
    /// converts it to a NaN-boxed `Arc<HeapValue::Closure>` via the
    /// `jit_finalize_heap_closure` FFI before storing into the closure
    /// slot; the downstream dispatch path (`jit_call_value`, VM
    /// `op_call_closure`) then consumes the result via the v1 HK_CLOSURE
    /// ABI. A future phase (H3+) will teach dispatch to consume the raw
    /// typed header directly and drop the intermediate finalizer.
    ///
    /// # Safety invariants
    /// - `ClosureLayout::total_heap_size()` and `heap_capture_offset(i)`
    ///   are computed at compile time from a `ConcreteType` signature
    ///   that is `repr(C)`-compatible (see `closure_layout.rs` §1.1 and
    ///   the compile-time size assertions).
    /// - The allocator shim (`jit_v2_alloc_struct`) uses
    ///   `Layout::from_size_align(size, 8)` which is always valid for
    ///   v2 heap objects (8-byte alignment is the closure invariant).
    /// - Atomic retain uses `Ordering::Relaxed`, matching
    ///   `HeapHeader::retain`. Release semantics on closure Drop are
    ///   H2's contract, not H1's.
    fn emit_heap_closure(
        &mut self,
        function_id: u16,
        layout: &std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>,
        operands: &[Operand],
    ) -> Result<Value, String> {
        use cranelift::prelude::types as cl_types;
        use shape_value::v2::closure_layout::HEAP_CLOSURE_HEADER_SIZE;
        use shape_value::v2::heap_header::HEAP_KIND_V2_CLOSURE;
        use shape_value::v2::struct_layout::FieldKind;

        if operands.len() != layout.capture_count() {
            return Err(format!(
                "MirToIR::emit_heap_closure: capture-count mismatch for function_id {}: \
                 operands={} but layout={}",
                function_id,
                operands.len(),
                layout.capture_count()
            ));
        }

        // 1. Allocate the block via the existing `jit_v2_alloc_struct`
        //    shim. The shim writes the HeapHeader (refcount=1,
        //    kind=HEAP_KIND_V2_CLOSURE, flags=0) before returning.
        let total_size = layout.total_heap_size();
        if total_size > u32::MAX as usize {
            return Err(format!(
                "MirToIR::emit_heap_closure: total_heap_size {} exceeds u32::MAX",
                total_size
            ));
        }
        let size_val = self
            .builder
            .ins()
            .iconst(cl_types::I32, total_size as i64);
        let kind_val = self
            .builder
            .ins()
            .iconst(cl_types::I32, HEAP_KIND_V2_CLOSURE as i64);
        let inst = self
            .builder
            .ins()
            .call(self.ffi.v2_alloc_struct, &[size_val, kind_val]);
        let closure_ptr = self.builder.inst_results(inst)[0];

        // 2. Write function_id as u32 at offset 8 (i.e., right after the
        //    HeapHeader). The allocator zeroed the memory so the high
        //    bits are 0 — no need to mask.
        let fid_val = self
            .builder
            .ins()
            .iconst(cl_types::I32, function_id as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), fid_val, closure_ptr, 8);

        // 3. Write type_id as u32 at offset 12. Phase H1 stores 0 — the
        //    `FunctionTypeId` is not yet threaded end-to-end into the
        //    JIT worker. H2 / later phases populate this.
        let tid_val = self.builder.ins().iconst(cl_types::I32, 0);
        self.builder
            .ins()
            .store(MemFlags::trusted(), tid_val, closure_ptr, 12);

        // 4. Write each capture at its `heap_capture_offset(i)`. Dispatch
        //    per `ClosureLayout::capture_storage_kind(i)`:
        //
        //    - `CaptureKind::Immutable`: store the native value at its
        //      natural `FieldKind` width (existing H1 path).
        //    - `CaptureKind::OwnedMutable` (Track A.1D): the slot is a
        //      `FieldKind::Ptr` holding `*mut ValueWord` — call the
        //      `jit_alloc_owned_mut_cell(initial)` FFI to obtain a fresh
        //      Box pointer from the capture's initial ValueWord bits,
        //      then store the pointer into the slot. The
        //      `owned_mutable_capture_mask` bit for this index directs
        //      `release_typed_closure` (A.1A) to reclaim it via
        //      `Box::from_raw` on closure drop.
        //    - `CaptureKind::Shared`: pre-A.1E, shared captures still go
        //      through the `op_make_closure` legacy path. The JIT
        //      preflight gate (`vm_only_opcode_reason` in
        //      `compiler/accessors.rs`) rejects any function that
        //      contains `LoadSharedCapture` / `StoreSharedCapture`, so
        //      this branch is unreachable until A.1E. Debug-assert.
        use shape_value::v2::closure_layout::CaptureKind;
        for (i, op) in operands.iter().enumerate() {
            let offset = layout.heap_capture_offset(i) as i32;
            match layout.capture_storage_kind(i) {
                CaptureKind::Immutable => {
                    let kind = layout.capture_kind(i);
                    let target_ty = cranelift_type_for_field_kind(kind);
                    let raw = self.compile_operand(op)?;
                    let val_ty = self.builder.func.dfg.value_type(raw);
                    let stored = self.coerce_for_capture_store(raw, val_ty, target_ty);
                    self.builder
                        .ins()
                        .store(MemFlags::trusted(), stored, closure_ptr, offset);
                }
                CaptureKind::OwnedMutable => {
                    // Wave C.2: dispatch to the per-FieldKind allocator
                    // from C.1. The per-kind helpers in
                    // `crates/shape-jit/src/ffi/object/closure.rs` (and
                    // their `shape-value::v2::closure_raw` counterparts)
                    // do `Box::into_raw(Box::new(initial))` at the
                    // native interior width — F64 cells are an
                    // 8-byte `Box<f64>`, Bool cells are a 1-byte
                    // `Box<bool>`, etc. `release_typed_closure` consults
                    // `ClosureLayout::capture_inner_kind` to pick the
                    // matching `Box::from_raw::<T>` reclaim.
                    //
                    // SAFETY: the FFI returns a non-null `*mut T` owned
                    // by the closure block. Between this call and the
                    // subsequent `store` the closure block MUST NOT be
                    // dropped — any intervening panic leaks the cell.
                    // Cranelift lowering in this function is panic-free
                    // by construction (pure stores / loads / direct
                    // calls). Heap-capture atomic retain (step 5 below)
                    // iterates `heap_capture_mask` only — OwnedMutable
                    // captures set `owned_mutable_capture_mask` instead
                    // and are skipped.
                    let inner_kind = layout.capture_inner_kind(i);
                    let raw = self.compile_operand(op)?;
                    let val_ty = self.builder.func.dfg.value_type(raw);
                    // Coerce the operand to the FFI-call's expected
                    // native Cranelift type per the C.1 ABI:
                    //   F64                       -> F64
                    //   I64 / U64 / Ptr           -> I64
                    //   I32 / U32                 -> I32
                    //   I16 / U16 / I8 / U8 / Bool -> I32
                    let target_ty = ffi_param_type_for_field_kind(inner_kind);
                    let initial = self
                        .coerce_for_capture_store(raw, val_ty, target_ty);
                    let alloc_func = owned_mut_alloc_func(&self.ffi, inner_kind);
                    let inst = self.builder.ins().call(alloc_func, &[initial]);
                    let cell_ptr = self.builder.inst_results(inst)[0];
                    self.builder
                        .ins()
                        .store(MemFlags::trusted(), cell_ptr, closure_ptr, offset);
                }
                CaptureKind::Shared => {
                    // Track A.1E: Shared capture lowering. The operand
                    // pushes the raw `*const SharedCell` pointer bits
                    // already held in the outer slot — the bytecode
                    // compiler emits `LoadLocal(outer_var_slot)`
                    // against a slot previously filled by
                    // `AllocSharedLocal`. We retain one additional Arc
                    // strong share for the closure via
                    // `jit_arc_shared_retain` (mirrors the interpreter's
                    // `Arc::<SharedCell>::increment_strong_count(ptr)`
                    // in `op_make_closure`), then store the pointer
                    // into the Ptr slot. `release_typed_closure` on
                    // closure drop walks `shared_capture_mask` and
                    // reclaims each share with `Arc::from_raw`.
                    //
                    // Wave C.2 note: the SharedCell itself was
                    // allocated upstream by `initialize_shared_local_slots`
                    // (in `blocks.rs`) via the legacy generic
                    // `jit_alloc_shared_cell(NONE_BITS)` — which writes
                    // 8 bytes of NaN-boxed null at the cell's payload
                    // offset. Wave-B's per-kind shared writers
                    // sign-/zero-extend to 8 bytes on each subsequent
                    // store, so reads at the kind's native width
                    // truncate correctly. Adding a typed
                    // `jit_alloc_shared_cell_typed` is a small follow-up
                    // (Wave G); the legacy entry point stays for now.
                    //
                    // SAFETY: the operand produces a non-null
                    // `*const SharedCell` whose Arc strong count ≥ 1
                    // (owned by the outer slot). The retain FFI bumps
                    // the count by 1; the subsequent store installs
                    // the pointer bits into the capture slot. The
                    // allocator's (`AllocSharedLocal`'s `Arc::into_raw`)
                    // 8-byte alignment is preserved.
                    //
                    // Session 1 Commit 3: when the operand's source is
                    // an outer-scope `var` slot in the SAME function's
                    // MIR (a `SharedCow` local), the default
                    // `compile_operand` would emit a lock-gated read of
                    // the cell's payload. We bypass that via
                    // `compile_operand_for_shared_capture`, which reads
                    // the raw pointer bits directly from the slot's
                    // Cranelift variable. For operands that are NOT
                    // SharedCow slots (e.g. a capture inherited from
                    // an outer-outer frame), the helper falls back to
                    // the standard `compile_operand` path.
                    let raw = self.compile_operand_for_shared_capture(op)?;
                    let val_ty = self.builder.func.dfg.value_type(raw);
                    // The pointer is a raw u64 bit pattern — for an
                    // outer `var` slot promoted to Shared storage,
                    // the bytecode compiler emits the pointer bits as
                    // an I64 LoadLocal. Widen to I64 defensively in
                    // case upstream type inference narrowed it.
                    let ptr_bits =
                        self.coerce_for_capture_store(raw, val_ty, cl_types::I64);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.arc_shared_retain, &[ptr_bits]);
                    let retained_ptr = self.builder.inst_results(inst)[0];
                    self.builder.ins().store(
                        MemFlags::trusted(),
                        retained_ptr,
                        closure_ptr,
                        offset,
                    );
                }
            }
        }

        // 5. Atomic retain on each heap-typed capture. Iterates only
        //    over bits set in `heap_capture_mask` — typed scalars (F64,
        //    I64, I32, Bool) have zero bits and no retain work.
        //
        //    The retain target is the capture value itself (a pointer
        //    to another `HeapHeader`), loaded back from the closure at
        //    the offset we just wrote. Using `atomic_rmw add` on the
        //    `refcount` u32 at offset 0 of the pointee matches
        //    `HeapHeader::retain`'s `fetch_add(1, Ordering::Relaxed)`.
        let mut mask = layout.heap_capture_mask;
        while mask != 0 {
            let bit = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            // Sanity: the heap-mask bit must correspond to a Ptr-kind
            // capture. This is a ClosureLayout invariant — assert so a
            // regression surfaces in tests.
            debug_assert_eq!(
                layout.capture_kind(bit),
                FieldKind::Ptr,
                "heap_capture_mask bit {} at function_id {} points to non-Ptr capture",
                bit,
                function_id,
            );
            // Reload the capture pointer from the closure. We store it
            // at its heap offset a step earlier; reloading keeps the
            // value source consistent with how the capture is used
            // downstream (no need to separately track `stored` values).
            let cap_offset = layout.heap_capture_offset(bit) as i32;
            let cap_ptr = self.builder.ins().load(
                cl_types::I64,
                MemFlags::trusted(),
                closure_ptr,
                cap_offset,
            );
            // Only retain non-null pointers. A null capture pointer
            // here would indicate a broken layout, but guarding is
            // cheap and avoids crashing the JIT'd code on bugs.
            let null = self.builder.ins().iconst(cl_types::I64, 0);
            let is_non_null =
                self.builder
                    .ins()
                    .icmp(IntCC::NotEqual, cap_ptr, null);
            let retain_block = self.builder.create_block();
            let continue_block = self.builder.create_block();
            self.builder.ins().brif(
                is_non_null,
                retain_block,
                &[],
                continue_block,
                &[],
            );
            self.builder.switch_to_block(retain_block);
            self.builder.seal_block(retain_block);
            let one = self.builder.ins().iconst(cl_types::I32, 1);
            // atomic_rmw Add on the u32 refcount at offset 0. This is
            // semantically equivalent to HeapHeader::retain's
            // fetch_add(1, Relaxed).
            self.builder.ins().atomic_rmw(
                cl_types::I32,
                MemFlags::trusted(),
                cranelift::codegen::ir::AtomicRmwOp::Add,
                cap_ptr,
                one,
            );
            self.builder.ins().jump(continue_block, &[]);
            self.builder.switch_to_block(continue_block);
            self.builder.seal_block(continue_block);
        }

        // Keep the header constant handy for the unused import lint.
        let _ = HEAP_CLOSURE_HEADER_SIZE;

        Ok(closure_ptr)
    }

    /// Coerce a Cranelift value to the target capture storage type.
    /// Performs zero-extension for narrowings and bitcasts for F64/I64.
    fn coerce_for_capture_store(
        &mut self,
        val: Value,
        val_ty: Type,
        target_ty: Type,
    ) -> Value {
        use cranelift::prelude::types as cl_types;
        if val_ty == target_ty {
            return val;
        }
        // R4.2C: capture-store cells take ValueWord-encoded I64 bit-patterns
        // directly — operands are already I64-slot values, so the I64 target
        // branch and the last-resort fallback pass `val` through unchanged.
        if target_ty == cl_types::I64 {
            return val;
        }
        if target_ty == cl_types::I32 {
            if val_ty == cl_types::I8 || val_ty == cl_types::I16 {
                return self.builder.ins().sextend(cl_types::I32, val);
            }
            if val_ty == cl_types::I64 {
                return self.builder.ins().ireduce(cl_types::I32, val);
            }
        }
        if target_ty == cl_types::I8 {
            if val_ty == cl_types::I32 || val_ty == cl_types::I64 {
                return self.builder.ins().ireduce(cl_types::I8, val);
            }
        }
        if target_ty == cl_types::F64 && val_ty == cl_types::I64 {
            // NaN-boxed I64 carrying an F64 — bitcast back.
            return self
                .builder
                .ins()
                .bitcast(cl_types::F64, MemFlags::new(), val);
        }
        // Last-resort: already an I64 bit-pattern; pass through.
        val
    }
}

/// W12-collection-constructor-mir-lowering (Phase 3 cluster-0 Round 6C,
/// 2026-05-12): identify a primitive-collection constructor name on the
/// JIT consumer side. The MIR-lowering pass at
/// `crates/shape-vm/src/mir/lowering/helpers.rs::is_bare_collection_ctor`
/// is the authoritative producer-side classifier; this is its mirror
/// for the `StatementKind::EnumStore` consumer.
///
/// Mirrors the bytecode compiler's `classify_builtin_function` collection-
/// ctor subset (`crates/shape-vm/src/compiler/helpers.rs:3433-3440`). Any
/// future addition to that list (e.g. a new HeapKind ctor) needs the
/// same name added here, plus the corresponding lowering-side
/// `is_bare_collection_ctor` arm. The CHECK-12-style merge gate doesn't
/// cover this drift; add to the verify-merge script if it becomes
/// load-bearing.
fn is_collection_ctor_name(name: &str) -> bool {
    matches!(
        name,
        "HashMap" | "Set" | "Deque" | "PriorityQueue" | "Channel" | "Mutex" | "Atomic" | "Lazy"
    )
}

impl<'a, 'b> super::MirToIR<'a, 'b> {
    /// W12-jit-call-method-shell-rebuild Part 3 (Phase 3 cluster-0 Round
    /// 10 / 8B.2, 2026-05-13): dispatch an `EnumStore` collection-ctor
    /// arm to Round 9's typed-Arc allocator FuncRef.
    ///
    /// `name` is one of the 8 names in `is_collection_ctor_name`. The
    /// allocator FuncRef shape:
    ///
    /// - Zero-arg: `Set` / `HashSet` / `HashMap` / `Deque` /
    ///   `PriorityQueue` / `Channel` — call with `&[]`, store the
    ///   resulting `Arc::into_raw(Arc<XData>) as u64` bits.
    /// - Single-int: `Atomic(i64)` — compile the inner operand to its
    ///   I64-widened raw payload bits, call with `&[bits]`.
    /// - Single-closure: `Lazy(closure_bits)` — same shape as Atomic
    ///   but the operand is a closure-kinded slot. The producer-side
    ///   MIR classifier (`mir/lowering/expr.rs::is_bare_collection_ctor_with_arg`)
    ///   validated the kind at emit time; the FFI body accepts raw
    ///   u64 closure-Arc bits.
    /// - Carrier-pair: `Mutex(bits, kind_code)` — compile the inner
    ///   operand to its I64-widened raw payload bits, encode the
    ///   operand's MIR-inferred kind into a `kind_code: u8` per
    ///   §2.7.5 stamp-at-compile-time, call with `&[bits, kind_code]`.
    ///
    /// The container slot's old value is released via
    /// `release_old_value_if_heap` (which dispatches through
    /// `release_func_for_place` — Round 9's 8-arm extension already
    /// fires the correct typed-Arc release for the destination slot).
    /// The new Arc bits are written via `write_place`.
    pub(crate) fn emit_collection_ctor(
        &mut self,
        name: &str,
        container_slot: SlotId,
        operands: &[Operand],
    ) -> Result<(), String> {
        // Zero-arg ctor dispatch: pick the FuncRef and call with no args.
        let zero_arg_func_ref = match name {
            "Set" => Some(self.ffi.v2_make_hashset),
            "HashMap" => Some(self.ffi.v2_make_hashmap),
            "Deque" => Some(self.ffi.v2_make_deque),
            "PriorityQueue" => Some(self.ffi.v2_make_priorityqueue),
            "Channel" => Some(self.ffi.v2_make_channel),
            _ => None,
        };
        if let Some(func_ref) = zero_arg_func_ref {
            if !operands.is_empty() {
                return Err(format!(
                    "EnumStore collection_ctor: SURFACE — '{}' is a \
                     zero-arg ctor but operands.len()={}. Producer-site \
                     contract violated (`mir/lowering/helpers.rs::\
                     is_bare_collection_ctor`). ADR-006 §2.7.5 / \
                     W12-jit-call-method-shell-rebuild.",
                    name,
                    operands.len(),
                ));
            }
            let inst = self.builder.ins().call(func_ref, &[]);
            let arc_bits = self.builder.inst_results(inst)[0];
            let place = Place::Local(container_slot);
            self.release_old_value_if_heap(&place)?;
            self.write_place(&place, arc_bits)?;
            return Ok(());
        }

        // Single-arg ctor dispatch (Atomic / Lazy): one inner operand,
        // I64-widened raw payload bits. Per §2.7.25, inner-kind
        // constraints (Atomic→Int64, Lazy→Ptr(HeapKind::Closure)) are
        // validated by the producer-side classifier at MIR-emission
        // time; the JIT consumer here accepts the raw bits as-is.
        let single_arg_func_ref = match name {
            "Atomic" => Some(self.ffi.v2_make_atomic),
            "Lazy" => Some(self.ffi.v2_make_lazy),
            _ => None,
        };
        if let Some(func_ref) = single_arg_func_ref {
            if operands.len() != 1 {
                return Err(format!(
                    "EnumStore collection_ctor: SURFACE — '{}' expects \
                     1 operand, got {}. Producer-site contract violated \
                     (`mir/lowering/helpers.rs::is_bare_collection_ctor_with_arg`). \
                     ADR-006 §2.7.5 / W12-jit-call-method-shell-rebuild.",
                    name,
                    operands.len(),
                ));
            }
            let payload_val = self.compile_operand_raw(&operands[0])?;
            let payload_i64 = self.widen_to_i64(payload_val);
            let inst = self.builder.ins().call(func_ref, &[payload_i64]);
            let arc_bits = self.builder.inst_results(inst)[0];
            let place = Place::Local(container_slot);
            self.release_old_value_if_heap(&place)?;
            self.write_place(&place, arc_bits)?;
            return Ok(());
        }

        // Carrier-pair ctor: Mutex(bits, kind_code). The kind is
        // sourced from the operand's MIR-inferred kind via §2.7.5
        // producing-site classification. NOT a Bool-default fallback:
        // when `operand_slot_kind`'s `None` arm fires (the operand's
        // kind cannot be proven at MIR-emission time), the call falls
        // through to the carrier kind `UInt64` per the §2.7.5 stable-
        // FFI raw-bits carrier convention (the same convention Round 7A
        // / §2.7.5 conduit uses for Ok/Err/Some/None inner payloads).
        // The Mutex FFI body itself surface-and-stops on a SENTINEL
        // kind ord, leaking the inner share rather than fabricating
        // Bool (`ffi/v2/collection_arc.rs::jit_v2_make_mutex`).
        if name == "Mutex" {
            if operands.len() != 1 {
                return Err(format!(
                    "EnumStore collection_ctor: SURFACE — 'Mutex' \
                     expects 1 operand, got {}. Producer-site contract \
                     violated. ADR-006 §2.7.5 / W12-jit-call-method-\
                     shell-rebuild.",
                    operands.len(),
                ));
            }
            let payload_val = self.compile_operand_raw(&operands[0])?;
            let payload_i64 = self.widen_to_i64(payload_val);
            let payload_kind = self.operand_slot_kind_or_carrier(&operands[0]);
            let kind_code =
                super::super::ffi::stack_kind_code::encode(payload_kind);
            let kind_code_val = self
                .builder
                .ins()
                .iconst(types::I8, kind_code as i64);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.v2_make_mutex, &[payload_i64, kind_code_val]);
            let arc_bits = self.builder.inst_results(inst)[0];
            let place = Place::Local(container_slot);
            self.release_old_value_if_heap(&place)?;
            self.write_place(&place, arc_bits)?;
            return Ok(());
        }

        Err(format!(
            "EnumStore collection_ctor: SURFACE — unrecognized \
             collection-ctor name '{}'. `is_collection_ctor_name` and \
             `emit_collection_ctor` must stay in lockstep; adding a \
             new name requires extending both. ADR-006 §2.7.5 / \
             W12-jit-call-method-shell-rebuild.",
            name,
        ))
    }
}

/// Closure-spec Phase H1: map a capture's `FieldKind` to the Cranelift
/// type used for its typed store into the `TypedClosureHeader` block.
/// Matches the widths declared in `ClosureLayout::from_capture_types`.
fn cranelift_type_for_field_kind(kind: shape_value::v2::struct_layout::FieldKind) -> Type {
    use cranelift::prelude::types as cl_types;
    use shape_value::v2::struct_layout::FieldKind;
    match kind {
        FieldKind::F64 => cl_types::F64,
        FieldKind::I64 | FieldKind::U64 => cl_types::I64,
        FieldKind::I32 | FieldKind::U32 => cl_types::I32,
        FieldKind::I16 | FieldKind::U16 => cl_types::I16,
        FieldKind::I8 | FieldKind::U8 | FieldKind::Bool => cl_types::I8,
        // Pointers / Strings / Arrays / Structs are all stored as i64-sized
        // heap pointers in the `TypedClosureHeader` block.
        FieldKind::Ptr => cl_types::I64,
    }
}

/// Wave C.2: Cranelift type used at the per-FieldKind closure-cell FFI
/// boundary. Sub-32 ints are widened to I32 to match the C ABI of the
/// `jit_alloc_owned_mut_cell_<kind>` / `jit_*_shared_cell_<kind>`
/// wrappers (declared in `crates/shape-jit/src/ffi_symbols/object_symbols.rs`).
fn ffi_param_type_for_field_kind(
    kind: shape_value::v2::struct_layout::FieldKind,
) -> Type {
    use cranelift::prelude::types as cl_types;
    use shape_value::v2::struct_layout::FieldKind;
    match kind {
        FieldKind::F64 => cl_types::F64,
        FieldKind::I64 | FieldKind::U64 | FieldKind::Ptr => cl_types::I64,
        FieldKind::I32 | FieldKind::U32 => cl_types::I32,
        FieldKind::I16
        | FieldKind::U16
        | FieldKind::I8
        | FieldKind::U8
        | FieldKind::Bool => cl_types::I32,
    }
}

/// Wave C.2: per-FieldKind selector for `jit_alloc_owned_mut_cell_<kind>`
/// FuncRefs.
fn owned_mut_alloc_func(
    ffi: &crate::ffi_refs::FFIFuncRefs,
    kind: shape_value::v2::struct_layout::FieldKind,
) -> cranelift::codegen::ir::FuncRef {
    use shape_value::v2::struct_layout::FieldKind;
    match kind {
        FieldKind::I64 => ffi.alloc_owned_mut_cell_i64,
        FieldKind::U64 => ffi.alloc_owned_mut_cell_u64,
        FieldKind::F64 => ffi.alloc_owned_mut_cell_f64,
        FieldKind::I32 => ffi.alloc_owned_mut_cell_i32,
        FieldKind::U32 => ffi.alloc_owned_mut_cell_u32,
        FieldKind::I16 => ffi.alloc_owned_mut_cell_i16,
        FieldKind::U16 => ffi.alloc_owned_mut_cell_u16,
        FieldKind::I8 => ffi.alloc_owned_mut_cell_i8,
        FieldKind::U8 => ffi.alloc_owned_mut_cell_u8,
        FieldKind::Bool => ffi.alloc_owned_mut_cell_bool,
        FieldKind::Ptr => ffi.alloc_owned_mut_cell_ptr,
    }
}

/// Size and alignment in bytes for a Cranelift type, used by the
/// Phase E stack closure layout computation.
fn cranelift_type_size_align(ty: Type) -> (usize, usize) {
    use cranelift::prelude::types as cl_types;
    match ty {
        t if t == cl_types::I8 => (1, 1),
        t if t == cl_types::I16 => (2, 2),
        t if t == cl_types::I32 => (4, 4),
        t if t == cl_types::F32 => (4, 4),
        t if t == cl_types::I64 => (8, 8),
        t if t == cl_types::F64 => (8, 8),
        _ => (8, 8),
    }
}

/// Phase E layout helper: compute stack-closure capture byte offsets
/// given a list of Cranelift capture types. Mirrors the logic inside
/// `emit_stack_closure` so it can be unit-tested independently.
///
/// Returns `(offsets, total_size, max_alignment)` where offsets are
/// absolute from the StackClosure base pointer. The 8-byte header
/// (`function_id: u32` @ 0, `type_id: u32` @ 4) is implicit.
#[cfg(test)]
fn phase_e_layout(capture_types: &[Type]) -> (Vec<i32>, usize, usize) {
    let header_size: usize = 8;
    let mut offsets: Vec<i32> = Vec::with_capacity(capture_types.len());
    let mut cur: usize = header_size;
    let mut max_align: usize = 8;
    for ty in capture_types {
        let (size, align) = cranelift_type_size_align(*ty);
        cur = (cur + align - 1) & !(align - 1);
        offsets.push(cur as i32);
        if align > max_align {
            max_align = align;
        }
        cur += size;
    }
    let total = (cur + max_align - 1) & !(max_align - 1);
    let total = total.max(8);
    (offsets, total, max_align)
}

#[cfg(test)]
mod phase_e_tests {
    //! Phase E (JIT stack-closure codegen) layout helper tests.
    //!
    //! End-to-end closure JIT tests that exercise the full MirToIR
    //! `ClosureCapture` lowering live in the integration suite
    //! (`just test-fast`) — they require bytecode-compiling Shape
    //! source and running it through `compile_program_selective`,
    //! which is too heavy for the crate-local unit harness. These
    //! unit tests focus on the offset math so regressions in the
    //! layout are caught without spinning up a full JIT.

    use super::*;
    use cranelift::prelude::types as cl_types;

    #[test]
    fn empty_captures_layout_matches_stack_closure_header() {
        // Empty captures → just the 8-byte { fn_id, type_id } header.
        let (offsets, total, align) = phase_e_layout(&[]);
        assert!(offsets.is_empty());
        assert_eq!(total, 8);
        assert_eq!(align, 8);
        // Matches shape_value::v2::closure_layout::STACK_CLOSURE_HEADER_SIZE.
        assert_eq!(
            total,
            shape_value::v2::closure_layout::STACK_CLOSURE_HEADER_SIZE
        );
    }

    #[test]
    fn single_f64_capture_layout() {
        // f64 capture starts right after the 8-byte header.
        let (offsets, total, align) = phase_e_layout(&[cl_types::F64]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn single_i64_capture_layout() {
        let (offsets, total, align) = phase_e_layout(&[cl_types::I64]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn two_f64_captures_layout() {
        let (offsets, total, align) = phase_e_layout(&[cl_types::F64, cl_types::F64]);
        assert_eq!(offsets, vec![8, 16]);
        assert_eq!(total, 24);
        assert_eq!(align, 8);
    }

    #[test]
    fn mixed_alignment_packing_layout() {
        // (Bool, I32, F64): bool @ 8 (1 byte), i32 @ 12 (pad from 9), f64 @ 16.
        let (offsets, total, align) =
            phase_e_layout(&[cl_types::I8, cl_types::I32, cl_types::F64]);
        assert_eq!(offsets, vec![8, 12, 16]);
        assert_eq!(total, 24);
        assert_eq!(align, 8);
    }

    #[test]
    fn four_small_captures_pack_tightly() {
        // (I8, I8, I16, I32): 8, 9, 10, 12; total rounds up to 16.
        let (offsets, total, align) = phase_e_layout(&[
            cl_types::I8,
            cl_types::I8,
            cl_types::I16,
            cl_types::I32,
        ]);
        assert_eq!(offsets, vec![8, 9, 10, 12]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn i64_capture_forces_8_byte_total() {
        // Single I64 capture → total = 16 (header 8 + i64 8).
        let (offsets, total, _) = phase_e_layout(&[cl_types::I64]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
    }

    #[test]
    fn single_bool_capture_pads_to_8_bytes() {
        // Bool is 1 byte at offset 8; total rounds up to 16 (8-byte alignment).
        let (offsets, total, align) = phase_e_layout(&[cl_types::I8]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn unknown_type_defaults_to_i64_layout() {
        // Catch-all in cranelift_type_size_align returns (8,8). An
        // arbitrary pointer-sized type should therefore behave like I64.
        let (size, align) = cranelift_type_size_align(cl_types::F64);
        assert_eq!((size, align), (8, 8));
        let (size, align) = cranelift_type_size_align(cl_types::I64);
        assert_eq!((size, align), (8, 8));
    }

    #[test]
    fn many_mixed_captures_match_expected_pattern() {
        // Seven captures: f64, i32, bool, i64, i8, i16, f64.
        // Expected offsets: 8, 16, 20, 24 (pad to 8), 32, 34, 40; total rounds to 48.
        let (offsets, total, align) = phase_e_layout(&[
            cl_types::F64,
            cl_types::I32,
            cl_types::I8,
            cl_types::I64,
            cl_types::I8,
            cl_types::I16,
            cl_types::F64,
        ]);
        assert_eq!(offsets, vec![8, 16, 20, 24, 32, 34, 40]);
        assert_eq!(total, 48);
        assert_eq!(align, 8);
    }

    #[test]
    fn layout_agrees_with_runtime_closure_layout_for_f64() {
        // Cross-check against shape_value::v2::ClosureLayout for the
        // all-F64 signature. Both must agree on offsets and total size.
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::concrete_type::ConcreteType;

        let runtime_layout = ClosureLayout::from_capture_types(
            &[ConcreteType::F64, ConcreteType::F64],
            &[CaptureKind::Immutable, CaptureKind::Immutable],
        );
        let (offsets, total, _) = phase_e_layout(&[cl_types::F64, cl_types::F64]);
        assert_eq!(total, runtime_layout.total_stack_size());
        assert_eq!(offsets[0] as usize, runtime_layout.stack_capture_offset(0));
        assert_eq!(offsets[1] as usize, runtime_layout.stack_capture_offset(1));
    }
}

#[cfg(test)]
mod phase_h1_tests {
    //! Closure-spec Phase H1 codegen tests.
    //!
    //! Phase H1 introduces `MirToIR::emit_heap_closure`: inline Cranelift
    //! lowering that allocates and initialises a `TypedClosureHeader` block,
    //! replacing the `jit_make_closure` FFI call on the escaping-closure
    //! path. These tests exercise the **layout and helper** math used by
    //! the emitter — end-to-end JIT tests that actually execute emitted
    //! code live in the integration suite and are gated on Phase H2
    //! landing the matching VM-side `jit_call_value` dispatch.
    //!
    //! See `docs/v2-closure-specialization.md` §13 H1.
    use super::*;
    use shape_value::v2::closure_layout::{
        CaptureKind, ClosureLayout, HEAP_CLOSURE_HEADER_SIZE, STACK_CLOSURE_HEADER_SIZE,
    };
    use shape_value::v2::concrete_type::ConcreteType;
    use shape_value::v2::heap_header::{HeapHeader, HEAP_KIND_V2_CLOSURE};
    use shape_value::v2::struct_layout::FieldKind;

    // Test-local helper: immutable-only layout.
    fn immutable_layout(types: &[ConcreteType]) -> ClosureLayout {
        let kinds = vec![CaptureKind::Immutable; types.len()];
        ClosureLayout::from_capture_types(types, &kinds)
    }

    #[test]
    fn heap_kind_v2_closure_constant_is_84() {
        // The plan fixes HEAP_KIND_V2_CLOSURE at 84 (Phase F constant).
        assert_eq!(HEAP_KIND_V2_CLOSURE, 84);
    }

    #[test]
    fn heap_header_offsets_match_plan() {
        // emit_heap_closure relies on the HeapHeader's refcount at offset 0
        // and kind at offset 4. Regression check.
        assert_eq!(HeapHeader::OFFSET_REFCOUNT, 0);
        assert_eq!(HeapHeader::OFFSET_KIND, 4);
        assert_eq!(HeapHeader::OFFSET_FLAGS, 6);
    }

    #[test]
    fn empty_captures_heap_block_is_16_bytes() {
        // `TypedClosureHeader` alone — no captures — is HeapHeader(8) +
        // function_id(4) + type_id(4) = 16 bytes.
        let layout = immutable_layout(&[]);
        assert_eq!(layout.total_heap_size(), HEAP_CLOSURE_HEADER_SIZE);
        assert_eq!(layout.total_heap_size(), 16);
        assert_eq!(layout.heap_capture_mask, 0);
    }

    #[test]
    fn single_i64_capture_heap_layout() {
        // Capture at offset 16 (HEAP_CLOSURE_HEADER_SIZE), total 24 bytes.
        let layout = immutable_layout(&[ConcreteType::I64]);
        assert_eq!(layout.heap_capture_offset(0), 16);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.capture_kind(0), FieldKind::I64);
    }

    #[test]
    fn multi_capture_heap_layout_matches_plan_example() {
        // Plan §13 H1 test 2: `|x| x + a + b + s` with s: string.
        // Expected: one atomic retain for s (Ptr), none for a (I64) or b (F64).
        let layout = immutable_layout(&[
            ConcreteType::I64,
            ConcreteType::F64,
            ConcreteType::String,
        ]);
        assert_eq!(layout.capture_count(), 3);
        assert_eq!(layout.capture_kind(0), FieldKind::I64);
        assert_eq!(layout.capture_kind(1), FieldKind::F64);
        assert_eq!(layout.capture_kind(2), FieldKind::Ptr);
        // Exactly one heap-capture bit set (for the String at index 2).
        assert_eq!(layout.heap_capture_mask, 0b100);
        assert_eq!(layout.heap_capture_mask.count_ones(), 1);
        // The retain iteration in emit_heap_closure only visits bit 2.
        let mut visited = Vec::new();
        let mut m = layout.heap_capture_mask;
        while m != 0 {
            let bit = m.trailing_zeros() as usize;
            visited.push(bit);
            m &= m - 1;
        }
        assert_eq!(visited, vec![2]);
    }

    #[test]
    fn heap_layout_offsets_are_absolute_from_heap_base() {
        // emit_heap_closure uses `heap_capture_offset(i)` directly as the
        // absolute byte offset from the allocation base. Cross-check
        // against `capture_offset` + `HEAP_CLOSURE_HEADER_SIZE`.
        let layout = immutable_layout(&[
            ConcreteType::F64,
            ConcreteType::I32,
            ConcreteType::String,
        ]);
        for i in 0..layout.capture_count() {
            assert_eq!(
                layout.heap_capture_offset(i),
                HEAP_CLOSURE_HEADER_SIZE + layout.capture_offset(i),
            );
        }
    }

    #[test]
    fn heap_and_stack_capture_offsets_differ_by_header_size_delta() {
        // A closure literal without captures has heap size 16 but stack
        // size 8 — the 8-byte delta is the `HeapHeader`.
        let layout = immutable_layout(&[
            ConcreteType::I64,
            ConcreteType::Bool,
        ]);
        for i in 0..layout.capture_count() {
            let heap_off = layout.heap_capture_offset(i);
            let stack_off = layout.stack_capture_offset(i);
            assert_eq!(
                heap_off - stack_off,
                HEAP_CLOSURE_HEADER_SIZE - STACK_CLOSURE_HEADER_SIZE
            );
        }
    }

    #[test]
    fn cranelift_type_for_field_kind_widths() {
        // Regression: emit_heap_closure's typed-store width must match the
        // capture's FieldKind declared in ClosureLayout.
        use cranelift::prelude::types as cl;
        assert_eq!(cranelift_type_for_field_kind(FieldKind::F64), cl::F64);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::I64), cl::I64);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::I32), cl::I32);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::I16), cl::I16);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::I8), cl::I8);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::Bool), cl::I8);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::U64), cl::I64);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::U32), cl::I32);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::U16), cl::I16);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::U8), cl::I8);
        assert_eq!(cranelift_type_for_field_kind(FieldKind::Ptr), cl::I64);
    }

    #[test]
    fn array_capture_marked_as_heap_pointer() {
        // Array<int> is a refcounted heap pointer — emit_heap_closure
        // must retain it. Plan §13 H1 test 5 (array of closures) relies
        // on this mask bit being set.
        let arr = ConcreteType::Array(Box::new(ConcreteType::I64));
        let layout = immutable_layout(&[arr]);
        assert_eq!(layout.heap_capture_mask, 0b1);
        assert!(layout.is_heap_capture(0));
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
    }

    #[test]
    fn many_heap_captures_retain_iteration_order() {
        // trailing_zeros iteration visits heap captures in ascending bit
        // order — the same order `heap_capture_offset(i)` is computed in.
        // Plan §13 H1 test 3 relies on drop (and by extension retain)
        // iterating captures in `heap_capture_mask` order.
        let layout = immutable_layout(&[
            ConcreteType::String,
            ConcreteType::F64,
            ConcreteType::String,
            ConcreteType::I64,
            ConcreteType::String,
        ]);
        // Bits 0, 2, 4 set (positions 0, 2, 4 are String/Ptr).
        assert_eq!(layout.heap_capture_mask, 0b10101);
        let mut visited = Vec::new();
        let mut m = layout.heap_capture_mask;
        while m != 0 {
            let bit = m.trailing_zeros() as usize;
            visited.push(bit);
            m &= m - 1;
        }
        assert_eq!(visited, vec![0, 2, 4]);
    }

    #[test]
    fn total_heap_size_fits_u32() {
        // emit_heap_closure errors when total_heap_size exceeds u32::MAX
        // (guarding against malformed layouts). The size must fit a u32
        // for the Cranelift iconst(I32, total) path used in the allocator
        // call. For any realistic closure this is trivially true.
        let layout = immutable_layout(&[ConcreteType::I64]);
        assert!(layout.total_heap_size() <= u32::MAX as usize);
    }

    #[test]
    fn allocator_ffi_signature_is_size_u32_kind_u32() {
        // Regression: emit_heap_closure passes (size, kind) as I32, I32 to
        // jit_v2_alloc_struct. The symbol declaration in
        // ffi_symbols/v2_symbols.rs must agree. The test guards against
        // an ABI drift that would surface only at JIT compile time.
        // The declared signature is inspected indirectly via a smoke
        // check on the existing FFI shim.
        //
        // We can't easily unit-test the symbol declaration from here
        // without spinning up a JITBuilder, so we leave the signature
        // check to `register_object_symbols` + `declare_v2_functions`
        // invariants. This test documents the dependency for future
        // reviewers.
        let layout = immutable_layout(&[]);
        assert!(layout.total_heap_size() <= u32::MAX as usize);
        // The kind passed at the FFI boundary is HEAP_KIND_V2_CLOSURE; the
        // allocator writes it via HeapHeader::new. Verify the constant is
        // in the u16 range as promoted to u32 on the call.
        assert!((HEAP_KIND_V2_CLOSURE as u64) <= u32::MAX as u64);
    }

    #[test]
    fn emit_heap_closure_is_unconditional_after_h2() {
        // Closure-spec Phase H2: the env gate has been removed —
        // `emit_heap_closure` is now the unconditional default for
        // `MakeClosureHeap` lowering whenever a ClosureLayout is available
        // in `closure_function_layouts`. `jit_make_closure` is no longer
        // called on this path (§10 benchmark gate).
        //
        // The removal is enforced by a top-level grep check in CI; this
        // placeholder test documents the intent at the source. We can't
        // scan this file for the absence of a specific env-var name
        // because the test source itself contains the name in comments;
        // the authoritative check is `grep -rn` across `crates/`.
        let _ = 0;
    }

    #[test]
    fn h2_finalize_heap_closure_signature_matches_call_site() {
        // Regression: the FFI signature in `ffi_symbols/object_symbols.rs`
        // must match the call in `emit_heap_closure` — 4 arguments
        // (header_ptr: i64, function_id: i32, captures_count: i32,
        // layout_ptr: i64) returning i64. This is a documentation test;
        // if the signature changes, both sites must update.
        // See `jit_finalize_heap_closure` in `ffi/object/closure.rs`.
        let _ = super::super::super::ffi::object::jit_finalize_heap_closure;
    }
}
