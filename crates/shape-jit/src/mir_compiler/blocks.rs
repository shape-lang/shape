//! MIR BasicBlock → Cranelift Block mapping.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::type_tracking::SlotKind;

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
    /// Variables are declared with their native Cranelift type per SlotKind:
    /// - Float64 → F64
    /// - Int32/UInt32 → I32
    /// - Bool/Int8/UInt8 → I8
    /// - Unknown/NanBoxed/Int64/String/etc → I64 (NaN-boxed)
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

    /// Produce the default (zero/null) value for a given SlotKind.
    fn default_value_for_kind(&mut self, kind: SlotKind) -> Value {
        match kind {
            SlotKind::Float64 => self.builder.ins().f64const(0.0),
            SlotKind::Int32 | SlotKind::UInt32 => {
                self.builder.ins().iconst(types::I32, 0)
            }
            SlotKind::Int8 | SlotKind::UInt8 | SlotKind::Bool => {
                self.builder.ins().iconst(types::I8, 0)
            }
            SlotKind::Int16 | SlotKind::UInt16 => {
                self.builder.ins().iconst(types::I16, 0)
            }
            // v2-boundary: I64 NaN-boxed slots use TAG_NULL as default
            _ => self
                .builder
                .ins()
                .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64),
        }
    }
}
