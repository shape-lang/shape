//! MIR BasicBlock → Cranelift Block mapping.

use cranelift::prelude::*;

use super::MirToIR;
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
    /// ADR-020 §3.3: unit slots get no variable. A void call's destination
    /// holds no value, so there is no width to declare it at — declaring
    /// one and initializing it to zero is the unit sentinel §6 forbids,
    /// just spelled as a Cranelift variable. `read_place` / `write_place`
    /// on such a slot fail with "unknown local slot", which is the
    /// surface-and-stop we want: it means codegen tried to move a value
    /// that was never produced.
    pub(crate) fn declare_locals(&mut self) -> Result<(), String> {
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            if self.unit_slots.contains(&slot_id) {
                continue;
            }
            // #236 / R-G7: an unproven kind gets NO variable rather than a
            // fabricated `I64` one. Declaring is not the right place to
            // surface — `num_locals` covers slots no instruction ever
            // touches (a top-level program's return slot when the trailing
            // statement produces nothing), and those have no soundness
            // content to get wrong. Skipping leaves `self.locals` without
            // an entry, so the FIRST codegen site that actually reads or
            // writes the slot surfaces through `local_var`, and a slot
            // nothing touches costs nothing. `Return` already treats an
            // undeclared `SlotId(0)` as the unit return.
            let Ok(kind) = self.local_storage_kind(slot_id) else {
                continue;
            };
            let cl_type = slot_types::cranelift_type_for_slot(kind);

            let var = Variable::new(self.next_var);
            self.next_var += 1;
            self.builder.declare_var(var, cl_type);
            self.locals.insert(slot_id, var);
        }
        Ok(())
    }

    /// `SHAPE_DEBUG_SLOT_KINDS=1` — dump the MIR and the slot-kind vector at
    /// the point a kind proof was demanded and missing. This is what located
    /// the ADR-020 §3.3 unit-temp chain behind the 11/488 native rate; the
    /// bail message alone names a slot number, which is not enough to see
    /// which MIR shape failed to stamp it. Mirrors `SHAPE_DEBUG_FIELD_STAMPS`.
    pub(crate) fn debug_dump_slot_kinds(&self, slot_id: shape_vm::mir::types::SlotId) {
        if std::env::var_os("SHAPE_DEBUG_SLOT_KINDS").is_none() {
            return;
        }
        eprintln!(
            "[slot-kinds] fn={} unproven_slot={} kinds={:?} unit={:?}",
            self.mir.name, slot_id.0, self.slot_kinds, self.unit_slots
        );
        for b in &self.mir.blocks {
            for s in &b.statements {
                eprintln!("[slot-kinds]   stmt {:?}", s.kind);
            }
            eprintln!("[slot-kinds]   term {:?}", b.terminator.kind);
        }
    }

    /// The Cranelift variable backing a MIR local, or a surface-and-stop
    /// describing WHY there isn't one.
    ///
    /// Two undeclared cases, and they are different diagnoses: the slot
    /// carries unit (ADR-020 §3.3 — codegen is trying to move a value that
    /// was never produced), or its kind was never proven (#236 — there was
    /// no sound width to declare it at). Both are bails; conflating them
    /// under "unknown local slot" is what made the #236 surface read as a
    /// codegen bug.
    pub(crate) fn local_var(
        &self,
        slot_id: shape_vm::mir::types::SlotId,
    ) -> Result<Variable, String> {
        if let Some(&var) = self.locals.get(&slot_id) {
            return Ok(var);
        }
        self.debug_dump_slot_kinds(slot_id);
        if self.unit_slots.contains(&slot_id) {
            return Err(format!(
                "ADR-020 §3.3 surface-and-stop: SURFACE — local slot {} carries UNIT (it is a \
                 void call's destination, or a move chain from one), so no value was ever \
                 produced for it and it has no storage. Codegen reached a site that wants to \
                 read or write it.",
                slot_id.0
            ));
        }
        Err(format!(
            "MirToIR: SURFACE — local slot {} has no compile-time-proven NativeKind, so there \
             is no sound Cranelift storage width to declare it at and it was left undeclared. \
             The producing site must stamp the slot kind (ADR-006 §2.7.5); the \
             `unwrap_or(Int64)` that used to stand here declared floats and heap pointers as \
             integers (#236).",
            slot_id.0
        ))
    }

    /// Initialize all local variables to their type-appropriate zero/null.
    /// Must be called AFTER switching to the entry block.
    pub(crate) fn initialize_locals(&mut self) -> Result<(), String> {
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            if !self.locals.contains_key(&slot_id) {
                continue;
            }
            let kind = self.local_storage_kind(slot_id)?;
            if let Some(&var) = self.locals.get(&slot_id) {
                let init_val = self.default_value_for_kind(kind);
                self.builder.def_var(var, init_val);
            }
        }
        Ok(())
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
    pub(crate) fn local_storage_kind(
        &self,
        slot_id: shape_vm::mir::types::SlotId,
    ) -> Result<NativeKind, String> {
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
            return Ok(NativeKind::Int64);
        }

        // #236 / R-G7: no fabricated default. A slot with no proven kind has
        // no sound storage width, and picking I64 is how a `Float64` or a
        // heap pointer ends up declared as an integer. Unit slots never
        // reach here — `declare_locals` skips them (ADR-020 §3.3).
        slot_types::slot_kind_for_local(&self.slot_kinds, slot_id.0).ok_or_else(|| {
            format!(
                "MirToIR: SURFACE — local slot {} has no compile-time-proven NativeKind, so \
                 there is no sound Cranelift storage width to declare it at. The producing \
                 site must stamp the slot kind (ADR-006 §2.7.5); the `unwrap_or(Int64)` that \
                 used to stand here declared floats and heap pointers as integers (#236).",
                slot_id.0
            )
        })
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
}
