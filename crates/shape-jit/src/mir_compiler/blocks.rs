//! MIR BasicBlock → Cranelift Block mapping.

use cranelift::prelude::*;

use super::MirToIR;

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
    /// Variables are declared but NOT initialized here — initialization
    /// happens in initialize_locals() after switching to the entry block.
    pub(crate) fn declare_locals(&mut self) {
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            let var = Variable::new(self.next_var);
            self.next_var += 1;
            self.builder.declare_var(var, types::I64);
            self.locals.insert(slot_id, var);
        }
    }

    /// Initialize all local variables to TAG_NULL.
    /// Must be called AFTER switching to the entry block.
    pub(crate) fn initialize_locals(&mut self) {
        let null_val = self
            .builder
            .ins()
            .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
        for slot_idx in 0..self.mir.num_locals {
            let slot_id = shape_vm::mir::types::SlotId(slot_idx);
            if let Some(&var) = self.locals.get(&slot_id) {
                self.builder.def_var(var, null_val);
            }
        }
    }
}
