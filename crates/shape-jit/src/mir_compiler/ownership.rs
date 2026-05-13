//! Ownership-aware codegen: Move, Copy, Drop.
//!
//! This is the core of what makes MirToIR correct where BytecodeToIR isn't:
//! - Move: read value, null source slot (prevents double-drop)
//! - Copy: read value, arc_retain if heap type (Arc::clone)
//! - Drop: arc_release for heap types, no-op for primitives
//!
//! ## Refcount discrimination (W11-jit-new-array, ADR-006 §2.7.5 / §2.7.6 / Q8)
//!
//! Post-strict-typing the kind IS the discriminator that decides refcount
//! semantics; there is no tag-bit probing. The discrimination here uses
//! [`shape_value::NativeKind::is_refcounted`] which returns `true` for the
//! two heap-pointer kinds (`String`, `Ptr(HeapKind::*)`) and `false` for
//! every numeric / bool / nullable-scalar kind — including `NativeKind::Int64`,
//! which the legacy `types::is_native_slot` predicate excluded (the legacy
//! exclusion was correct under the deleted ValueWord ABI where an `Int64`
//! slot might carry NaN-boxed pointer bits; under strict typing an `Int64`
//! slot stores a raw native `i64`, period).
//!
//! When the slot's `NativeKind` is not proven by either source (the
//! bytecode compiler's seed, the MIR-level forward/backward inference in
//! `infer_slot_kinds`), the response is **surface-and-stop** — never a
//! kind-blind fall-through to `arc_retain` / `arc_release`. Defaulting
//! "unknown kind → assume heap and retain" is the W-series Bool-default
//! defection-attractor (CLAUDE.md "Forbidden rationalizations": *"Soft-fail
//! counter for now, harden later."*) applied to a different surface; the
//! prior W11-jit-new-array close attempted the symmetric variant
//! ("unknown kind → silently skip retain") via a no-op FFI body, which
//! refcount-leaks every heap value the JIT routes through. Both are refused
//! on sight per §2.7.7 #9 / W10 jit-playbook §5.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::NativeKind;

