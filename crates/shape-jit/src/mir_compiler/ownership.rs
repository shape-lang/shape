//! Ownership-aware codegen: Move, Copy, Drop.
//!
//! This is the core of what makes MirToIR correct where BytecodeToIR isn't:
//! - Move: read value, null source slot (prevents double-drop)
//! - Copy: read value, arc_retain if heap type (Arc::clone)
//! - Drop: arc_release for heap types, no-op for primitives

use cranelift::prelude::*;
use shape_value::ValueWordExt;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
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
                // Copy: read the value. For heap types, increment the refcount.
                let val = self.read_place(place)?;
                {
                    let slot = place.root_local();
                    // Phase E: stack closures carry a raw Cranelift stack-slot
                    // address. No refcount → skip arc_retain.
                    let is_stack_closure = matches!(place, Place::Local(s) if self.stack_closure_slots.contains_key(s));
                    let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
                    // Native primitive types (Float64, Int32, Bool) never need refcounting.
                    if !is_stack_closure && !super::types::is_native_slot(slot_kind) {
                        let type_info = self
                            .local_types
                            .get(slot.0 as usize)
                            .cloned()
                            .unwrap_or(LocalTypeInfo::Unknown);
                        if super::types::is_heap_type(&type_info) {
                            self.builder.ins().call(self.ffi.arc_retain, &[val]);
                        } else if matches!(type_info, LocalTypeInfo::Unknown) {
                            self.builder.ins().call(self.ffi.arc_retain, &[val]);
                        }
                    }
                }
                Ok(val)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
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
    /// v2-boundary: Int, None, StringId, Str, Function, Method, ClosurePlaceholder
    /// all produce I64 (ValueWord bit-pattern) because the VM stack and FFI
    /// boundaries expect the uniform 8-byte slot.
    pub(crate) fn compile_constant(&mut self, constant: &MirConstant) -> Result<Value, String> {
        match constant {
            MirConstant::Int(n) => {
                // NaN-box the integer (I64 can't distinguish native from NaN-boxed).
                let boxed = shape_value::ValueWord::from_i64(*n).raw_bits();
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
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
                // Look up the string from the string table and NaN-box it at compile time.
                let idx = *id as usize;
                if idx < self.strings.len() {
                    let s = self.strings[idx].clone();
                    let boxed = crate::ffi::value_ffi::box_string(s);
                    Ok(self.builder.ins().iconst(types::I64, boxed as i64))
                } else {
                    Ok(self
                        .builder
                        .ins()
                        .iconst(types::I64, 0i64))
                }
            }
            MirConstant::Str(s) => {
                // String literal carried in MIR — NaN-box at compile time.
                let boxed = crate::ffi::value_ffi::box_string(s.clone());
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::Function(name) => {
                // Resolve function name to index, NaN-box as function ref
                if let Some(&idx) = self.function_indices.get(name.as_str()) {
                    let boxed = shape_value::ValueWord::from_function(idx).raw_bits();
                    Ok(self.builder.ins().iconst(types::I64, boxed as i64))
                } else {
                    Ok(self.builder.ins().iconst(types::I64, 0i64))
                }
            }
            MirConstant::Method(name) => {
                // Method name for dispatch — NaN-box the string at compile time.
                let boxed = crate::ffi::value_ffi::box_string(name.clone());
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
                        let boxed =
                            shape_value::ValueWord::from_function(fid).raw_bits();
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
        // v2 fast path: typed-array slots hold raw `*mut TypedArray<T>`
        // pointers (with their own HeapHeader refcount). The legacy
        // `arc_release` FFI expects a NaN-boxed value and would either
        // crash or no-op on a raw pointer. Skip the release for now —
        // a follow-up will plumb v2_release through to call `jit_v2_release`.
        if self.v2_typed_array_elem_kind(place).is_some() {
            self.null_place(place)?;
            return Ok(());
        }

        // Phase E: stack-allocated closures own no refcounted data; the
        // Cranelift stack slot is freed implicitly at function return.
        // Skip arc_release to avoid accidentally treating the raw
        // stack-slot address as a NaN-boxed heap handle.
        if let Place::Local(slot_id) = place {
            if self.stack_closure_slots.contains_key(slot_id) {
                self.null_place(place)?;
                return Ok(());
            }
        }

        // Track A.1D.2: the OwnedMutable capture slot holds a raw
        // `*mut ValueWord` cell pointer, not a NaN-boxed refcounted
        // value. Reclaim is handled by `release_typed_closure`'s
        // `Box::from_raw` (A.1A) when the closure itself is dropped;
        // frame-exit `Drop` on the capture slot must be a no-op.
        // `null_place` also early-returns for these slots (preserving
        // the cell pointer for release_typed_closure), so we skip it
        // here too.
        //
        // Track A.1E: parallel rationale for Shared capture slots.
        // The slot holds a `*const SharedCell` (Arc pointer). Calling
        // `arc_release` on it would misinterpret the Arc pointer as a
        // NaN-boxed value. The Arc share is reclaimed exactly once by
        // `release_typed_closure`'s `Arc::from_raw` (gated on the
        // `shared_capture_mask`, A.1A). Frame-exit `Drop` must be a
        // no-op.
        if let Place::Local(slot_id) = place {
            if self.owned_mutable_capture_slots.contains_key(slot_id)
                || self.shared_capture_slots.contains_key(slot_id)
            {
                return Ok(());
            }
        }

        // Session 1 Commit 3: SharedCow outer-scope local slots hold a
        // raw `*const SharedCell` Arc pointer (allocated at function
        // entry by `initialize_shared_local_slots`). The MIR emits
        // `StatementKind::Drop(Place::Local(slot))` at scope exit; we
        // consume the slot's one strong share via
        // `jit_arc_shared_release`. Mirrors the interpreter's
        // `op_drop_shared_local` handler: reconstruct `Arc::from_raw`,
        // drop it (one atomic decrement), then overwrite the slot
        // bits with 0 so any reentrant access reports a null pointer
        // rather than dereferencing freed memory.
        //
        // SAFETY: the pointer is a live `Arc::into_raw`-produced
        // `*const SharedCell` (from `jit_alloc_shared_cell`) with at
        // least one outstanding strong share at the time of release.
        // Additional shares minted by `jit_arc_shared_retain` for
        // capturing closures are reclaimed independently by
        // `release_typed_closure`.
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

        let slot = place.root_local();
        let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);

        // Native primitive types never need refcounting.
        if !super::types::is_native_slot(slot_kind) {
            let val = self.read_place(place)?;
            let type_info = self
                .local_types
                .get(slot.0 as usize)
                .cloned()
                .unwrap_or(LocalTypeInfo::Unknown);

            if super::types::is_heap_type(&type_info) {
                self.builder.ins().call(self.ffi.arc_release, &[val]);
            } else if matches!(type_info, LocalTypeInfo::Unknown) {
                self.builder.ins().call(self.ffi.arc_release, &[val]);
            }
        }

        // Null the slot to prevent use-after-drop.
        self.null_place(place)?;
        Ok(())
    }

    /// Release the old value of a local before overwriting it.
    /// This prevents Arc leaks when a heap local is reassigned.
    pub(crate) fn release_old_value_if_heap(
        &mut self,
        place: &Place,
    ) -> Result<(), String> {
        // v2 fast path: same reasoning as `emit_drop` — skip the legacy
        // arc_release for slots whose ConcreteType is a v2 typed array.
        if matches!(place, Place::Local(_))
            && self.v2_typed_array_elem_kind(place).is_some()
        {
            return Ok(());
        }

        // Phase E: stack-resident closure handles are raw stack-slot
        // addresses, not refcounted heap pointers. Skip arc_release.
        if let Place::Local(slot_id) = place {
            if self.stack_closure_slots.contains_key(slot_id) {
                return Ok(());
            }
        }

        // Track A.1D.2: the "old value" of an OwnedMutable capture slot
        // is the raw `*mut ValueWord` cell pointer bits — NOT a
        // NaN-boxed heap handle. Calling `arc_release` on it would
        // misinterpret the pointer as a refcounted value and crash /
        // double-free. The cell's interior contents are reclaimed
        // exactly once by `release_typed_closure`'s `Box::from_raw`
        // loop (A.1A) when the closure's refcount hits zero; the
        // interpreter's `op_store_owned_mutable_capture` likewise does
        // not release the old inner value (see its SAFETY note), so
        // skipping release here is parity-correct.
        //
        // Track A.1E: parallel rationale for Shared capture slots. The
        // slot holds a `*const SharedCell` Arc pointer. `arc_release`
        // would misinterpret it. The Arc share is reclaimed once by
        // `release_typed_closure`'s `Arc::from_raw` (gated on
        // `shared_capture_mask`, A.1A); the interpreter's
        // `op_store_shared_capture` does not modify the Arc strong
        // count either.
        if let Place::Local(slot_id) = place {
            if self.owned_mutable_capture_slots.contains_key(slot_id)
                || self.shared_capture_slots.contains_key(slot_id)
            {
                return Ok(());
            }
        }

        // Session 1 Commit 3: SharedCow outer-scope local slots: the
        // "old value" is a `*const SharedCell` pointer — not a
        // refcounted NaN-boxed heap value. `jit_arc_shared_release`
        // runs once at `Drop(slot)`, not on every reassignment.
        // Subsequent assignments only update the cell's payload
        // via the lock-gated store in `write_place`; the cell
        // pointer stays intact.
        if let Place::Local(slot_id) = place {
            if self.shared_local_slots.contains(slot_id) {
                return Ok(());
            }
        }

        let slot = place.root_local();
        if matches!(place, Place::Local(_)) {
            let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
            // Native primitive types never need refcounting.
            if super::types::is_native_slot(slot_kind) {
                return Ok(());
            }

            let type_info = self
                .local_types
                .get(slot.0 as usize)
                .cloned()
                .unwrap_or(LocalTypeInfo::Unknown);

            if super::types::is_heap_type(&type_info)
                || matches!(type_info, LocalTypeInfo::Unknown)
            {
                let old_val = self.read_place(place)?;
                self.builder.ins().call(self.ffi.arc_release, &[old_val]);
            }
        }
        Ok(())
    }
}
