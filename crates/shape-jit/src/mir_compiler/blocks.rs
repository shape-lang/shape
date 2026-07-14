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
}
