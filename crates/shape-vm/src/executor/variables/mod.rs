//! Variable operations for the VM executor
//!
//! Handles: LoadLocal, StoreLocal, LoadModuleBinding, StoreModuleBinding, LoadClosure, StoreClosure, CloseUpvalue

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
    memory::{record_heap_write, write_barrier_vw},
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::{Arc, RwLock};
impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_variables(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            LoadLocal => self.op_load_local(instruction)?,
            LoadLocalTrusted => self.op_load_local_trusted(instruction)?,
            StoreLocal => self.op_store_local(instruction)?,
            StoreLocalTyped => self.op_store_local_typed(instruction)?,
            LoadModuleBinding => self.op_load_module_binding(instruction)?,
            StoreModuleBinding => self.op_store_module_binding(instruction)?,
            StoreModuleBindingTyped => self.op_store_module_binding_typed(instruction)?,
            LoadClosure => self.op_load_closure(instruction)?,
            StoreClosure => self.op_store_closure(instruction)?,
            CloseUpvalue => self.op_close_upvalue(instruction)?,
            MakeRef => self.op_make_ref(instruction)?,
            DerefLoad => self.op_deref_load(instruction)?,
            DerefStore => self.op_deref_store(instruction)?,
            SetIndexRef => self.op_set_index_ref(instruction)?,
            BoxLocal => self.op_box_local(instruction)?,
            BoxModuleBinding => self.op_box_module_binding(instruction)?,
            _ => unreachable!(
                "exec_variables called with non-variable opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Load value from an upvalue in the current closure's environment
    fn op_load_closure(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        if let Some(Operand::Local(upvalue_idx)) = instruction.operand {
            // Get the current call frame's upvalues
            if let Some(frame) = self.call_stack.last() {
                if let Some(upvalues) = &frame.upvalues {
                    if let Some(upvalue) = upvalues.get(upvalue_idx as usize) {
                        let value_nb = upvalue.get();
                        self.push_vw(value_nb)?;
                        return Ok(());
                    }
                }
            }
            Err(VMError::RuntimeError(format!(
                "Upvalue index {} not found in closure",
                upvalue_idx
            )))
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Store value to an upvalue in the current closure's environment.
    ///
    /// If the upvalue is `Immutable`, it is upgraded to `Mutable` on the first write.
    fn op_store_closure(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        if let Some(Operand::Local(upvalue_idx)) = instruction.operand {
            let value_nb = self.pop_vw()?;
            // Get the current call frame's upvalues (mutable for potential upgrade)
            if let Some(frame) = self.call_stack.last_mut() {
                if let Some(upvalues) = &mut frame.upvalues {
                    if let Some(upvalue) = upvalues.get_mut(upvalue_idx as usize) {
                        record_heap_write();
                        write_barrier_vw(&upvalue.get(), &value_nb);
                        upvalue.set(value_nb);
                        return Ok(());
                    }
                }
            }
            Err(VMError::RuntimeError(format!(
                "Upvalue index {} not found in closure",
                upvalue_idx
            )))
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Close upvalue - currently a no-op since we already capture by value
    /// In a full implementation, this would move the value from stack to heap
    fn op_close_upvalue(&mut self, _instruction: &Instruction) -> Result<(), VMError> {
        // With Arc<RwLock<ValueWord>> upvalues, closing is automatic
        // The value is already on the heap and shared
        Ok(())
    }

    /// Load value from a local variable slot (register window on the unified stack).
    ///
    /// Optimized: reads the raw u64 bits directly via pointer to skip bounds
    /// checks and Option wrapping. For inline values (numbers, ints, bools —
    /// the common case), constructs a ValueWord by bit-copying without going
    /// through clone dispatch. Only heap-tagged values take the clone path.
    ///
    /// If the slot contains a SharedCell (boxed local for mutable closure capture),
    /// the inner value is read through the Arc transparently.
    pub(in crate::executor) fn op_load_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "LoadLocal slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            // SAFETY: The compiler ensures local slots are within the frame's register
            // window which is pre-allocated on the stack. We read the raw bits and
            // only call clone for heap-tagged values (which own a Box<HeapValue>).
            let nb = unsafe {
                let bits = *(self.stack.as_ptr().add(slot) as *const u64);
                ValueWord::clone_from_bits(bits)
            };
            // Auto-deref SharedCell: read the inner value through the Arc
            if let Some(HeapValue::SharedCell(arc)) = nb.as_heap_ref() {
                let inner = arc.read().unwrap().clone();
                self.push_vw(inner)?;
            } else {
                self.push_vw(nb)?;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Load value from a local variable slot — trusted variant.
    ///
    /// The compiler has proved that this slot has a known SlotKind in the
    /// FrameDescriptor. Skips SharedCell auto-deref and tag validation.
    #[inline(always)]
    pub(in crate::executor) fn op_load_local_trusted(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "LoadLocalTrusted slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            // SAFETY: Compiler proved the slot has a known type. Skip SharedCell
            // check and read the raw bits directly.
            let nb = unsafe {
                let bits = *(self.stack.as_ptr().add(slot) as *const u64);
                ValueWord::clone_from_bits(bits)
            };
            self.push_vw(nb)?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Store value to a local variable slot (register window on the unified stack).
    ///
    /// If the slot contains a SharedCell (boxed local for mutable closure capture),
    /// the value is written through the Arc so all holders see the update.
    pub(in crate::executor) fn op_store_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let nb = self.pop_vw()?;
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;

            // Ensure stack is large enough (should already be, but safety check)
            if slot >= self.stack.len() {
                self.stack.resize_with(slot + 1, ValueWord::none);
            }

            // Auto-deref SharedCell: write through the Arc
            if let Some(HeapValue::SharedCell(arc)) = self.stack[slot].as_heap_ref() {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &nb);
                *arc.write().unwrap() = nb;
            } else {
                record_heap_write();
                write_barrier_vw(&self.stack[slot], &nb);
                self.stack[slot] = nb;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Store a local with integer width truncation.
    /// Operand: TypedLocal(idx, width)
    fn op_store_local_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        if let Some(Operand::TypedLocal(idx, width)) = instruction.operand {
            let nb = self.pop_vw()?;
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;

            if slot >= self.stack.len() {
                self.stack.resize_with(slot + 1, ValueWord::none);
            }

            // Truncate the value to the declared width
            let truncated = if let Some(int_w) = width.to_int_width() {
                let raw = Self::int_operand(&nb).unwrap_or(0);
                ValueWord::from_i64(int_w.truncate(raw))
            } else {
                // I64 or float width: no truncation
                nb
            };

            if let Some(HeapValue::SharedCell(arc)) = self.stack[slot].as_heap_ref() {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &truncated);
                *arc.write().unwrap() = truncated;
            } else {
                record_heap_write();
                write_barrier_vw(&self.stack[slot], &truncated);
                self.stack[slot] = truncated;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Load value from a module_binding variable slot.
    ///
    /// If the slot contains a SharedCell, the inner value is read through the Arc.
    pub(in crate::executor) fn op_load_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::ModuleBinding(idx)) = instruction.operand {
            let nb = self
                .module_bindings
                .get(idx as usize)
                .cloned()
                .unwrap_or_else(ValueWord::none);
            // Auto-deref SharedCell
            if let Some(HeapValue::SharedCell(arc)) = nb.as_heap_ref() {
                let inner = arc.read().unwrap().clone();
                self.push_vw(inner)?;
            } else {
                self.push_vw(nb)?;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// MakeRef: Push a TAG_REF value pointing to a local variable's absolute stack slot.
    ///
    /// The operand is a local slot index. The absolute stack address is computed
    /// as `base_pointer + local_idx` and packed into a ValueWord TAG_REF value.
    pub(in crate::executor) fn op_make_ref(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let absolute_slot = bp + idx as usize;
            self.push_vw(ValueWord::from_ref(absolute_slot))?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// DerefLoad: Follow a reference stored in a local slot and push the target value.
    ///
    /// The operand is the local slot holding the TAG_REF value. We extract the
    /// absolute stack index from the ref, then clone the value at that location.
    pub(in crate::executor) fn op_deref_load(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(ref_slot)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + ref_slot as usize;
            let ref_val = &self.stack[slot];
            let target = ref_val.as_ref_slot().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;
            // Clone the value at the target absolute slot
            let nb = unsafe {
                let bits = *(self.stack.as_ptr().add(target) as *const u64);
                ValueWord::clone_from_bits(bits)
            };
            self.push_vw(nb)?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// DerefStore: Pop a value and write it through a reference stored in a local slot.
    ///
    /// The operand is the local slot holding the TAG_REF value. We extract the
    /// absolute stack index from the ref, then overwrite the value at that location.
    pub(in crate::executor) fn op_deref_store(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(ref_slot)) = instruction.operand {
            let value = self.pop_vw()?;
            let bp = self.current_locals_base();
            let slot = bp + ref_slot as usize;
            let target = self.stack[slot].as_ref_slot().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;
            record_heap_write();
            write_barrier_vw(&self.stack[target], &value);
            self.stack[target] = value;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// SetIndexRef: Mutate an array element in-place through a reference.
    ///
    /// Stack: [index, value] (value on top)
    /// Operand: local slot holding the TAG_REF
    ///
    /// Follows the ref to the target slot, then delegates to
    /// `set_array_index_on_object` which handles CoW via Arc::make_mut.
    /// The borrow checker guarantees exclusive access at compile time.
    pub(in crate::executor) fn op_set_index_ref(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(ref_slot)) = instruction.operand {
            let value = self.pop_vw()?;
            let index_nb = self.pop_vw()?;
            let bp = self.current_locals_base();
            let slot = bp + ref_slot as usize;
            let target = self.stack[slot].as_ref_slot().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;

            // Take the object out of the target slot, mutate it, put it back
            // (same pattern as op_set_local_index)
            let mut object_nb = std::mem::replace(&mut self.stack[target], ValueWord::none());
            let result = Self::set_array_index_on_object(&mut object_nb, &index_nb, value);
            record_heap_write();
            write_barrier_vw(&ValueWord::none(), &object_nb);
            self.stack[target] = object_nb;
            result
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Store value to a module_binding variable slot.
    ///
    /// If the slot contains a SharedCell, the value is written through the Arc.
    pub(in crate::executor) fn op_store_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::ModuleBinding(idx)) = instruction.operand {
            let nb = self.pop_vw()?;
            let index = idx as usize;

            // Ensure module_bindings vector is large enough
            while self.module_bindings.len() <= index {
                self.module_bindings.push(ValueWord::none());
            }

            // Auto-deref SharedCell: write through the Arc
            if let Some(HeapValue::SharedCell(arc)) = self.module_bindings[index].as_heap_ref() {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &nb);
                *arc.write().unwrap() = nb;
            } else {
                record_heap_write();
                write_barrier_vw(&self.module_bindings[index], &nb);
                self.module_bindings[index] = nb;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Store value to a module_binding variable slot with integer width truncation.
    ///
    /// Operand: TypedModuleBinding(idx, width)
    pub(in crate::executor) fn op_store_module_binding_typed(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::TypedModuleBinding(idx, width)) = instruction.operand {
            let nb = self.pop_vw()?;
            let index = idx as usize;

            // Truncate the value to the declared width
            let truncated = if let Some(int_w) = width.to_int_width() {
                let raw = Self::int_operand(&nb).unwrap_or(0);
                ValueWord::from_i64(int_w.truncate(raw))
            } else {
                nb
            };

            // Ensure module_bindings vector is large enough
            while self.module_bindings.len() <= index {
                self.module_bindings.push(ValueWord::none());
            }

            // Auto-deref SharedCell: write through the Arc
            if let Some(HeapValue::SharedCell(arc)) = self.module_bindings[index].as_heap_ref() {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &truncated);
                *arc.write().unwrap() = truncated;
            } else {
                record_heap_write();
                write_barrier_vw(&self.module_bindings[index], &truncated);
                self.module_bindings[index] = truncated;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Box a local variable into a SharedCell for mutable closure capture.
    ///
    /// If the slot doesn't already contain a SharedCell, wraps its value in one.
    /// Then pushes the SharedCell ValueWord onto the stack for MakeClosure to consume.
    /// This establishes a shared mutable cell between the enclosing scope and the closure.
    pub(in crate::executor) fn op_box_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;

            // If not already a SharedCell, wrap the value in one
            let is_cell = self.stack[slot]
                .as_heap_ref()
                .map(|hv| matches!(hv, HeapValue::SharedCell(_)))
                .unwrap_or(false);

            if !is_cell {
                let old = self.stack[slot].clone();
                let value = std::mem::replace(&mut self.stack[slot], ValueWord::none());
                let cell_vw =
                    ValueWord::from_heap_value(HeapValue::SharedCell(Arc::new(RwLock::new(value))));
                record_heap_write();
                write_barrier_vw(&old, &cell_vw);
                self.stack[slot] = cell_vw;
            }

            // Push the SharedCell onto the stack for MakeClosure to consume
            let nb = self.stack[slot].clone();
            self.push_vw(nb)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Box a module binding into a SharedCell for mutable closure capture.
    ///
    /// Same as op_box_local but operates on the module_bindings vector.
    pub(in crate::executor) fn op_box_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::ModuleBinding(idx)) = instruction.operand {
            let index = idx as usize;

            // Ensure module_bindings vector is large enough
            while self.module_bindings.len() <= index {
                self.module_bindings.push(ValueWord::none());
            }

            // If not already a SharedCell, wrap the value in one
            let is_cell = self.module_bindings[index]
                .as_heap_ref()
                .map(|hv| matches!(hv, HeapValue::SharedCell(_)))
                .unwrap_or(false);

            if !is_cell {
                let old = self.module_bindings[index].clone();
                let value = std::mem::replace(&mut self.module_bindings[index], ValueWord::none());
                let cell_vw =
                    ValueWord::from_heap_value(HeapValue::SharedCell(Arc::new(RwLock::new(value))));
                record_heap_write();
                write_barrier_vw(&old, &cell_vw);
                self.module_bindings[index] = cell_vw;
            }

            // Push the SharedCell onto the stack for MakeClosure to consume
            let nb = self.module_bindings[index].clone();
            self.push_vw(nb)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }
}
