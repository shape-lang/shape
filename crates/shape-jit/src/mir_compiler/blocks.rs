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
    /// Variables are declared with their native Cranelift type per NativeKind:
    /// - Float64 → F64
    /// - Int32/UInt32 → I32
    /// - Bool/Int8/UInt8 → I8
    /// - Unknown/Dynamic/Int64/String/etc → I64 (dynamic)
    ///
    /// Variables are declared but NOT initialized here — initialization
    /// happens in initialize_locals() after switching to the entry block.
    pub(crate) fn declare_locals(&mut self) {
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            let kind = slot_types::slot_kind_for_local(&self.slot_kinds, slot_idx);
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
            let kind = slot_types::slot_kind_for_local(&self.slot_kinds, slot_idx);

            if let Some(&var) = self.locals.get(&slot_id) {
                let init_val = self.default_value_for_kind(kind);
                self.builder.def_var(var, init_val);
            }
        }
    }

    /// Produce the default (zero/null) value for a given NativeKind.
    fn default_value_for_kind(&mut self, kind: NativeKind) -> Value {
        match kind {
            NativeKind::Float64 => self.builder.ins().f64const(0.0),
            NativeKind::Int32 | NativeKind::UInt32 => {
                self.builder.ins().iconst(types::I32, 0)
            }
            NativeKind::Int8 | NativeKind::UInt8 | NativeKind::Bool => {
                self.builder.ins().iconst(types::I8, 0)
            }
            NativeKind::Int16 | NativeKind::UInt16 => {
                self.builder.ins().iconst(types::I16, 0)
            }
            // v2-boundary: I64 NaN-boxed slots use TAG_NULL as default
            _ => self
                .builder
                .ins()
                .iconst(types::I64, 0i64),
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
    pub(crate) fn initialize_shared_local_slots(&mut self) {
        if self.shared_local_slots.is_empty() {
            return;
        }
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            eprintln!(
                "[jit-init-shared] shared_local_slots.len()={} slots={:?}",
                self.shared_local_slots.len(),
                self.shared_local_slots.iter().copied().collect::<Vec<_>>()
            );
        }
        // Collect into a Vec to avoid borrowing self across the loop.
        let slots: Vec<_> = self.shared_local_slots.iter().copied().collect();
        for slot in slots {
            let Some(&var) = self.locals.get(&slot) else {
                continue;
            };
            // NONE_BITS — matches the interpreter's pre-AllocSharedLocal
            // slot state (legacy NaN-boxed null sentinel). Using a
            // well-known bit pattern avoids undefined bits in the cell.
            let none_bits = shape_value::tag_bits::TAG_BASE
                | (shape_value::tag_bits::TAG_NONE << shape_value::tag_bits::TAG_SHIFT);
            let init = self.builder.ins().iconst(types::I64, none_bits as i64);
            let inst = self
                .builder
                .ins()
                .call(self.ffi.alloc_shared_cell, &[init]);
            let cell_ptr = self.builder.inst_results(inst)[0];
            self.builder.def_var(var, cell_ptr);
        }
    }
}
