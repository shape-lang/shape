//! Assignment and let expression compilation

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use shape_ast::ast::{Expr, Spanned};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::FieldType;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a let expression
    pub(super) fn compile_expr_let(&mut self, let_expr: &shape_ast::ast::LetExpr) -> Result<()> {
        self.push_scope();

        let mut future_names = std::collections::HashSet::new();
        self.collect_reference_use_names_from_expr(
            &let_expr.body,
            self.current_expr_result_mode() == crate::compiler::ExprResultMode::PreserveRef,
            &mut future_names,
        );
        self.push_future_reference_use_names(future_names);

        let compile_result = (|| -> Result<()> {
            let mut ref_borrow = None;
            if let Some(value) = &let_expr.value {
                let saved_pending_variable_name = self.pending_variable_name.clone();
                let saved_pending_variable_span = self.pending_variable_span;
                self.pending_variable_name = let_expr
                    .pattern
                    .as_simple_name()
                    .map(|name| name.to_string());
                self.pending_variable_span = let_expr.pattern.binder_span();
                let compile_result = self.compile_expr_for_reference_binding(value);
                self.pending_variable_name = saved_pending_variable_name;
                self.pending_variable_span = saved_pending_variable_span;
                ref_borrow = compile_result?;
            } else {
                self.emit(Instruction::simple(OpCode::PushNull));
            }

            self.compile_pattern_binding(&let_expr.pattern)?;
            self.mark_value_pattern_bindings_immutable(&let_expr.pattern);
            self.apply_binding_semantics_to_value_pattern_bindings(
                &let_expr.pattern,
                Self::owned_immutable_binding_semantics(),
            );
            if let Some(name) = let_expr.pattern.as_simple_name()
                && let Some(local_idx) = self.resolve_local(name)
            {
                if let Some(value) = &let_expr.value {
                    self.finish_reference_binding_from_expr(
                        local_idx, true, name, value, ref_borrow,
                    );
                    self.update_callable_binding_from_expr(local_idx, true, value);
                } else {
                    self.clear_reference_binding(local_idx, true);
                    self.clear_callable_binding(local_idx, true);
                }
            }
            if self.current_expr_result_mode() == crate::compiler::ExprResultMode::PreserveRef {
                self.compile_expr_preserving_refs(&let_expr.body)?;
            } else {
                self.compile_expr(&let_expr.body)?;
            }

            Ok(())
        })();

        self.pop_future_reference_use_names();
        self.pop_scope();
        compile_result
    }

    /// Compile an assignment expression
    pub(super) fn compile_expr_assign(
        &mut self,
        assign_expr: &shape_ast::ast::AssignExpr,
    ) -> Result<()> {
        // Session 1 — Rust-move: `let mut` bindings captured by a
        // closure are consumed at the capture site. Reject outer-scope
        // writes to a moved binding with the same diagnostic used on
        // the read path (use-after-move).
        if let Expr::Identifier(name, target_span) = assign_expr.target.as_ref() {
            if self.captured_let_mut_moved.contains_key(name) {
                let move_span = self.captured_let_mut_moved[name];
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0005] `let mut` binding '{name}' was moved into a closure and cannot \
                         be assigned to in the outer scope afterwards (Rust-move semantics). \
                         Use `var {name}` if the binding needs to be mutated in the outer scope \
                         after capture."
                    ),
                    location: {
                        let _ = move_span;
                        Some(self.span_to_source_location(*target_span))
                    },
                });
            }
        }
        // Check for const reassignment (covers compound assignments like +=)
        if let Expr::Identifier(name, _) = assign_expr.target.as_ref() {
            if let Some(local_idx) = self.resolve_local(name) {
                if !self.current_binding_uses_mir_write_authority(true)
                    && self.const_locals.contains(&local_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!("Cannot reassign const variable '{}'", name),
                        location: None,
                    });
                }
            } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    if !self.current_binding_uses_mir_write_authority(false)
                        && self.const_module_bindings.contains(&binding_idx)
                    {
                        return Err(ShapeError::SemanticError {
                            message: format!("Cannot reassign const variable '{}'", name),
                            location: None,
                        });
                    }
                }
            }
        }

        match assign_expr.target.as_ref() {
            Expr::Identifier(name, id_span) => {
                // Optimization: x = x.push(val) → ArrayPushLocal (O(1) in-place mutation)
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } = assign_expr.value.as_ref()
                {
                    if method == "push" && args.len() == 1 {
                        if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                            if recv_name == name {
                                let source_loc = self.span_to_source_location(*id_span);
                                // R1 empty-array-push let-gen (2026-06-14):
                                // `a = a.push(x)` (assignment EXPRESSION form —
                                // this is the path a loop-body `a = a.push(x*x)`
                                // takes) where `a` is a bare empty-array
                                // accumulator (`let mut a = []`, placeholder
                                // `NewArray(0)`). The v1 `ArrayPushLocal` path
                                // below assumes a materialized array carrier in
                                // the slot; the unpromoted accumulator slot is
                                // not yet a typed array — at MODULE scope it read
                                // None and SIGSEGV'd. Route the first such self-
                                // push through the accumulator finalizer: it
                                // resolves the element kind from `x`'s producer-
                                // side proof, PATCHES the placeholder allocator to
                                // the typed `NewTypedArray*` opcode AFTER the
                                // element type resolves, emits the typed push, and
                                // leaves the typed array on the stack. Store it
                                // back into the slot, then re-load so the
                                // assignment-expression result (the updated array)
                                // is on the stack — matching the v1-path contract.
                                if self.compile_first_push_to_empty_accumulator(
                                    recv_name,
                                    &args[0],
                                    Some(source_loc.clone()),
                                )? {
                                    if let Some(local_idx) = self.resolve_local(name) {
                                        self.emit(Instruction::new(
                                            OpCode::StoreLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                        self.emit(Instruction::new(
                                            OpCode::LoadLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                    } else {
                                        let binding_idx = self.get_or_create_module_binding(name);
                                        self.emit(Instruction::new(
                                            OpCode::StoreModuleBinding,
                                            Some(Operand::ModuleBinding(binding_idx)),
                                        ));
                                        self.emit(Instruction::new(
                                            OpCode::LoadModuleBinding,
                                            Some(Operand::ModuleBinding(binding_idx)),
                                        ));
                                    }
                                    return Ok(());
                                }
                                if let Some(local_idx) = self.resolve_local(name) {
                                    if !self.ref_locals.contains(&local_idx) {
                                        self.check_named_binding_write_allowed(
                                            name,
                                            Some(source_loc),
                                        )?;
                                        self.compile_expr(&args[0])?;
                                        // U4-4: pushed element kind from the one resolved Type.
                                        let pushed_numeric = self.numeric_type_of(&args[0]);
                                        self.emit(Instruction::new(
                                            OpCode::ArrayPushLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                        if let Some(numeric_type) = pushed_numeric {
                                            self.mark_slot_as_numeric_array(
                                                local_idx,
                                                true,
                                                numeric_type,
                                            );
                                        }
                                        self.plan_flexible_binding_storage_from_expr(
                                            local_idx,
                                            true,
                                            assign_expr.value.as_ref(),
                                        );
                                        // Push expression result (the updated array)
                                        self.emit(Instruction::new(
                                            OpCode::LoadLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                        return Ok(());
                                    }
                                } else {
                                    self.check_named_binding_write_allowed(name, Some(source_loc))?;
                                    // ModuleBinding variable: same optimization with ModuleBinding operand
                                    let binding_idx = self.get_or_create_module_binding(name);
                                    self.compile_expr(&args[0])?;
                                    // U4-4: pushed element kind from the one resolved Type.
                                    let pushed_numeric = self.numeric_type_of(&args[0]);
                                    self.emit(Instruction::new(
                                        OpCode::ArrayPushLocal,
                                        Some(Operand::ModuleBinding(binding_idx)),
                                    ));
                                    if let Some(numeric_type) = pushed_numeric {
                                        self.mark_slot_as_numeric_array(
                                            binding_idx,
                                            false,
                                            numeric_type,
                                        );
                                    }
                                    self.plan_flexible_binding_storage_from_expr(
                                        binding_idx,
                                        false,
                                        assign_expr.value.as_ref(),
                                    );
                                    // Push expression result (the updated array)
                                    self.emit(Instruction::new(
                                        OpCode::LoadModuleBinding,
                                        Some(Operand::ModuleBinding(binding_idx)),
                                    ));
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                let saved_pending_variable_name = self.pending_variable_name.clone();
                let saved_pending_variable_span = self.pending_variable_span;
                self.pending_variable_name = Some(name.clone());
                self.pending_variable_span = Some(*id_span);
                let compile_result = self.compile_expr_for_reference_binding(&assign_expr.value);
                self.pending_variable_name = saved_pending_variable_name;
                self.pending_variable_span = saved_pending_variable_span;
                let ref_borrow = compile_result?;
                // Phase V1.2C/D — Site B: a `var`-like assignment whose
                // target is `SharedCow` (Arc-shared) receives a freshly-
                // owned (Box-backed) rhs. Insert `PromoteToShared` so
                // the `StoreLocal` below lands an Arc in the slot
                // rather than a mixed Box/Arc representation for the
                // same binding.
                //
                // The handler is a no-op for inline scalars and
                // already-Arc values, so emitting it is correctness-
                // safe. We still gate conservatively — per V1.2C
                // guidance ("owned AND UniqueHeap in storage plan AND
                // target is SharedCow") — to keep bytecode lean:
                //   (a) immediately-preceding `PromoteToOwned`: a Box
                //       was produced on TOS by this very expression
                //       (rare but possible for heap-allocating rhs);
                //   (b) rhs identifier refers to a `UniqueHeap`
                //       source: `LoadLocal` / `CloneLocal` will have
                //       pushed a Box-backed value.
                // Anything else (function call, method call, literal,
                // computed expression) falls through — today those
                // rarely produce Box-backed TOS values at assignment
                // sites; V1.3's param-ownership hints will refine the
                // heuristic.
                if crate::compiler::helpers::promote_to_shared_enabled() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        let target_is_shared_cow = matches!(
                            self.mir_storage_class_for_slot(local_idx),
                            Some(crate::type_tracking::BindingStorageClass::SharedCow)
                        );
                        if target_is_shared_cow {
                            // (a) Collapse an adjacent PromoteToOwned into
                            //     PromoteToShared — the rhs was just
                            //     boxed, and the next op-chain would
                            //     unbox it anyway.
                            let collapsed = self
                                .program
                                .instructions
                                .last()
                                .map(|ins| ins.opcode == OpCode::PromoteToOwned)
                                .unwrap_or(false);
                            if collapsed {
                                self.program.instructions.pop();
                                self.emit(Instruction::simple(OpCode::PromoteToShared));
                            } else {
                                // (b) rhs is a bare identifier whose
                                //     source slot is Box-backed
                                //     (UniqueHeap canonically, or
                                //     Direct + non-scalar storage
                                //     hint per the Phase 4 Box
                                //     promotion). `LoadLocal` /
                                //     `CloneLocal` will have pushed
                                //     a Box on TOS.
                                if let Expr::Identifier(src_name, _) = assign_expr.value.as_ref() {
                                    if let Some(src_idx) = self.resolve_local(src_name) {
                                        if self.slot_is_heap_backed_owned(src_idx) {
                                            self.emit(Instruction::simple(OpCode::PromoteToShared));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.emit(Instruction::simple(OpCode::Dup));
                // Mutable closure captures: dispatch by CaptureKind.
                //   * `CaptureAccess::SharedCell`       → A.1B StoreSharedCapture.
                //   * `CaptureAccess::OwnedMutableCell` → A.1B StoreOwnedMutableCapture.
                //   * legacy SharedCell fallback  → StoreClosure.
                if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name.as_str()) {
                    // Track A.1C.2: Shared (var) captures route through the
                    // A.1B StoreSharedCapture opcode, which takes the
                    // parking_lot mutex on the `Arc<SharedCell>` pointer
                    // stored in the capture slot and overwrites the inner
                    // raw bits.
                    if let Some(&shared_idx) = self.shared_closure_captures.get(name.as_str()) {
                        debug_assert_eq!(upvalue_idx, shared_idx);
                        // A2-refined / task #17: dispatch to Wave D.2's typed
                        // `StoreSharedCapture<Kind>` opcodes (codes
                        // 0x161-0x16B) by looking up the cell's interior
                        // `FieldKind` from `shared_capture_inner_kinds`.
                        // Falls back to legacy `StoreSharedCapture` (0x135)
                        // for unresolved capture types.
                        let opcode =
                            match self.shared_capture_inner_kinds.get(name.as_str()).copied() {
                                Some(kind) => {
                                    crate::compiler::helpers::shared_typed_store_opcode(kind)
                                }
                                None => OpCode::StoreSharedCapture,
                            };
                        self.emit(Instruction::new(opcode, Some(Operand::Local(shared_idx))));
                        return Ok(());
                    }
                    // Track A.1C.2b + Wave E: OwnedMutable (let mut)
                    // captures route through Wave D.1's per-`FieldKind`
                    // typed `StoreOwnedMutableCapture<Kind>` opcodes
                    // (codes 0x14B-0x155). The interior `FieldKind` is
                    // looked up in `owned_mutable_capture_inner_kinds`
                    // (populated at closure-construction time from the
                    // captured binding's resolved `ConcreteType`); each
                    // typed opcode pops raw native bytes from the stack
                    // and writes through the matching native cell. Falls
                    // back to legacy `StoreOwnedMutableCapture` (0x133)
                    // for unresolved capture types — Wave G removes the
                    // legacy opcode after every emit path is type-aware.
                    // The Shared (`var`) write path above stays on the
                    // legacy `StoreSharedCapture` (0x135) — atomic flip
                    // is follow-up #17.
                    if let Some(&owned_idx) = self.owned_mutable_closure_captures.get(name.as_str())
                    {
                        debug_assert_eq!(upvalue_idx, owned_idx);
                        let opcode = match self
                            .owned_mutable_capture_inner_kinds
                            .get(name.as_str())
                            .copied()
                        {
                            Some(kind) => {
                                crate::compiler::helpers::owned_mutable_typed_store_opcode(kind)
                            }
                            None => OpCode::StoreOwnedMutableCapture,
                        };
                        self.emit(Instruction::new(opcode, Some(Operand::Local(owned_idx))));
                        return Ok(());
                    }
                    self.emit(Instruction::new(
                        OpCode::StoreClosure,
                        Some(Operand::Local(upvalue_idx)),
                    ));
                    return Ok(());
                }
                if let Some(local_idx) = self.resolve_local(name) {
                    if self.local_binding_is_reference_value(local_idx) {
                        if !self.local_reference_binding_is_exclusive(local_idx) {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "cannot assign through shared reference variable '{}'",
                                    name
                                ),
                                location: Some(self.span_to_source_location(*id_span)),
                            });
                        }
                        // Reference parameter or reference-valued binding: write through the reference
                        self.emit(Instruction::new(
                            OpCode::DerefStore,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else if self.shared_locals.contains(name) {
                        // Track A.1C.2: the slot has been promoted to
                        // `Arc<SharedCell>` via `AllocSharedLocal`. The
                        // new value on top of the stack must be stored
                        // into the cell through the parking_lot mutex,
                        // not into the slot itself (which holds the
                        // `*const SharedCell` pointer bits).
                        let source_loc = self.span_to_source_location(*id_span);
                        self.check_named_binding_write_allowed(name, Some(source_loc))?;
                        self.emit(Instruction::new(
                            OpCode::StoreSharedLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else {
                        // Borrow check: reject writes to borrowed variables
                        let source_loc = self.span_to_source_location(*id_span);
                        self.check_named_binding_write_allowed(name, Some(source_loc))?;

                        // E+5.5 Unit C step 1: typed local assignment for
                        // width-typed locals (i32 / u16 / etc.) — patch
                        // through the existing `StoreLocalTyped` lane that
                        // does width truncation. For other proven Int /
                        // Bool / F64 / Ptr slots, emit the typed
                        // `StoreLocal<Kind>` (E+3 codes 0x177-0x181) so
                        // the slot's bit-pattern stays in lockstep with
                        // the post-Unit-A native producer contract.
                        // Unproven hints fall back to polymorphic
                        // `StoreLocal`.
                        let width_typed = self
                            .type_tracker
                            .get_local_type(local_idx)
                            .and_then(|info| info.type_name.as_deref())
                            .and_then(shape_ast::IntWidth::from_name);
                        if let Some(w) = width_typed {
                            self.emit(Instruction::new(
                                OpCode::StoreLocalTyped,
                                Some(Operand::TypedLocal(
                                    local_idx,
                                    crate::bytecode::NumericWidth::from_int_width(w),
                                )),
                            ));
                        } else {
                            // Per ADR-006 §2.7.5.1, "kind not yet known"
                            // is `Option<StorageHint>` locally — there is
                            // no `StorageHint::Unknown` sentinel. On
                            // `None`, fall back to the polymorphic
                            // `StoreLocal` (mirrors helpers_binding.rs
                            // emit_load_local_owned migration).
                            // `info.storage_hint` is itself
                            // `Option<StorageHint>`, so `.and_then`
                            // collapses both Option layers into one.
                            let hint = self
                                .type_tracker
                                .get_local_type(local_idx)
                                .and_then(|info| info.storage_hint);
                            match hint {
                                Some(h) => self.emit_store_local_for_hint(local_idx, h),
                                None => {
                                    self.emit(Instruction::new(
                                        OpCode::StoreLocal,
                                        Some(Operand::Local(local_idx)),
                                    ));
                                }
                            }
                        }
                    }
                    if !self.local_binding_is_reference_value(local_idx) {
                        self.finish_reference_binding_from_expr(
                            local_idx,
                            true,
                            name,
                            &assign_expr.value,
                            ref_borrow,
                        );
                        self.update_callable_binding_from_expr(local_idx, true, &assign_expr.value);
                    }
                    self.plan_flexible_binding_storage_from_expr(
                        local_idx,
                        true,
                        &assign_expr.value,
                    );
                } else {
                    let source_loc = self.span_to_source_location(*id_span);
                    self.check_named_binding_write_allowed(name, Some(source_loc))?;
                    let binding_idx = self.get_or_create_module_binding(name);
                    // Track A.1C.3: if the module-binding slot has been
                    // promoted to `Arc<SharedCell>` by a prior closure
                    // capture, writes must go through
                    // `StoreSharedModuleBinding` (which takes the mutex
                    // and writes the inner raw bits). Plain
                    // `StoreModuleBinding` would overwrite the raw Arc
                    // pointer bits, leaking the Arc and losing the
                    // shared state.
                    let shared_module_name = self
                        .resolve_scoped_module_binding_name(name)
                        .unwrap_or_else(|| name.to_string());
                    if self.shared_module_bindings.contains(&shared_module_name) {
                        self.emit(Instruction::new(
                            OpCode::StoreSharedModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                    } else {
                        self.emit(Instruction::new(
                            OpCode::StoreModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        // Patch StoreModuleBinding → StoreModuleBindingTyped for width-typed bindings
                        if let Some(type_name) = self
                            .type_tracker
                            .get_binding_type(binding_idx)
                            .and_then(|info| info.type_name.as_deref())
                        {
                            if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                                if let Some(last) = self.program.instructions.last_mut() {
                                    if last.opcode == OpCode::StoreModuleBinding {
                                        last.opcode = OpCode::StoreModuleBindingTyped;
                                        last.operand = Some(Operand::TypedModuleBinding(
                                            binding_idx,
                                            crate::bytecode::NumericWidth::from_int_width(w),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    self.finish_reference_binding_from_expr(
                        binding_idx,
                        false,
                        name,
                        &assign_expr.value,
                        ref_borrow,
                    );
                    self.update_callable_binding_from_expr(binding_idx, false, &assign_expr.value);
                    self.plan_flexible_binding_storage_from_expr(
                        binding_idx,
                        false,
                        &assign_expr.value,
                    );
                }
                self.propagate_assignment_type_to_identifier(name, Some(&assign_expr.value));
                Ok(())
            }
            Expr::PropertyAccess {
                object, property, ..
            } => {
                const OBJECT_REF_STORAGE_ERROR: &str = "cannot store a reference in an object or struct literal — references are scoped borrows that cannot escape into aggregate values. Use owned values instead";
                // The MakeRef + MakeFieldRef + DerefStore path is only valid
                // for fields whose declared type maps to one fixed carrier
                // kind. Dynamic Any and Option<T> fields route through
                // SetFieldTyped so the runtime can source/update
                // TypedObjectStorage::field_kinds in lockstep with the slot.
                let typed_field_place = self
                    .try_resolve_typed_field_place(object, property)
                    .filter(Self::typed_field_place_has_fixed_field_ref_kind);
                if let Some(place) = typed_field_place {
                    let label = format!("{}.{}", place.root_name, property);
                    let source_loc = self.span_to_source_location(assign_expr.target.span());
                    self.check_write_allowed_in_current_context(place.borrow_key, Some(source_loc))
                        .map_err(|err| Self::relabel_borrow_error(err, place.borrow_key, &label))?;

                    // v0.3.3 c2-A (audit `docs/cluster-audits/v0.3.3/02-adr-006-2-7-13-kind-drift.md`
                    // Sub-bug A — int→number assignment-side widening gap): reject at
                    // compile time when the RHS literal's inferred `FieldType` does not
                    // exactly match the resolved field's declared type. The DerefStore
                    // chain emitted below stamps the captured place-kind from
                    // `place.typed_operand.field_type_tag`; the RHS producer (a literal
                    // here) lands its own `NativeKind` on the stack with no widening
                    // (assignment-side has no `kinded_to_slot` equivalent — that is the
                    // construction-only path at `executor/objects/object_creation.rs:448-487`).
                    // The runtime invariant at ADR-006 §2.7.13 (`executor/variables/mod.rs:2718`)
                    // then SURFACEs the drift as a debug_assert! panic; in release the
                    // assert is stripped and the writer silently lays the RHS bits into
                    // a kind-mismatched slot — subsequent reads via `field_kinds[i]`
                    // reinterpret the bits (verified: `5e-323` denormal for an Int64
                    // pattern of `10` in an F64 slot at HEAD `53549fcb`).
                    //
                    // Fix shape: compile-time reject (strict-typing playbook). Per audit
                    // §2 Sub-bug A "Fix #1" + CLAUDE.md §Type System Rules "NO runtime
                    // coercion" — adding a runtime int->number coercion opcode here is
                    // the W4-δ Convert-opcode defection-attractor named in CLAUDE.md
                    // §Forbidden Patterns ("paper over a kind-tracker gap"). The user
                    // writes `10.0` or `10 as number` explicitly.
                    //
                    // Scope: only literal RHS (matches `infer_field_type_from_expr`,
                    // identical helper used construction-side at `collections.rs:1026`).
                    // Non-literal RHS that would drift falls through and may still hit
                    // the §2.7.13 SURFACE — that is a kind-tracker followup, not c2-A
                    // scope. `FieldType::Any` is already filtered out at the
                    // `typed_field_place` `.filter(...)` above (W15.2-LANG-8 path).
                    if let Some(inferred) =
                        super::collections::infer_field_type_from_expr(&assign_expr.value)
                    {
                        // Numeric-conversion §4 literal adoption (field-assignment
                        // context, THE RULE user 2026-06-01): a bare int-literal RHS
                        // into a numeric field (`p.x = 10` where `x: number`) adopts
                        // the field type when the literal value is losslessly
                        // representable — the construction-side twin
                        // (`collections.rs::int_literal_adopts_field_type`, used at
                        // the struct-literal producer) already accepts `P { x: 1 }`,
                        // so the mutation form must agree. An out-of-range literal
                        // (`p.x = 300` into u8) does NOT adopt and still rejects;
                        // a non-literal int VAR keeps the §2 value-level reject in
                        // the `else` arm below.
                        let literal_adopts = super::collections::int_literal_adopts_field_type(
                            &assign_expr.value,
                            &place.field_type_info,
                        );
                        if inferred != place.field_type_info && !literal_adopts {
                            let value_loc = self.span_to_source_location(assign_expr.value.span());
                            let mut loc = value_loc;
                            loc.hints.push(format!(
                                "expected `{}`, found `{}`",
                                place.field_type_info, inferred
                            ));
                            loc.hints.push(format!(
                                "use an explicit conversion: `... as {}`",
                                place.field_type_info
                            ));
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "type mismatch: cannot assign `{}` to field `{}.{}` of type `{}`",
                                    inferred, place.root_name, property, place.field_type_info
                                ),
                                location: Some(loc),
                            });
                        }
                    } else if let Ok(rhs_ty) = self.infer_expr_type(&assign_expr.value) {
                        // v0.3.3 c2a-cluster sub-fix (ii) — non-literal RHS at the
                        // assignment producer site (audit `docs/cluster-audits/v0.3.3/
                        // 02-adr-006-2-7-13-kind-drift.md` Sub-bug A "Recommended"
                        // disposition + supervisor 2026-05-28 dispatch). The c2-A literal
                        // arm above handles `p.x = 10` where the RHS is a literal we can
                        // classify via `infer_field_type_from_expr`. For non-literal RHS
                        // like `let v = 10; p.x = v`, the literal helper returns `None`
                        // and the pre-fix code path fell through to silent corruption —
                        // verified at HEAD `67768f17`:
                        //
                        //   type Point { x: number, y: number }
                        //   let mut p = Point { x: 1.0, y: 2.0 }
                        //   let v = 10
                        //   p.x = v
                        //   p.x   // -> {"Number": 5e-323}    ← Int64-bits-as-F64 denormal
                        //
                        // Fix shape: read-only AST-level inference via `infer_expr_type`
                        // (defined at `crates/shape-vm/src/compiler/expressions/mod.rs:1364`).
                        // This is NOT a producer-walk-back over already-emitted opcodes —
                        // that path would require the RHS expression to ALREADY be emitted,
                        // which would commit a `MakeRef + MakeFieldRef + ... + DerefStore`
                        // chain to the program before discovering the mismatch (we'd then
                        // have to dead-pop the stream, fragile). The AST-level type
                        // inference is read-only and emission-free.
                        //
                        // For non-literal sites we resolve the RHS expression's type to a
                        // `FieldType` via the same mapping `type_annotation_to_field_type`
                        // uses for type-def field declarations
                        // (`helpers.rs:4933`). If both sides resolve to a primitive
                        // `FieldType` (I64 / F64 / Bool / Decimal / String / Timestamp /
                        // width-int) AND they differ → compile-reject with the same
                        // diagnostic shape as the literal arm. When either side fails to
                        // resolve (generic param, untracked binding, complex chain) we
                        // skip the check — leaving the §2.7.13 SURFACE for the
                        // kind-tracker walk-back to catch downstream. This is the same
                        // conservatism `infer_expr_type` already applies to identifier
                        // resolution at lines 1389-1397.
                        //
                        // Forbidden patterns explicitly avoided: no runtime
                        // `ConvertIntToNumber` opcode (W4-δ defection-attractor per
                        // CLAUDE.md §Forbidden Patterns); no producer-side fabrication
                        // of NativeKind at emit time; no defection-attractor descriptor
                        // (per §Renames-to-refuse-on-sight family).
                        let rhs_field_type = Self::primitive_type_to_field_type(&rhs_ty);
                        if let Some(rhs_ft) = rhs_field_type {
                            // Only fire when BOTH sides resolve to a primitive — the
                            // generic / object / unresolved cases are conservatively
                            // skipped (see comment above).
                            let lhs_is_primitive = matches!(
                                place.field_type_info,
                                FieldType::I64
                                    | FieldType::F64
                                    | FieldType::Bool
                                    | FieldType::String
                                    | FieldType::Decimal
                                    | FieldType::Timestamp
                                    | FieldType::I8
                                    | FieldType::U8
                                    | FieldType::I16
                                    | FieldType::U16
                                    | FieldType::I32
                                    | FieldType::U32
                                    | FieldType::U64
                            );
                            if lhs_is_primitive && rhs_ft != place.field_type_info {
                                let value_loc =
                                    self.span_to_source_location(assign_expr.value.span());
                                let mut loc = value_loc;
                                loc.hints.push(format!(
                                    "expected `{}`, found `{}`",
                                    place.field_type_info, rhs_ft
                                ));
                                loc.hints.push(format!(
                                    "use an explicit conversion: `... as {}`",
                                    place.field_type_info
                                ));
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "type mismatch: cannot assign `{}` to field `{}.{}` of type `{}`",
                                        rhs_ft, place.root_name, property, place.field_type_info
                                    ),
                                    location: Some(loc),
                                });
                            }
                        }
                    }

                    let field_ref = self.declare_temp_local("__field_assign_ref_")?;
                    let root_operand = if place.is_local {
                        Operand::Local(place.slot)
                    } else {
                        Operand::ModuleBinding(place.slot)
                    };
                    self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));
                    // JOINT-FIX #2 (v0.3.3 c2-sub-bug-B nested-projection):
                    // emit a `MakeFieldRef` per chain entry so a nested write
                    // like `o.data.val = 42` produces
                    //   MakeRef(o); MakeFieldRef(data); MakeFieldRef(val)
                    // — the previous shape emitted only ONE `MakeFieldRef`
                    // with the leaf's `(type_id=Inner, field_idx=val_idx)`
                    // operand. At runtime that mis-projected the leaf's
                    // field_idx against the root receiver's schema (Outer),
                    // landing kind=Int64 against `Outer.field_kinds[0]=
                    // Ptr(TypedObject)` and tripping the §2.7.13 / Q14
                    // `DerefStore` debug_assert (release: silent v2-raw
                    // double-free SIGABRT). For a non-nested write the
                    // chain has a single entry and the emission shape is
                    // identical to the pre-fix code path.
                    let field_chain = self.collect_property_access_chain(object, property);
                    debug_assert!(
                        !field_chain.is_empty(),
                        "JOINT-FIX #2: collect_property_access_chain produced \
                         an empty chain for a resolved typed_field_place",
                    );
                    for field_operand in field_chain {
                        self.emit(Instruction::new(OpCode::MakeFieldRef, Some(field_operand)));
                    }
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(field_ref)),
                    ));

                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        OBJECT_REF_STORAGE_ERROR,
                    )?;
                    // Numeric-conversion §4 literal adoption (field-assignment
                    // widening, THE RULE user 2026-06-01): when a bare int-literal
                    // RHS adopts a `number`(F64) field, it IS the number literal —
                    // compile it as a `Number` constant so the value laid into the
                    // F64 slot is f64-kinded, not Int64 (the assignment path has no
                    // construction-side `kinded_to_slot` runtime widen, so without
                    // this the DerefStore lays Int64 bits into the F64 place and the
                    // §2.7.13 kind-drift invariant fires). This is compile-time
                    // literal re-typing, NOT a runtime coercion opcode (the literal
                    // `10` is exactly `10.0` in a number context) — no W4-δ Convert
                    // defection. Width-int / decimal fields keep their natural
                    // literal lowering (Int64 bits are the correct slot payload).
                    let widened_literal = if place.field_type_info == FieldType::F64 {
                        if let Expr::Literal(lit, lit_span) = assign_expr.value.as_ref() {
                            match lit {
                                shape_ast::ast::Literal::Int(v) => Some(Expr::Literal(
                                    shape_ast::ast::Literal::Number(*v as f64),
                                    *lit_span,
                                )),
                                shape_ast::ast::Literal::UInt(v) => Some(Expr::Literal(
                                    shape_ast::ast::Literal::Number(*v as f64),
                                    *lit_span,
                                )),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    match &widened_literal {
                        Some(num_lit) => self.compile_expr(num_lit)?,
                        None => self.compile_expr(&assign_expr.value)?,
                    }
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::simple(OpCode::Dup));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::DerefStore,
                        Some(Operand::Local(field_ref)),
                    ));
                    // WF-2B snapshot fix (defect 2): the `field_ref` temp now
                    // holds a dead `RefTarget::TypedField` reference. Left in
                    // the register window it trips the §2.7.30.7 non-promoted-
                    // reference guard on any later `snapshot()` in this frame.
                    // Release it (StoreLocal drops the prior occupant); the
                    // temp is provably dead here. See `release_field_ref_temp`.
                    self.release_field_ref_temp(field_ref);
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    return Ok(());
                }

                if let Expr::Identifier(name, id_span) = object.as_ref()
                    && let Some(local_idx) = self.resolve_local(name)
                    && !self.ref_locals.contains(&local_idx)
                {
                    let source_loc = self.span_to_source_location(*id_span);
                    self.check_write_allowed_in_current_context(
                        Self::borrow_key_for_local(local_idx),
                        Some(source_loc),
                    )
                    .map_err(|e| match e {
                        ShapeError::SemanticError { message, location } => {
                            let user_msg = message
                                .replace(&format!("(slot {})", local_idx), &format!("'{}'", name));
                            ShapeError::SemanticError {
                                message: user_msg,
                                location,
                            }
                        }
                        other => other,
                    })?;
                }
                self.compile_expr(object)?;
                let Some(schema_id) = self.last_expr_schema else {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Assignment to '{}.{}' requires compile-time field resolution. Generic runtime property lookup is disabled.",
                            match object.as_ref() {
                                Expr::Identifier(name, _) => name,
                                _ => "<expr>",
                            },
                            property
                        ),
                        location: None,
                    });
                };

                let (typed_operand, field_type_info) = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .and_then(|schema| {
                        schema.get_field(property).and_then(|field| {
                            if schema_id <= u16::MAX as u32 {
                                Some((
                                    Operand::TypedField {
                                        type_id: schema_id as u16,
                                        field_idx: field.index as u16,
                                        field_type_tag: field_type_to_tag(&field.field_type),
                                    },
                                    field.field_type.clone(),
                                ))
                            } else {
                                None
                            }
                        })
                    })
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Property '{}.{}' is not resolvable at compile time for assignment.",
                            match object.as_ref() {
                                Expr::Identifier(name, _) => name,
                                _ => "<expr>",
                            },
                            property
                        ),
                        location: None,
                    })?;

                self.reject_direct_reference_storage(&assign_expr.value, OBJECT_REF_STORAGE_ERROR)?;
                self.check_option_field_assignment_value(
                    &field_type_info,
                    property,
                    &assign_expr.value,
                )?;
                if Self::field_type_accepts_none_literal(&field_type_info, &assign_expr.value) {
                    self.compile_canonical_option_none_carrier()?;
                } else {
                    self.compile_expr(&assign_expr.value)?;
                }
                let value_local = self.declare_temp_local("__assign_value_")?;
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::new(OpCode::SetFieldTyped, Some(typed_operand)));
                // Store the modified object back through the property chain
                // (handles nested field mutation like o.data.val = 42)
                self.emit_nested_store_back(object)?;
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                Ok(())
            }
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                ..
            } => {
                const ARRAY_REF_STORAGE_ERROR: &str = "cannot store a reference in an array — references are scoped borrows that cannot escape into collections. Use owned values instead";
                // v2 Phase 3.1 (Agent 3): typed-array fast path for `arr[i] = x`.
                // Resolve BEFORE compiling the value (compile_expr may
                // overwrite tracker state).
                let typed_kind = self.resolve_receiver_typed_array_kind(object);

                // W1.11 (v0.3 R2): user-type `IndexMut` trait dispatch for
                // `c[k] = v`. Fires when the receiver's type implements
                // `IndexMut` and the built-in typed-array fast path
                // doesn't apply (typed_kind is None). The dispatch emits
                // `CallMethod("index_set", arg_count=2)` after pushing
                // (receiver, key, value) onto the stack, then preserves
                // the value as the assignment-expression result. Sibling
                // of the `Index` dispatch in `property_access.rs:compile_expr_index_access`.
                if typed_kind.is_none() && self.receiver_type_implements_trait(object, "IndexMut") {
                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        ARRAY_REF_STORAGE_ERROR,
                    )?;
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    self.compile_expr(&assign_expr.value)?;
                    // Stash the value for the assignment-expression result
                    // before `index_set` consumes it.
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    // Re-push value as the third argument so `index_set`
                    // sees (receiver, key, value) on the stack.
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    super::property_access::emit_index_trait_call(self, "index_set", 2);
                    // `index_set` returns `void` — drop the unit result so
                    // the assignment-expression result is the stashed value.
                    self.emit(Instruction::simple(OpCode::Pop));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    return Ok(());
                }

                if let Expr::Identifier(name, _) = object.as_ref() {
                    // Local-slot-based SetElem fast path: `SetElemI64`/
                    // `SetElemF64` take the array slot as operand and
                    // the (index, value) on the stack. Saves LoadLocal.
                    if let Some(kind) = typed_kind {
                        let set_elem_opcode = match kind {
                            crate::compiler::v2_typed_emission::TypedArrayKind::I64 => {
                                Some(OpCode::SetElemI64)
                            }
                            crate::compiler::v2_typed_emission::TypedArrayKind::F64 => {
                                Some(OpCode::SetElemF64)
                            }
                            _ => None,
                        };
                        if let Some(set_opcode) = set_elem_opcode {
                            if let Some(local_idx) = self.resolve_local(name) {
                                if !self.ref_locals.contains(&local_idx) {
                                    let source_loc = self.span_to_source_location(index.span());
                                    self.check_write_allowed_in_current_context(
                                        Self::borrow_key_for_local(local_idx),
                                        Some(source_loc),
                                    )
                                    .map_err(|e| match e {
                                        ShapeError::SemanticError { message, location } => {
                                            let user_msg = message.replace(
                                                &format!("(slot {})", local_idx),
                                                &format!("'{}'", name),
                                            );
                                            ShapeError::SemanticError {
                                                message: user_msg,
                                                location,
                                            }
                                        }
                                        other => other,
                                    })?;
                                    self.reject_direct_reference_storage(
                                        &assign_expr.value,
                                        ARRAY_REF_STORAGE_ERROR,
                                    )?;
                                    self.compile_expr(index)?;
                                    self.compile_expr(&assign_expr.value)?;
                                    let value_local = self.declare_temp_local("__assign_value_")?;
                                    self.emit(Instruction::simple(OpCode::Dup));
                                    self.emit(Instruction::new(
                                        OpCode::StoreLocal,
                                        Some(Operand::Local(value_local)),
                                    ));
                                    self.emit(Instruction::new(
                                        set_opcode,
                                        Some(Operand::Local(local_idx)),
                                    ));
                                    self.emit(Instruction::new(
                                        OpCode::LoadLocal,
                                        Some(Operand::Local(value_local)),
                                    ));
                                    return Ok(());
                                }
                            }
                        }
                    }
                    // Typed array fast path: emit (arr_ptr, index, value)
                    // directly so the v2 set opcode can pop them in order.
                    // Skip ref-parameter cases — they fall back to the
                    // legacy SetIndexRef path below.
                    if let Some(kind) = typed_kind {
                        if let Some(local_idx) = self.resolve_local(name) {
                            if !self.ref_locals.contains(&local_idx) {
                                let source_loc = self.span_to_source_location(index.span());
                                self.check_write_allowed_in_current_context(
                                    Self::borrow_key_for_local(local_idx),
                                    Some(source_loc),
                                )
                                .map_err(|e| match e {
                                    ShapeError::SemanticError { message, location } => {
                                        let user_msg = message.replace(
                                            &format!("(slot {})", local_idx),
                                            &format!("'{}'", name),
                                        );
                                        ShapeError::SemanticError {
                                            message: user_msg,
                                            location,
                                        }
                                    }
                                    other => other,
                                })?;
                                self.reject_direct_reference_storage(
                                    &assign_expr.value,
                                    ARRAY_REF_STORAGE_ERROR,
                                )?;
                                // Push (arr, index, value) in the order
                                // `TypedArraySet*` expects.
                                self.emit(Instruction::new(
                                    OpCode::LoadLocal,
                                    Some(Operand::Local(local_idx)),
                                ));
                                self.compile_expr(index)?;
                                self.compile_expr(&assign_expr.value)?;
                                // Stash the value for the assignment-result
                                // expression.
                                let value_local = self.declare_temp_local("__assign_value_")?;
                                self.emit(Instruction::simple(OpCode::Dup));
                                self.emit(Instruction::new(
                                    OpCode::StoreLocal,
                                    Some(Operand::Local(value_local)),
                                ));
                                self.emit(Instruction::simple(kind.set_opcode()));
                                self.emit(Instruction::new(
                                    OpCode::LoadLocal,
                                    Some(Operand::Local(value_local)),
                                ));
                                return Ok(());
                            }
                        } else {
                            // Module binding case (top-level `let arr`).
                            let binding_idx = self.get_or_create_module_binding(name);
                            let source_loc = self.span_to_source_location(index.span());
                            self.check_write_allowed_in_current_context(
                                Self::borrow_key_for_module_binding(binding_idx),
                                Some(source_loc),
                            )
                            .map_err(|e| match e {
                                ShapeError::SemanticError { message, location } => {
                                    let user_msg = message.replace(
                                        &format!(
                                            "(slot {})",
                                            Self::borrow_key_for_module_binding(binding_idx)
                                        ),
                                        &format!("'{}'", name),
                                    );
                                    ShapeError::SemanticError {
                                        message: user_msg,
                                        location,
                                    }
                                }
                                other => other,
                            })?;
                            self.reject_direct_reference_storage(
                                &assign_expr.value,
                                ARRAY_REF_STORAGE_ERROR,
                            )?;
                            // Push (arr, index, value) in the order
                            // `TypedArraySet*` expects.
                            self.emit(Instruction::new(
                                OpCode::LoadModuleBinding,
                                Some(Operand::ModuleBinding(binding_idx)),
                            ));
                            self.compile_expr(index)?;
                            self.compile_expr(&assign_expr.value)?;
                            let value_local = self.declare_temp_local("__assign_value_")?;
                            self.emit(Instruction::simple(OpCode::Dup));
                            self.emit(Instruction::new(
                                OpCode::StoreLocal,
                                Some(Operand::Local(value_local)),
                            ));
                            self.emit(Instruction::simple(kind.set_opcode()));
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(value_local)),
                            ));
                            return Ok(());
                        }
                    }
                    self.compile_expr(index)?;
                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        ARRAY_REF_STORAGE_ERROR,
                    )?;
                    self.compile_expr(&assign_expr.value)?;
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::simple(OpCode::Dup));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.ref_locals.contains(&local_idx) {
                            // Reference parameter: mutate array in-place through the reference
                            self.emit(Instruction::new(
                                OpCode::SetIndexRef,
                                Some(Operand::Local(local_idx)),
                            ));
                        } else {
                            let source_loc = self.span_to_source_location(index.span());
                            self.check_write_allowed_in_current_context(
                                Self::borrow_key_for_local(local_idx),
                                Some(source_loc),
                            )
                            .map_err(|e| match e {
                                ShapeError::SemanticError { message, location } => {
                                    let user_msg = message.replace(
                                        &format!("(slot {})", local_idx),
                                        &format!("'{}'", name),
                                    );
                                    ShapeError::SemanticError {
                                        message: user_msg,
                                        location,
                                    }
                                }
                                other => other,
                            })?;
                            self.emit(Instruction::new(
                                OpCode::SetLocalIndex,
                                Some(Operand::Local(local_idx)),
                            ));
                        }
                    } else {
                        let binding_idx = self.get_or_create_module_binding(name);
                        let source_loc = self.span_to_source_location(index.span());
                        self.check_write_allowed_in_current_context(
                            Self::borrow_key_for_module_binding(binding_idx),
                            Some(source_loc),
                        )
                        .map_err(|e| match e {
                            ShapeError::SemanticError { message, location } => {
                                let user_msg = message.replace(
                                    &format!(
                                        "(slot {})",
                                        Self::borrow_key_for_module_binding(binding_idx)
                                    ),
                                    &format!("'{}'", name),
                                );
                                ShapeError::SemanticError {
                                    message: user_msg,
                                    location,
                                }
                            }
                            other => other,
                        })?;
                        self.emit(Instruction::new(
                            OpCode::SetModuleBindingIndex,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                    }
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    Ok(())
                } else {
                    self.compile_expr(object)?;
                    self.compile_expr(index)?;
                    self.reject_direct_reference_storage(
                        &assign_expr.value,
                        ARRAY_REF_STORAGE_ERROR,
                    )?;
                    self.compile_expr(&assign_expr.value)?;
                    let value_local = self.declare_temp_local("__assign_value_")?;
                    self.emit(Instruction::simple(OpCode::Dup));
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit(Instruction::simple(OpCode::SetProp));
                    self.emit(Instruction::simple(OpCode::Pop));
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    Ok(())
                }
            }
            Expr::IndexAccess {
                object,
                index,
                end_index: Some(end_index),
                ..
            } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.compile_expr(end_index)?;
                // Push inclusive flag (exclusive by default for slice syntax)
                let const_idx = self.program.add_constant(Constant::Bool(false));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(const_idx)),
                ));
                self.emit(Instruction::simple(OpCode::MakeRange));
                self.compile_expr(&assign_expr.value)?;
                let value_local = self.declare_temp_local("__assign_value_")?;
                self.emit(Instruction::simple(OpCode::Dup));
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit(Instruction::simple(OpCode::SetProp));
                if let Expr::Identifier(name, _) = object.as_ref() {
                    self.emit_store_identifier(name)?;
                } else {
                    self.emit(Instruction::simple(OpCode::Pop));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(value_local)),
                ));
                Ok(())
            }
            _ => Err(ShapeError::RuntimeError {
                message: "Invalid assignment target".to_string(),
                location: None,
            }),
        }
    }

    /// Store the modified object back through a property access chain.
    /// After SetFieldTyped, the modified object is on the stack. If the parent
    /// expression is an Identifier, store directly. If it's a nested
    /// PropertyAccess, recursively store back through each level.
    fn emit_nested_store_back(&mut self, object: &Expr) -> Result<()> {
        match object {
            Expr::Identifier(name, _) => {
                self.emit_store_identifier(name)?;
                Ok(())
            }
            Expr::PropertyAccess {
                object: parent,
                property,
                ..
            } => {
                // The modified child object is on the stack.
                // Store it to a temp so we can reload the parent.
                let child_temp = self.declare_temp_local("__nested_assign_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(child_temp)),
                ));

                // Load the parent object
                self.compile_expr(parent)?;
                let schema_id = self
                    .last_expr_schema
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Nested assignment requires compile-time schema for parent of '{}'.",
                            property
                        ),
                        location: None,
                    })?;

                // Load the modified child
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(child_temp)),
                ));

                // Set the field on the parent
                let typed_operand = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .and_then(|schema| {
                        schema.get_field(property).and_then(|field| {
                            if schema_id <= u16::MAX as u32 {
                                Some(Operand::TypedField {
                                    type_id: schema_id as u16,
                                    field_idx: field.index as u16,
                                    field_type_tag: field_type_to_tag(&field.field_type),
                                })
                            } else {
                                None
                            }
                        })
                    })
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Property '{}' is not resolvable for nested store-back.",
                            property
                        ),
                        location: None,
                    })?;
                self.emit(Instruction::new(OpCode::SetFieldTyped, Some(typed_operand)));

                // Recurse up the chain
                self.emit_nested_store_back(parent)
            }
            _ => {
                self.emit(Instruction::simple(OpCode::Pop));
                Ok(())
            }
        }
    }

    /// v0.3.3 c2a-cluster sub-fix (ii) helper — map a `Type::Concrete(Basic(...))`
    /// result from `infer_expr_type` to a primitive `FieldType` so the
    /// assignment-side compile-reject can compare against the resolved field's
    /// declared FieldType.
    ///
    /// Mirrors the primitive-name table of `type_annotation_to_field_type` at
    /// `crates/shape-vm/src/compiler/helpers.rs:4933`. We intentionally only
    /// classify the Basic-name primitives here; complex shapes (Array<T>,
    /// HashMap<K,V>, Option<T>, Object("...")) return `None` and the caller
    /// conservatively skips the check.
    ///
    /// Why a focused helper instead of reusing `type_annotation_to_field_type`:
    /// `infer_expr_type` returns `shape_runtime::type_system::Type`, not
    /// `shape_ast::ast::TypeAnnotation`. The mappings overlap on primitives
    /// but diverge on generic shapes and tuple syntax. Sub-fix (ii) only
    /// needs the primitive cohort — keeping the helper scoped here documents
    /// the scope and avoids drift across the boundary.
    fn primitive_type_to_field_type(ty: &shape_runtime::type_system::Type) -> Option<FieldType> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;
        let name = match ty {
            Type::Concrete(TypeAnnotation::Basic(s)) => s.as_str(),
            _ => return None,
        };
        Some(match name {
            "number" | "float" | "f64" | "f32" => FieldType::F64,
            "i8" => FieldType::I8,
            "u8" => FieldType::U8,
            "i16" => FieldType::I16,
            "u16" => FieldType::U16,
            "i32" => FieldType::I32,
            "u32" => FieldType::U32,
            "u64" => FieldType::U64,
            "int" | "i64" | "integer" | "isize" | "usize" | "byte" | "char" => FieldType::I64,
            "string" | "str" => FieldType::String,
            "decimal" => FieldType::Decimal,
            "bool" | "boolean" => FieldType::Bool,
            "timestamp" => FieldType::Timestamp,
            _ => return None,
        })
    }

    fn check_option_field_assignment_value(
        &mut self,
        field_type: &FieldType,
        field_name: &str,
        value: &Expr,
    ) -> Result<()> {
        if !matches!(field_type, FieldType::Option(_)) {
            return Ok(());
        }
        if Self::field_type_accepts_none_literal(field_type, value) {
            return Ok(());
        }

        let Some(value_type) = self.field_type_for_assignment_value(value) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "cannot prove value assigned to field '{}' has type '{}'",
                    field_name, field_type
                ),
                location: Some(self.span_to_source_location(value.span())),
            });
        };
        if &value_type != field_type || !Self::field_type_is_strictly_proven(&value_type) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "cannot assign value of type '{}' to field '{}' of type '{}'",
                    value_type, field_name, field_type
                ),
                location: Some(self.span_to_source_location(value.span())),
            });
        }
        Ok(())
    }

    fn field_type_for_assignment_value(&mut self, value: &Expr) -> Option<FieldType> {
        if let Some(field_type) = super::collections::infer_field_type_from_expr(value) {
            return Some(field_type);
        }
        if let Expr::FunctionCall { name, args, .. } = value
            && name == "Some"
            && args.len() == 1
        {
            let inner_ty = self.infer_expr_type(&args[0]).ok()?;
            let inner_field_type = Self::inferred_type_to_field_type(&inner_ty)?;
            return Some(FieldType::Option(Box::new(inner_field_type)));
        }
        let inferred = self.infer_expr_type(value).ok()?;
        Self::inferred_type_to_field_type(&inferred)
    }

    fn inferred_type_to_field_type(ty: &shape_runtime::type_system::Type) -> Option<FieldType> {
        use shape_runtime::type_system::Type;
        let Type::Concrete(annotation) = ty else {
            return None;
        };
        if annotation.as_type_name_str() == Some("unknown") {
            return None;
        }
        Some(Self::type_annotation_to_field_type(annotation))
    }

    fn field_type_is_strictly_proven(field_type: &FieldType) -> bool {
        match field_type {
            FieldType::Any => false,
            FieldType::Object(name) => name != "unknown",
            FieldType::Array(inner) | FieldType::Option(inner) | FieldType::Set(inner) => {
                Self::field_type_is_strictly_proven(inner)
            }
            FieldType::HashMap { key, value } => {
                Self::field_type_is_strictly_proven(key)
                    && Self::field_type_is_strictly_proven(value)
            }
            FieldType::F64
            | FieldType::I64
            | FieldType::Bool
            | FieldType::String
            | FieldType::Timestamp
            | FieldType::Decimal
            | FieldType::I8
            | FieldType::U8
            | FieldType::I16
            | FieldType::U16
            | FieldType::I32
            | FieldType::U32
            | FieldType::U64 => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_let_expression_binding_is_immutable() {
        let code = r#"
            function test() {
                return let x = 5 in {
                    x = 6
                    x
                }
            }
        "#;
        let program = parse_program(code).expect("parse failed");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "reassigning let-expression binding should fail"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("immutable variable 'x'"),
            "unexpected error: {}",
            err
        );
    }
}
