//! MIR BasicBlock → Cranelift Block mapping.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::SlotId;
use shape_vm::type_tracking::NativeKind;

// Alias to avoid conflict with cranelift::prelude::types
use super::types as slot_types;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Create a Cranelift block for each MIR basic block.
    ///
    /// MIR's CFG maps 1:1 to Cranelift blocks — no block discovery needed
    /// (unlike the bytecode path which must scan for jump targets).
    pub(crate) fn create_blocks(&mut self) {
        for block in &self.mir.blocks {
            if block.id == shape_vm::mir::types::BasicBlockId(0) {
                // bb0 maps to the caller's entry block (already has function params).
                self.block_map.insert(block.id, self.entry_block);
            } else {
                let cl_block = self.builder.create_block();
                self.block_map.insert(block.id, cl_block);
            }
        }
    }

    /// Declare Cranelift variables for each MIR local slot.
    ///
    /// Variables are declared with their native Cranelift storage type:
    /// - Float64 → F64
    /// - Int32/UInt32 → I32
    /// - Bool/Int8/UInt8 → I8
    /// - Unknown/Dynamic/Int64/String/etc → I64 (dynamic)
    /// - Captured-cell carrier slots → I64 cell-pointer bits, regardless
    ///   of the slot's semantic inner kind.
    ///
    /// Variables are declared but NOT initialized here — initialization
    /// happens in initialize_locals() after switching to the entry block.
    pub(crate) fn declare_locals(&mut self) {
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            let kind = self.local_storage_kind(slot_id);
            let cl_type = slot_types::cranelift_type_for_slot(kind);

            let var = Variable::new(self.next_var);
            self.next_var += 1;
            self.builder.declare_var(var, cl_type);
            self.locals.insert(slot_id, var);
        }
    }

    /// Initialize all local variables to their type-appropriate zero/null.
    /// Must be called AFTER switching to the entry block.
    pub(crate) fn initialize_locals(&mut self) {
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            let kind = self.local_storage_kind(slot_id);

            if let Some(&var) = self.locals.get(&slot_id) {
                let init_val = self.default_value_for_kind(kind);
                self.builder.def_var(var, init_val);
            }
        }
    }

    /// Physical Cranelift storage kind for a MIR local slot.
    ///
    /// `slot_kinds` records the value kind produced by `read_place` and
    /// consumed by arithmetic/compare dispatch. Captured-cell slots are the
    /// exception: their variable physically stores an OwnedMutable/Shared
    /// cell pointer for the frame lifetime, while reads and writes project
    /// through that pointer using the side-table's inner kind. Declaring or
    /// initializing those slots at the inner width truncates or type-mismatches
    /// the pointer before the cell lowering can run.
    pub(crate) fn local_storage_kind(&self, slot_id: shape_vm::mir::types::SlotId) -> NativeKind {
        if self.owned_mutable_capture_slots.contains_key(&slot_id)
            || self.shared_capture_slots.contains_key(&slot_id)
            || self.shared_local_slots.contains_key(&slot_id)
        {
            // NOTE (ADR-006 §2.7.8): `shared_local_slots` now carries the
            // cell's INNER payload kind, but that kind must NOT leak here.
            // The slot's Cranelift variable physically holds the
            // `*const SharedCell` POINTER for the frame lifetime; declaring
            // it at the inner width would truncate or type-mismatch the
            // pointer before the cell lowering can project through it.
            return NativeKind::Int64;
        }

        slot_types::slot_kind_for_local(&self.slot_kinds, slot_id.0).unwrap_or(NativeKind::Int64)
    }

    /// Produce the default (zero/null) value for a given NativeKind.
    fn default_value_for_kind(&mut self, kind: NativeKind) -> Value {
        match kind {
            NativeKind::Float64 => self.builder.ins().f64const(0.0),
            NativeKind::Int32 | NativeKind::UInt32 => self.builder.ins().iconst(types::I32, 0),
            NativeKind::Int8 | NativeKind::UInt8 | NativeKind::Bool => {
                self.builder.ins().iconst(types::I8, 0)
            }
            NativeKind::Int16 | NativeKind::UInt16 => self.builder.ins().iconst(types::I16, 0),
            // v2-boundary: I64 NaN-boxed slots use TAG_NULL as default
            _ => self.builder.ins().iconst(types::I64, 0i64),
        }
    }

    /// Session 1 Commit 3: for every SharedCow local slot, allocate a
    /// fresh `Arc<SharedCell>` and store its pointer bits into the
    /// slot's Cranelift variable.
    ///
    /// The interpreter's `op_alloc_shared_local` promotes the slot
    /// lazily (only at the first `MakeClosure` that captures it). The
    /// JIT doesn't have visibility into that promotion point from MIR
    /// (MIR sees plain `Assign` / `Drop` on the slot); instead we
    /// eagerly allocate the cell at function entry. The initial
    /// payload is `NONE_BITS` (u64::MAX / TAG_NULL tag pattern) —
    /// subsequent `Assign` statements on the slot will lock-gated
    /// store the real value through the cell.
    ///
    /// Must be called AFTER `initialize_locals` and BEFORE function
    /// parameters are stored (shared locals are never parameters so
    /// there is no ordering conflict, but callers follow the same
    /// order for all setup helpers).
    ///
    /// SAFETY: the cell is allocated exactly once per function entry
    /// and released exactly once by `emit_drop` when the MIR emits
    /// `StatementKind::Drop(Place::Local(slot))` at scope exit. A
    /// function that never emits a matching `Drop` would leak one
    /// strong share per SharedCow slot; the MIR lowering pass is
    /// responsible for emitting balanced `Drop` statements.
    pub(crate) fn initialize_shared_local_slots(&mut self) -> Result<(), String> {
        if self.shared_local_slots.is_empty() {
            return Ok(());
        }
        if tracing::enabled!(target: "shape_jit", tracing::Level::DEBUG) {
            tracing::debug!(
                target: "shape_jit",
                shared_local_slots_len = self.shared_local_slots.len(),
                slots = ?self.shared_local_slots.iter().collect::<Vec<_>>(),
                "jit-init-shared local slot inventory",
            );
        }
        // Collect into a Vec to avoid borrowing self across the loop, and
        // sort for deterministic emission order.
        let mut slots: Vec<(SlotId, Option<NativeKind>)> = self
            .shared_local_slots
            .iter()
            .map(|(s, k)| (*s, *k))
            .collect();
        slots.sort_by_key(|(s, _)| s.0);

        // ── SURFACE-AND-STOP GATE (ADR-006 §2.7.8 / Q10) ────────────────
        //
        // Run BEFORE any cell is allocated. Every `jit_alloc_shared_cell`
        // is balanced by exactly one `jit_arc_shared_release` in
        // `emit_drop`; bailing part-way through the loop would leave the
        // emitted-but-abandoned allocations unmatched. Cranelift discards
        // the whole function body on Err, so an up-front refusal is the
        // only shape that cannot leak a strong share.
        //
        // Two refusal classes, both of which must be a clean whole-function
        // JIT bail to the interpreter — NEVER a defaulted kind, and never a
        // `todo!()` inside the `extern "C"` FFI (nounwind => SIGABRT, which
        // is exactly the defect this replaces):
        for (slot, kind) in &slots {
            let Some(kind) = kind else {
                // (a) Kind-source gap: neither the closure layout's
                // `capture_types` nor the slot's inferred `slot_kinds`
                // entry yields a `NativeKind` (e.g. an aggregate/object
                // capture). §2.7.8 #4: surface-and-stop, never Bool-default.
                return Err(format!(
                    "SURFACE (ADR-006 §2.7.8 / Q10): SharedCell for local slot {slot} has no \
                     derivable NativeKind companion. Neither the ClosureLayout's capture_types \
                     nor the inferred slot_kinds entry projects to a NativeKind, and \
                     Bool-defaulting a cell's kind companion is forbidden — SharedCell::drop \
                     dispatches its Arc-retire matrix on this field. Whole-function JIT bail; \
                     the interpreter's op_alloc_shared_local takes the kind off the §2.7.7 \
                     parallel-kind stack track and runs this correctly."
                ));
            };
            if kind.is_refcounted() {
                // (b) Heap payload. Stamping a refcounted kind arms
                // `SharedCell::drop`'s Arc-retire arm, but the JIT's
                // cell-write lowering (`places.rs` `write_place` /
                // `read_place` shared-local arms) stores the payload as
                // raw i64 bits with NO retain of the new value and NO
                // release of the previous one, and the cell is SEEDED with
                // `TAG_NULL` (which is NONZERO, so `Drop`'s `bits == 0`
                // early-return does not fire). A heap-kinded cell would
                // therefore either release a share it never took
                // (double-free) or release TAG_NULL as a pointer
                // (segfault). Scalar kinds are safe precisely because
                // their Drop arm is a no-op regardless of the seed bits.
                //
                // The honest fix for heap-payload `var` captures is to
                // make the cell store path refcount-correct FIRST; until
                // then this is a clean deopt, not a silent memory bug.
                return Err(format!(
                    "SURFACE (ADR-006 §2.7.8 / Q10): SharedCell for local slot {slot} has a \
                     REFCOUNTED payload kind ({kind:?}). The JIT's shared-cell store lowering \
                     writes raw bits with no retain and no release-of-previous, and the cell is \
                     seeded with a nonzero TAG_NULL — so stamping a heap kind would arm \
                     SharedCell::drop to retire a share the cell never owned. Whole-function \
                     JIT bail until the cell store path is refcount-correct."
                ));
            }
        }

        for (slot, kind) in slots {
            let Some(&var) = self.locals.get(&slot) else {
                continue;
            };
            // SAFETY of the unwrap: the gate above returned Err for every
            // `None` kind before a single cell was emitted.
            let kind = kind.expect("shared-local kind gate ran before emission");
            // NONE bits — matches the interpreter's pre-AllocSharedLocal
            // slot state. The raw bits come from the value-ffi `TAG_NULL`
            // constant (the canonical None encoding at the FFI boundary).
            //
            // The cell's `NativeKind` companion is the slot's INNER payload
            // kind (ADR-006 §2.7.8 / Q10), stamped here from the producer
            // (ClosureLayout capture_types / inferred slot_kinds) — the same
            // kind the interpreter reads off the §2.7.7 parallel-kind stack
            // track in `op_alloc_shared_local`. The gate above guarantees it
            // is an inline-scalar kind, whose `Drop` arm is a no-op, so the
            // TAG_NULL seed is never interpreted as a pointer.
            let init = self
                .builder
                .ins()
                .iconst(types::I64, crate::ffi::value_ffi::TAG_NULL as i64);
            let kind_code = crate::ffi::stack_kind_code::encode(kind);
            let kind_code_val = self.builder.ins().iconst(types::I8, kind_code as i64);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.alloc_shared_cell, &[init, kind_code_val]);
            let cell_ptr = self.builder.inst_results(inst)[0];
            self.builder.def_var(var, cell_ptr);
        }
        Ok(())
    }
}
