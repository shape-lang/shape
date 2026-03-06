//! Reference opcode compilation: MakeRef, DerefLoad, DerefStore, SetIndexRef
//!
//! These opcodes implement explicit references (&) in Shape, enabling
//! in-place mutation of arrays and other compound values through pointers.
//!
//! In the JIT, references are implemented as raw pointers to Cranelift stack
//! slots. Using stack slots (instead of ctx.locals[]) ensures the reference
//! target survives function calls, which save/restore ctx.locals[0..arg_count].
//!
//! The borrow checker guarantees exclusive access at compile time,
//! so no runtime synchronization is needed.

use cranelift::prelude::*;

use crate::translator::storage::CraneliftRepr;
use shape_vm::bytecode::{Instruction, Operand};

use crate::translator::types::BytecodeToIR;

// Temporarily disabled by default until dense-bool fast paths are retuned.
const ENABLE_BOOL_DENSE_ARRAY_PATH: bool = false;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// MakeRef(Local(idx)): Create a stable reference to a local variable's value.
    ///
    /// VM semantics: pushes a TAG_REF value containing the absolute slot.
    /// JIT implementation: allocates a Cranelift stack slot, stores the current
    /// value of local `idx` into it, and pushes the slot's address. The stack
    /// slot lives in the native stack frame and is NOT affected by ctx.locals
    /// save/restore during function calls.
    pub(crate) fn compile_make_ref(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Local(idx)) = &instr.operand {
            // Get current SSA value of the local
            let var = self.get_or_create_local(*idx);
            let current_val = self.builder.use_var(var);

            // Allocate a Cranelift stack slot (8 bytes, aligned to 8)
            // This lives in the native function's stack frame, NOT in ctx.locals
            let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                8,
                3, // alignment = 2^3 = 8
            ));

            // Store the value to the stack slot
            self.builder.ins().stack_store(current_val, slot, 0);

            // Track this slot so we can reload the local after function calls
            // that may have modified the value through the reference
            self.ref_stack_slots.insert(*idx, slot);

            // Get the address of the stack slot and push it
            let addr = self.builder.ins().stack_addr(types::I64, slot, 0);
            self.stack_push(addr);
        }
        Ok(())
    }

    /// After a function call, reload all referenced locals from their stack slots.
    /// Called functions may have modified values through references, so the SSA
    /// variables need to be updated to reflect the current stack slot contents.
    pub(crate) fn reload_referenced_locals(&mut self) {
        let refs: Vec<_> = self.ref_stack_slots.iter().map(|(&k, &v)| (k, v)).collect();
        for (local_idx, slot) in refs {
            let reloaded = self.builder.ins().stack_load(types::I64, slot, 0);
            let var = self.get_or_create_local(local_idx);
            self.builder.def_var(var, reloaded);
        }
    }

    /// DerefLoad(Local(ref_slot)): Follow a reference and push the target value.
    ///
    /// VM semantics: reads reference from local, follows to target slot, pushes value.
    /// JIT implementation: loads the pointer from the local variable, then does a
    /// memory load at that address to get the NaN-boxed value.
    pub(crate) fn compile_deref_load(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Local(ref_slot)) = &instr.operand {
            // Load the reference pointer from the local variable
            let var = self.get_or_create_local(*ref_slot);
            let ref_addr = self.builder.use_var(var);

            // Dereference: load the NaN-boxed value at the pointer address
            let value = self
                .builder
                .ins()
                .load(types::I64, MemFlags::new(), ref_addr, 0);
            self.stack_push(value);
            if let Some(&(data_ptr, length)) = self.hoisted_ref_array_info.get(ref_slot) {
                let tv = crate::translator::storage::TypedValue::boxed(value)
                    .with_hoisted_array_info(data_ptr, length);
                self.typed_stack.replace_top(tv);
            }
        }
        Ok(())
    }

    /// DerefStore(Local(ref_slot)): Pop a value and write it through a reference.
    ///
    /// VM semantics: pops value, reads reference from local, stores value at target.
    /// JIT implementation: pops value from stack, loads the pointer from the local
    /// variable, then stores the value at the pointer address.
    pub(crate) fn compile_deref_store(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Local(ref_slot)) = &instr.operand {
            if let Some(value) = self.stack_pop() {
                // Load the reference pointer from the local variable
                let var = self.get_or_create_local(*ref_slot);
                let ref_addr = self.builder.use_var(var);

                // Write through reference: store value at the pointer address
                self.builder
                    .ins()
                    .store(MemFlags::new(), value, ref_addr, 0);
            }
        }
        Ok(())
    }

    /// SetIndexRef(Local(ref_slot)): Mutate array[index] through a reference.
    ///
    /// VM semantics: pops value and index, reads reference, does array[index] = value.
    /// JIT implementation: strict typed lowering with direct memory updates.
    /// No runtime type-check fallback is emitted here.
    pub(crate) fn compile_set_index_ref(&mut self, instr: &Instruction) -> Result<(), String> {
        if let Some(Operand::Local(ref_slot)) = &instr.operand {
            if self.stack_len() >= 2 {
                // Value stored to array must be NaN-boxed
                let value = self.stack_pop_boxed().unwrap();
                let index_hint = self.peek_stack_type();
                let index_is_raw_i64 = self
                    .typed_stack
                    .peek()
                    .map(|tv| tv.repr == CraneliftRepr::I64)
                    .unwrap_or(false);
                let index_is_typed_int = !index_is_raw_i64 && index_hint.is_integer_family();
                let index = self.stack_pop().unwrap();
                let trusted_set = self
                    .optimization_plan
                    .trusted_array_set_indices
                    .contains(&self.current_instr_idx);
                let non_negative_set = self
                    .optimization_plan
                    .non_negative_array_set_indices
                    .contains(&self.current_instr_idx);
                let planned_bool_set = self
                    .optimization_plan
                    .numeric_arrays
                    .bool_set_sites
                    .contains(&self.current_instr_idx)
                    && ENABLE_BOOL_DENSE_ARRAY_PATH;

                // Standard path source value: deref the reference local.
                let var = self.get_or_create_local(*ref_slot);
                let ref_addr = self.builder.use_var(var);
                let array = self
                    .builder
                    .ins()
                    .load(types::I64, MemFlags::new(), ref_addr, 0);

                // Array LICM: check if ref_slot has hoisted (data_ptr, length).
                // If so, skip deref + array extraction entirely.
                if let Some(&(data_ptr, length)) = self.hoisted_ref_array_info.get(ref_slot) {
                    let idx_i64 = if index_is_raw_i64 {
                        index
                    } else {
                        let idx_f64 = self.i64_to_f64(index);
                        self.builder.ins().fcvt_to_sint_sat(types::I64, idx_f64)
                    };
                    if planned_bool_set {
                        if trusted_set {
                            self.inline_array_set_hoisted_i64_trusted_bool(
                                array, idx_i64, data_ptr, length, value,
                            );
                        } else if non_negative_set && (index_is_raw_i64 || index_is_typed_int) {
                            self.inline_array_set_hoisted_i64_non_negative_bool(
                                array, idx_i64, data_ptr, length, value,
                            );
                        } else {
                            self.inline_array_set_hoisted_i64_bool(
                                array, idx_i64, data_ptr, length, value,
                            );
                        }
                    } else if trusted_set {
                        self.inline_array_set_hoisted_i64_trusted(
                            array, idx_i64, data_ptr, length, value,
                        );
                    } else if non_negative_set && (index_is_raw_i64 || index_is_typed_int) {
                        self.inline_array_set_hoisted_i64_non_negative(
                            array, idx_i64, data_ptr, length, value,
                        );
                    } else {
                        self.inline_array_set_hoisted_i64(array, idx_i64, data_ptr, length, value);
                    }
                    return Ok(());
                }

                if index_is_raw_i64 {
                    if planned_bool_set {
                        if trusted_set {
                            self.inline_array_set_i64_trusted_bool(array, index, value);
                        } else if non_negative_set {
                            self.inline_array_set_i64_non_negative_bool(array, index, value);
                        } else {
                            self.inline_array_set_i64_bool(array, index, value);
                        }
                    } else if trusted_set {
                        self.inline_array_set_i64_trusted(array, index, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative(array, index, value);
                    } else {
                        self.inline_array_set_i64(array, index, value);
                    }
                } else if index_is_typed_int {
                    let idx_f64 = self.i64_to_f64(index);
                    let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, idx_f64);
                    if planned_bool_set {
                        if trusted_set {
                            self.inline_array_set_i64_trusted_bool(array, idx_i64, value);
                        } else if non_negative_set {
                            self.inline_array_set_i64_non_negative_bool(array, idx_i64, value);
                        } else {
                            self.inline_array_set_i64_bool(array, idx_i64, value);
                        }
                    } else if trusted_set {
                        self.inline_array_set_i64_trusted(array, idx_i64, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative(array, idx_i64, value);
                    } else {
                        self.inline_array_set_i64(array, idx_i64, value);
                    }
                } else if planned_bool_set {
                    let idx_f64 = self.i64_to_f64(index);
                    let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, idx_f64);
                    if trusted_set {
                        self.inline_array_set_i64_trusted_bool(array, idx_i64, value);
                    } else if non_negative_set {
                        self.inline_array_set_i64_non_negative_bool(array, idx_i64, value);
                    } else {
                        self.inline_array_set_i64_bool(array, idx_i64, value);
                    }
                } else {
                    self.inline_array_set(array, index, value);
                }
            }
        }
        Ok(())
    }
}