/// Refcount disposition for an ownership-aware codegen site.
///
/// Computed from the slot's proven `NativeKind` (per ADR-006 §2.7.5
/// stamp-at-compile-time). The variants encode every legitimate answer the
/// emitter can give without falling back to a kind-blind default.
#[derive(Debug, Clone, Copy)]
enum RefcountDisposition {
    /// The slot is a raw scalar / bool / nullable-scalar — emit no
    /// retain/release call.
    Skip,
    /// The slot is a heap-pointer kind (`String` / `Ptr(HeapKind::*)`) —
    /// emit the matching retain/release call.
    Refcounted,
    /// The slot is one of the "raw pointer to a typed cell" carriers the
    /// MIR uses for closure capture cells / shared cells / stack closures.
    /// These have their own dedicated retain/release path (the matching
    /// per-FieldKind FFI in `ffi/object/closure.rs`, or no retain at all
    /// for stack closures) — emit nothing here.
    Skip_TypedCellCarrier,
}

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compute the refcount disposition for a place's root local.
    ///
    /// Returns the disposition or a surface-and-stop error when the slot's
    /// `NativeKind` cannot be resolved from either the bytecode-compiler
    /// seed or the MIR-level inference. The error path is the §2.7.7 #9
    /// principled response; no Bool-default fall-through.
    fn refcount_disposition(&self, place: &Place) -> Result<RefcountDisposition, String> {
        let slot = place.root_local();

        // Stack closures: the slot value is a raw Cranelift stack-slot
        // address, not a refcounted handle. (Phase E.)
        if let Place::Local(slot_id) = place {
            if self.stack_closure_slots.contains_key(slot_id) {
                return Ok(RefcountDisposition::Skip_TypedCellCarrier);
            }
            // Track A.1D.2 / A.1E: OwnedMutable / Shared capture slots
            // hold raw `*mut ValueWord` / `*const SharedCell` pointers
            // whose lifecycle is owned by `release_typed_closure`
            // (`ClosureLayout`'s owned-mutable / shared masks). Frame-
            // exit retain/release on these slots would mis-interpret
            // the pointer as a NaN-boxed heap handle.
            if self.owned_mutable_capture_slots.contains_key(slot_id)
                || self.shared_capture_slots.contains_key(slot_id)
            {
                return Ok(RefcountDisposition::Skip_TypedCellCarrier);
            }
            // Session 1 Commit 3: SharedCow outer-scope local slots
            // hold a `*const SharedCell` Arc pointer; their lifecycle
            // is `jit_arc_shared_release` (not the generic
            // `jit_arc_release`) at `Drop`. Skip here.
            if self.shared_local_slots.contains(slot_id) {
                return Ok(RefcountDisposition::Skip_TypedCellCarrier);
            }
        }

        // v2 typed-array slots: the value is a raw `*mut TypedArray<T>`
        // pointer with inline `HeapHeader` refcount. The kinded `v2`
        // retain/release surface is the right path (a §2.7.14 follow-up);
        // skip the generic arc_retain/release here.
        if matches!(place, Place::Local(_)) && self.v2_typed_array_elem_kind(place).is_some() {
            return Ok(RefcountDisposition::Skip_TypedCellCarrier);
        }

        // W12-jit-binop-after-heap-read-kind-tracker: for projection
        // places (`Place::Field` / `Place::Index`), the value being
        // copied is the field's / element's value, NOT the base struct/
        // array's heap handle. Refcount disposition must follow the
        // PROJECTED kind. `place_native_kind` does the project lookup
        // through the producer-side `field_native_kinds` map (§2.7.5
        // producer classification) for fields and through
        // `concrete_types`'s `Array<scalar>` shape for indexes — the
        // same kind sources the BinaryOp lowering picker uses.
        //
        // Without this projection, `Copy(Field(p_TypedObject, x_Int64))`
        // routed `refcount_disposition` to the base's `Ptr(TypedObject)`
        // kind (refcounted), then `compile_operand`'s Copy arm called
        // `arc_retain(i64_3_field_value)` — segfaulting in
        // `Arc::increment_strong_count` interpreting the integer 3 as
        // a pointer.
        match place {
            Place::Field(_, _) | Place::Index(_, _) => {
                match self.place_native_kind(place) {
                    Some(k) if k.is_refcounted() => {
                        return Ok(RefcountDisposition::Refcounted);
                    }
                    Some(_) => return Ok(RefcountDisposition::Skip),
                    None => {
                        // Projection kind genuinely unproven at this
                        // consumer site (e.g. the field name isn't in
                        // `field_native_kinds` because the producer-side
                        // walk didn't see the ObjectStore that stamps
                        // it, or the array's `ConcreteType` isn't
                        // `Array<scalar>`). Fall through to the
                        // root-local-kind dispatch below — that arm
                        // already has the surface-and-stop discipline
                        // for genuinely-unproven kinds via the
                        // `LocalTypeInfo` arms.
                    }
                }
            }
            _ => {}
        }

        // Authoritative kind source: the slot's proven `NativeKind` from
        // bytecode-compiler seed + MIR-level inference. Under §2.7.5
        // stamp-at-compile-time this is the canonical refcount
        // discriminator.
        let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
        match slot_kind {
            Some(k) if k.is_refcounted() => Ok(RefcountDisposition::Refcounted),
            Some(_) => Ok(RefcountDisposition::Skip),
            None => {
                // Kind genuinely unproven by both inference passes. Per
                // §2.7.7 #9 / CLAUDE.md "Forbidden rationalizations" the
                // emitter does NOT default to "assume heap and retain"
                // (the W-series Bool-default attractor); surface-and-stop
                // at JIT compile time so the program falls back to the
                // interpreter rather than refcount-leak / segfault.
                //
                // Practical fallback for the implicit-return slot 0 +
                // unused-tail-slot cases the MIR-inference pass leaves
                // unproven: those slots are never written via an Assign
                // the inference can see, so they never carry a live value
                // — emit no retain/release. We discriminate this from a
                // genuine kind-source gap via `LocalTypeInfo`: `Copy`
                // and `NonCopy` are bytecode-compiler-authoritative
                // (primitive / heap), `Unknown` is the "no annotation
                // and no Assign" path — that's the unused / implicit
                // slot, safe to skip.
                let type_info = self
                    .local_types
                    .get(slot.0 as usize)
                    .cloned()
                    .unwrap_or(LocalTypeInfo::Unknown);
                match type_info {
                    LocalTypeInfo::Copy => Ok(RefcountDisposition::Skip),
                    LocalTypeInfo::NonCopy => {
                        // The bytecode compiler classified this as heap,
                        // but MIR inference couldn't prove the kind.
                        // Surface-and-stop: a `NonCopy` slot needs a
                        // proven heap `NativeKind` to dispatch the
                        // correct retain (per-kind §2.7.6 / Q8). Falling
                        // through to a kind-blind retain on `String` /
                        // `Ptr(HeapKind::*)` ambiguity is the W-series
                        // attractor.
                        Err(format!(
                            "MirToIR ownership: SURFACE — slot {} has \
                             LocalTypeInfo::NonCopy but MIR inference did \
                             not prove its NativeKind. Refcount dispatch \
                             requires a proven `NativeKind::String` or \
                             `NativeKind::Ptr(HeapKind::*)` per ADR-006 \
                             §2.7.5 / §2.7.6 / Q8. Tracked as a \
                             W11-jit-new-array kind-source-gap follow-up. \
                             ADR-006 §2.7.7 #9 (no Bool-default fallback).",
                            slot.0
                        ))
                    }
                    LocalTypeInfo::Unknown => {
                        // Implicit-return / unused / dead-store slot —
                        // no live value, no refcount work. This is the
                        // structurally-safe arm: the slot has neither a
                        // proven kind nor a bytecode-compiler heap
                        // classification, so no Assign(_, _) flows a
                        // refcounted value into it.
                        Ok(RefcountDisposition::Skip)
                    }
                }
            }
        }
    }

    /// Public wrapper for `refcount_disposition` — returns `true` when the
    /// place's slot is heap-kinded (a refcount call is required). Used by
    /// `Rvalue::Clone` in `rvalues.rs` to share the same discrimination
    /// path as `compile_operand`'s `Copy` arm.
    pub(crate) fn refcount_disposition_for_place(&self, place: &Place) -> Result<bool, String> {
        Ok(matches!(
            self.refcount_disposition(place)?,
            RefcountDisposition::Refcounted
        ))
    }

    /// Compile an Operand, respecting Move/Copy ownership semantics.
    pub(crate) fn compile_operand(&mut self, operand: &Operand) -> Result<Value, String> {
        match operand {
            Operand::Move(place) | Operand::MoveExplicit(place) => {
                // Move: read the value, then null the source to prevent double-drop.
                let val = self.read_place(place)?;
                self.null_place(place)?;
                Ok(val)
            }
            Operand::Copy(place) => {
                // Copy: read the value. For heap-kind slots, increment the refcount.
                let val = self.read_place(place)?;
                if matches!(
                    self.refcount_disposition(place)?,
                    RefcountDisposition::Refcounted
                ) {
                    let retain_func = self.retain_func_for_place(place);
                    self.builder.ins().call(retain_func, &[val]);
                }
                Ok(val)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
        }
    }

    /// Pick the kind-appropriate retain FFI for a place. ADR-006 §2.7.17
    /// adopted `Arc<ResultData>` / `Arc<OptionData>` as the strict-typed
    /// Result/Option carriers; their refcount lives at offset -16 per
    /// Rust Arc contract, NOT at offset 4 like the legacy
    /// `UnifiedValue<T>` shape. The legacy `jit_arc_retain` would write
    /// to the wrong offset and corrupt the inner payload.
    ///
    /// Round 7A added `arc_result_retain` / `arc_option_retain` for
    /// Result/Option. W12-jit-collection-arc-ffi-ctors-and-refcount
    /// (Phase 3 cluster-0 Round 9 / 8B.1, 2026-05-13) extends the
    /// dispatch with 8 more typed-Arc collection carriers — HashSet,
    /// HashMap, Deque, PriorityQueue, Channel, Mutex, Atomic, Lazy.
    /// All 10 dispatch arms operate on `Arc::into_raw(Arc<XData>) as
    /// u64` carriers (refcount at offset -16); the legacy `arc_retain`
    /// fallback stays for kinds NOT in the typed-Arc family
    /// (Array / TypedObject / Closure / etc. — still on
    /// `UnifiedValue<T>` HeapHeader at offset 4).
    pub(crate) fn retain_func_for_place(
        &self,
        place: &Place,
    ) -> cranelift::codegen::ir::FuncRef {
        use shape_value::heap_value::HeapKind;
        use shape_vm::type_tracking::NativeKind;
        let kind = self.place_native_kind(place);
        match kind {
            Some(NativeKind::Ptr(HeapKind::Result)) => self.ffi.arc_result_retain,
            Some(NativeKind::Ptr(HeapKind::Option)) => self.ffi.arc_option_retain,
            Some(NativeKind::Ptr(HeapKind::HashSet)) => self.ffi.arc_hashset_retain,
            Some(NativeKind::Ptr(HeapKind::HashMap)) => self.ffi.arc_hashmap_retain,
            Some(NativeKind::Ptr(HeapKind::Deque)) => self.ffi.arc_deque_retain,
            Some(NativeKind::Ptr(HeapKind::PriorityQueue)) => self.ffi.arc_priorityqueue_retain,
            Some(NativeKind::Ptr(HeapKind::Channel)) => self.ffi.arc_channel_retain,
            Some(NativeKind::Ptr(HeapKind::Mutex)) => self.ffi.arc_mutex_retain,
            Some(NativeKind::Ptr(HeapKind::Atomic)) => self.ffi.arc_atomic_retain,
            Some(NativeKind::Ptr(HeapKind::Lazy)) => self.ffi.arc_lazy_retain,
            // W12-jit-string-carrier-unification (Phase 3 cluster-0 Round 12
            // T2/T3, 2026-05-13). ADR-006 §2.7.5 `NativeKind::String` slots
            // carry `Arc::into_raw(Arc<String>) as u64`; retain bumps the
            // Rust Arc control-block refcount at offset -16 via
            // `Arc::increment_strong_count::<String>`. The legacy
            // `arc_retain` would write a `fetch_add` at offset +4 — inside
            // the `String` payload, scribbling on `ptr/cap/len`.
            Some(NativeKind::String) => self.ffi.arc_string_retain,
            _ => self.ffi.arc_retain,
        }
    }

    /// Mirror of `retain_func_for_place` for release.
    pub(crate) fn release_func_for_place(
        &self,
        place: &Place,
    ) -> cranelift::codegen::ir::FuncRef {
        use shape_value::heap_value::HeapKind;
        use shape_vm::type_tracking::NativeKind;
        let kind = self.place_native_kind(place);
        match kind {
            Some(NativeKind::Ptr(HeapKind::Result)) => self.ffi.arc_result_release,
            Some(NativeKind::Ptr(HeapKind::Option)) => self.ffi.arc_option_release,
            Some(NativeKind::Ptr(HeapKind::HashSet)) => self.ffi.arc_hashset_release,
            Some(NativeKind::Ptr(HeapKind::HashMap)) => self.ffi.arc_hashmap_release,
            Some(NativeKind::Ptr(HeapKind::Deque)) => self.ffi.arc_deque_release,
            Some(NativeKind::Ptr(HeapKind::PriorityQueue)) => self.ffi.arc_priorityqueue_release,
            Some(NativeKind::Ptr(HeapKind::Channel)) => self.ffi.arc_channel_release,
            Some(NativeKind::Ptr(HeapKind::Mutex)) => self.ffi.arc_mutex_release,
            Some(NativeKind::Ptr(HeapKind::Atomic)) => self.ffi.arc_atomic_release,
            Some(NativeKind::Ptr(HeapKind::Lazy)) => self.ffi.arc_lazy_release,
            // W12-jit-string-carrier-unification: mirror of the
            // `retain_func_for_place` String arm.
            Some(NativeKind::String) => self.ffi.arc_string_release,
            _ => self.ffi.arc_release,
        }
    }

    /// Compile an operand without ownership tracking (raw value access).
    /// Used for index operands in Place::Index where we just need the value.
    pub(crate) fn compile_operand_raw(&mut self, operand: &Operand) -> Result<Value, String> {
        match operand {
            Operand::Move(place) | Operand::MoveExplicit(place) | Operand::Copy(place) => {
                self.read_place(place)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
        }
    }

    /// Session 1 Commit 3: compile an operand for a `ClosureCapture`
    /// slot whose capture kind is `Shared`.
    ///
    /// Semantics: when the capture's source is an outer-scope `var`
    /// local that has been promoted to `SharedCow` storage, the
    /// closure capture needs the RAW `*const SharedCell` pointer bits
    /// — not the locked payload. This matches the interpreter's
    /// `expressions/closures.rs` path, which emits
    /// `LoadLocal(outer_var_slot)` immediately after `AllocSharedLocal`
    /// to push the pointer bits that `op_make_closure` then feeds
    /// through `Arc::increment_strong_count`.
    ///
    /// For all other operand shapes (Constant, Copy/Move of a slot
    /// that isn't a SharedCow local), defer to the standard
    /// `compile_operand`. This keeps the legacy Immutable /
    /// OwnedMutable capture paths untouched.
    pub(crate) fn compile_operand_for_shared_capture(
        &mut self,
        operand: &Operand,
    ) -> Result<Value, String> {
        if let Operand::Move(place)
        | Operand::MoveExplicit(place)
        | Operand::Copy(place) = operand
        {
            if let Place::Local(slot) = place {
                if self.shared_local_slots.contains(slot) {
                    // Bypass the lock-gated read in `read_place` and
                    // produce the raw pointer bits held in the slot's
                    // Cranelift variable.
                    let var = *self.locals.get(slot).ok_or_else(|| {
                        format!("MirToIR: unknown local slot {}", slot)
                    })?;
                    return Ok(self.builder.use_var(var));
                }
            }
        }
        self.compile_operand(operand)
    }

    /// Compile a MIR constant to a Cranelift value.
    ///
    /// Returns native types when possible (F64 for floats, I64 for ints, I8 for bools).
    /// Consumers that need an I64 slot (e.g. for a dynamic local) rely on
    /// `ensure_kind` in `conversions.rs` to do the width extension.
    /// Per ADR-006 §2.7.5 the JIT FFI carrier is `(u64, NativeKind)` — the
    /// constant's `NativeKind` is stamped at the call signature; the bits
    /// emitted here are raw native u64 with no NaN-box / `tag_bits` wrap.
    pub(crate) fn compile_constant(&mut self, constant: &MirConstant) -> Result<Value, String> {
        match constant {
            MirConstant::Int(n) => {
                // Raw native i64 bits; kind companion is `NativeKind::Int64`
                // stamped at the JIT-FFI carrier site.
                Ok(self.builder.ins().iconst(types::I64, *n))
            }
            MirConstant::Float(bits) => {
                // Native F64 — direct float constant. ~100x faster than FFI path.
                Ok(self.builder.ins().f64const(f64::from_bits(*bits)))
            }
            MirConstant::Bool(b) => {
                // Native I8 bool — 0 or 1.
                Ok(self.builder.ins().iconst(types::I8, *b as i64))
            }
            MirConstant::None => {
                Ok(self
                    .builder
                    .ins()
                    .iconst(types::I64, 0i64))
            }
            MirConstant::StringId(id) => {
                // W12-jit-string-carrier-unification (Phase 3 cluster-0 Round
                // 12 T2/T3, 2026-05-13). ADR-006 §2.7.5: a `NativeKind::String`
                // slot carries `Arc::into_raw(Arc<String>) as u64`, refcount
                // at offset -16 per the standard Rust Arc layout. The VM-side
                // consumer (`set_methods.rs::result_slot_to_string_arc` and
                // `KindedSlot::Drop` for `NativeKind::String`) decodes via
                // `Arc::from_raw(bits as *const String)` / `Arc::decrement_
                // strong_count::<String>(bits)`. Pre-Round-12 this site
                // emitted `box_string(s)` returning `Box::into_raw(Box::new(
                // UnifiedValue<Arc<String>>))` — wrong carrier shape; the
                // VM consumer's `Arc::from_raw` read the UnifiedValue header
                // bytes as `String` pointer/cap/len, segfaulting on access.
                //
                // `arc_string_constant` boosts the initial refcount to keep
                // the constant alive across the JIT-compiled function's
                // full lifetime — see the helper's docstring for the
                // permanent-share discipline.
                let idx = *id as usize;
                if idx < self.strings.len() {
                    let s = self.strings[idx].clone();
                    let boxed = crate::ffi::string::arc_string_constant(s);
                    Ok(self.builder.ins().iconst(types::I64, boxed as i64))
                } else {
                    Ok(self
                        .builder
                        .ins()
                        .iconst(types::I64, 0i64))
                }
            }
            MirConstant::Str(s) => {
                // String literal carried in MIR. Same §2.7.5 producer
                // discipline as `MirConstant::StringId` above —
                // `Arc::into_raw(Arc<String>) as u64` with refcount boosted
                // for constant-lifetime stability.
                let boxed = crate::ffi::string::arc_string_constant(s.clone());
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::Function(name) => {
                // Resolve function name to index. Per ADR-006 §2.7.5 the
                // JIT-FFI carrier flows the function-ref kind on the
                // companion; the boxing helper at the value-ffi boundary
                // produces the raw u64 the carrier wraps.
                if let Some(&idx) = self.function_indices.get(name.as_str()) {
                    let boxed = crate::ffi::value_ffi::box_function(idx);
                    Ok(self.builder.ins().iconst(types::I64, boxed as i64))
                } else {
                    Ok(self.builder.ins().iconst(types::I64, 0i64))
                }
            }
            MirConstant::Method(name) => {
                // Method name. Per `rvalues.rs:310` the operand-kind stamp
                // for `MirConstant::Method` is `NativeKind::String` — the
                // §2.7.5 String carrier shape. W12-jit-string-carrier-
                // unification migrates this arm to match the stamp.
                //
                // Note: `MirConstant::Method` is principally used as the
                // `func` field of a Call terminator (see `terminators.rs`
                // method-call path); the method-name push at line 235 of
                // that file uses `crate::ffi::value_ffi::box_string`
                // directly (JIT-internal NaN-box; dispatch shell decodes
                // via the same NaN-box `unbox_string`). This
                // `compile_constant` arm covers the residual case where
                // `MirConstant::Method` flows as a value operand — its
                // stamp on the parallel-kind track says `String`, so the
                // §2.7.5 Arc-shape carrier is the correct producer.
                let boxed = crate::ffi::string::arc_string_constant(name.clone());
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::ClosurePlaceholder => {
                // Canonical path: the bytecode compiler's back-patcher rewrites
                // this to `Function(name)` during final MIR assembly
                // (`shape-vm/src/compiler/functions.rs` + `compiler_impl_reference_model.rs`).
                //
                // JIT-side fallback: monomorphization-triggered
                // `compile_function` clears `closure_function_ids` before the
                // top-level MIR patching runs, so unpatched placeholders leak
                // into the MIR we receive for top-level code. `scan_closure_placeholder_fids`
                // (called at MirToIR construction time) replays the same scan the
                // bytecode patcher would have run and resolves the N-th unpaired
                // placeholder to `__closure_<N>` via `function_indices`. We
                // consume that pairing here in statement-visit order.
                let idx = self.next_closure_placeholder_idx.get();
                self.next_closure_placeholder_idx.set(idx + 1);
                let fid_opt = self.closure_placeholder_fids.get(idx).copied();
                if let Some(fid) = fid_opt {
                    if fid != u16::MAX {
                        let boxed = crate::ffi::value_ffi::box_function(fid);
                        return Ok(self.builder.ins().iconst(types::I64, boxed as i64));
                    }
                }
                // Exhausted side-table or sentinel (capture-paired placeholder,
                // whose closure allocation is handled by `emit_heap_closure` /
                // `emit_stack_closure`; this Assign is a dead store the caller
                // discards). Preserve the legacy "return null bits" behaviour
                // so the JIT's error path still matches the pre-fix contract
                // if the scan misses.
                Ok(self.builder.ins().iconst(types::I64, 0i64))
            }
        }
    }

    /// Emit Drop for a local: release refcount if it's a heap type.
    pub(crate) fn emit_drop(&mut self, place: &Place) -> Result<(), String> {
        // Session 1 Commit 3 SharedCow path: outer-scope local slots
        // holding a `*const SharedCell` Arc pointer use the dedicated
        // `jit_arc_shared_release` (not the generic `arc_release`).
        // Handled here BEFORE the generic disposition because the
        // disposition's `Skip_TypedCellCarrier` arm would otherwise
        // suppress this required release.
        if let Place::Local(slot_id) = place {
            if self.shared_local_slots.contains(slot_id) {
                let var = *self.locals.get(slot_id).ok_or_else(|| {
                    format!("MirToIR: unknown local slot {}", slot_id)
                })?;
                let cell_ptr = self.builder.use_var(var);
                self.builder
                    .ins()
                    .call(self.ffi.arc_shared_release, &[cell_ptr]);
                // Mark the slot spent. 0 is a genuine null pointer,
                // distinct from NONE_BITS; matches the interpreter's
                // `self.stack[slot] = 0u64` step in
                // `op_drop_shared_local`.
                let zero = self.builder.ins().iconst(types::I64, 0);
                self.builder.def_var(var, zero);
                return Ok(());
            }
        }

        let disposition = self.refcount_disposition(place)?;
        match disposition {
            RefcountDisposition::Refcounted => {
                let val = self.read_place(place)?;
                let release_func = self.release_func_for_place(place);
                self.builder.ins().call(release_func, &[val]);
                self.null_place(place)?;
            }
            RefcountDisposition::Skip => {
                // Raw scalar / unused-tail-slot — no refcount work.
                // Still null the slot per the use-after-drop contract:
                // scalar slots get clobbered to 0, which is the
                // default-init value the runtime expects on re-read.
                self.null_place(place)?;
            }
            RefcountDisposition::Skip_TypedCellCarrier => {
                // OwnedMutable / Shared capture slots: lifecycle is
                // owned by `release_typed_closure`; per the
                // Track A.1D.2 / A.1E SAFETY notes the slot must NOT
                // be nulled here (the cell pointer is reclaimed by
                // the closure-drop, not by frame-exit).
                //
                // v2 typed-array / stack-closure slots: null the slot
                // to prevent use-after-drop reads from picking up the
                // raw pointer bits. (Match the prior behaviour.)
                let null_slot = match place {
                    Place::Local(slot_id) => {
                        !(self.owned_mutable_capture_slots.contains_key(slot_id)
                            || self.shared_capture_slots.contains_key(slot_id))
                    }
                    _ => true,
                };
                if null_slot {
                    self.null_place(place)?;
                }
            }
        }
        Ok(())
    }

    /// Release the old value of a local before overwriting it.
    /// This prevents Arc leaks when a heap local is reassigned.
    pub(crate) fn release_old_value_if_heap(
        &mut self,
        place: &Place,
    ) -> Result<(), String> {
        // Skip non-local places — only Place::Local supplies the
        // discrimination plumbing.
        if !matches!(place, Place::Local(_)) {
            return Ok(());
        }
        let disposition = self.refcount_disposition(place)?;
        match disposition {
            RefcountDisposition::Refcounted => {
                let old_val = self.read_place(place)?;
                let release_func = self.release_func_for_place(place);
                self.builder.ins().call(release_func, &[old_val]);
            }
            RefcountDisposition::Skip | RefcountDisposition::Skip_TypedCellCarrier => {
                // Scalar / typed-cell-carrier slots: no refcount work.
                // (TypedCellCarrier: the dedicated reclaim path runs at
                // Drop / closure-drop, not at reassign.)
            }
        }
        Ok(())
    }
}

// Silence unused-import warnings — `NativeKind` is imported for the
// `RefcountDisposition` deductions in `refcount_disposition`; if the
// reader uses no `NativeKind` directly, this stays a documentation
// anchor for the kind-discriminator import.
#[allow(dead_code)]
const _: fn() = || {
    let _ = NativeKind::Int64;
};
