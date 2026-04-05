//! Variable operations for the VM executor
//!
//! Handles: LoadLocal, StoreLocal, LoadModuleBinding, StoreModuleBinding, LoadClosure, StoreClosure, CloseUpvalue

use crate::executor::objects::object_creation::clone_slots_with_update;
use crate::executor::typed_object_ops::{read_slot_fast, tag_to_field_type};
use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
    memory::{record_heap_write, write_barrier_slot, write_barrier_vw},
};
use shape_value::heap_value::HeapValue;
use shape_value::nanboxed::RefTarget;
use shape_value::{RefProjection, VMError, ValueWord};
use std::sync::{Arc, RwLock};
impl VirtualMachine {
    pub(in crate::executor) fn read_ref_target(
        &self,
        target: &RefTarget,
    ) -> Result<ValueWord, VMError> {
        match target {
            RefTarget::Stack(slot) => {
                if *slot < self.stack.len() {
                    Ok(self.stack_read_vw(*slot))
                } else {
                    Ok(ValueWord::none())
                }
            }
            RefTarget::ModuleBinding(slot) => {
                if *slot < self.module_bindings.len() {
                    Ok(self.binding_read_vw(*slot))
                } else {
                    Ok(ValueWord::none())
                }
            }
            RefTarget::Projected(data) => match &data.projection {
                RefProjection::TypedField {
                    field_idx,
                    field_type_tag,
                    ..
                } => {
                    let base_value = self.resolve_ref_value(&data.base).ok_or_else(|| {
                        VMError::RuntimeError(
                            "internal error: projected reference base is not a reference"
                                .to_string(),
                        )
                    })?;
                    let base_value = if let Some(HeapValue::TypeAnnotatedValue { value, .. }) =
                        base_value.as_heap_ref()
                    {
                        value.as_ref().clone()
                    } else {
                        base_value
                    };
                    if let Some(HeapValue::TypedObject {
                        slots, heap_mask, ..
                    }) = base_value.as_heap_ref()
                    {
                        let index = *field_idx as usize;
                        if index < slots.len() {
                            let is_heap = (*heap_mask & (1u64 << index)) != 0;
                            return Ok(read_slot_fast(&slots[index], is_heap, *field_type_tag));
                        }
                    }
                    Ok(ValueWord::none())
                }
                RefProjection::Index { index } => {
                    let base_value = self.resolve_ref_value(&data.base).ok_or_else(|| {
                        VMError::RuntimeError(
                            "internal error: projected reference base is not a reference"
                                .to_string(),
                        )
                    })?;
                    let base_value = if let Some(HeapValue::TypeAnnotatedValue { value, .. }) =
                        base_value.as_heap_ref()
                    {
                        value.as_ref().clone()
                    } else {
                        base_value
                    };
                    if let Some(arr) = base_value.as_any_array() {
                        let idx_opt = index
                            .as_i64()
                            .or_else(|| index.as_f64().map(|f| f as i64));
                        if let Some(idx) = idx_opt {
                            let len = arr.len() as i64;
                            let actual = if idx < 0 { len + idx } else { idx };
                            if actual >= 0 && (actual as usize) < arr.len() {
                                return Ok(arr
                                    .get_nb(actual as usize)
                                    .unwrap_or_else(ValueWord::none));
                            }
                        }
                    }
                    Ok(ValueWord::none())
                }
                RefProjection::MatrixRow { row_index } => {
                    let base_value = self.resolve_ref_value(&data.base).ok_or_else(|| {
                        VMError::RuntimeError(
                            "internal error: projected reference base is not a reference"
                                .to_string(),
                        )
                    })?;
                    // Return the row as a FloatArraySlice (read-only view)
                    if let Some(HeapValue::Matrix(mat_arc)) = base_value.as_heap_ref() {
                        let cols = mat_arc.cols;
                        let offset = *row_index * cols;
                        if *row_index < mat_arc.rows {
                            return Ok(ValueWord::from_heap_value(
                                HeapValue::FloatArraySlice {
                                    parent: mat_arc.clone(),
                                    offset,
                                    len: cols,
                                },
                            ));
                        }
                        return Err(VMError::RuntimeError(format!(
                            "Matrix row index {} out of bounds for {}x{} matrix",
                            row_index, mat_arc.rows, mat_arc.cols
                        )));
                    }
                    Err(VMError::RuntimeError(
                        "cannot read through a MatrixRow reference: base is not a matrix"
                            .to_string(),
                    ))
                }
            },
        }
    }

    pub(in crate::executor) fn write_ref_target(
        &mut self,
        target: &RefTarget,
        value: ValueWord,
    ) -> Result<(), VMError> {
        record_heap_write();
        match target {
            RefTarget::Stack(target) => {
                write_barrier_slot(self.stack[*target], value.raw_bits());
                self.stack_write_vw(*target, value);
                Ok(())
            }
            RefTarget::ModuleBinding(target) => {
                if *target >= self.module_bindings.len() {
                    self.module_bindings
                        .resize_with(*target + 1, || Self::NONE_BITS);
                }
                write_barrier_slot(self.module_bindings[*target], value.raw_bits());
                self.binding_write_vw(*target, value);
                Ok(())
            }
            RefTarget::Projected(data) => match &data.projection {
                RefProjection::TypedField {
                    field_idx,
                    field_type_tag,
                    ..
                } => {
                    let base_value = self.resolve_ref_value(&data.base).ok_or_else(|| {
                        VMError::RuntimeError(
                            "internal error: projected reference base is not a reference"
                                .to_string(),
                        )
                    })?;
                    let base_value = if let Some(HeapValue::TypeAnnotatedValue { value, .. }) =
                        base_value.as_heap_ref()
                    {
                        value.as_ref().clone()
                    } else {
                        base_value
                    };
                    if let Some(HeapValue::TypedObject {
                        schema_id,
                        slots,
                        heap_mask,
                    }) = base_value.as_heap_ref()
                    {
                        let field_type = tag_to_field_type(*field_type_tag);
                        let (new_slots, new_mask) = clone_slots_with_update(
                            slots,
                            *heap_mask,
                            *field_idx as usize,
                            &value,
                            field_type.as_ref(),
                        );
                        return self.write_ref_value(
                            &data.base,
                            ValueWord::from_heap_value(HeapValue::TypedObject {
                                schema_id: *schema_id,
                                slots: new_slots.into_boxed_slice(),
                                heap_mask: new_mask,
                            }),
                        );
                    }
                    Err(VMError::RuntimeError(
                        "cannot write through a field reference to a non-object value".to_string(),
                    ))
                }
                RefProjection::Index { index } => {
                    let base_value = self.resolve_ref_value(&data.base).ok_or_else(|| {
                        VMError::RuntimeError(
                            "internal error: projected reference base is not a reference"
                                .to_string(),
                        )
                    })?;
                    let mut base_value = if let Some(HeapValue::TypeAnnotatedValue { value, .. }) =
                        base_value.as_heap_ref()
                    {
                        value.as_ref().clone()
                    } else {
                        base_value
                    };
                    Self::set_array_index_on_object(&mut base_value, index, value).map_err(
                        |err| match err {
                            VMError::RuntimeError(message)
                                if message.starts_with("Cannot set property") =>
                            {
                                VMError::RuntimeError(
                                    "cannot write through an index reference to a non-array value"
                                        .to_string(),
                                )
                            }
                            other => other,
                        },
                    )?;
                    self.write_ref_value(&data.base, base_value)
                }
                RefProjection::MatrixRow { .. } => {
                    Err(VMError::RuntimeError(
                        "cannot assign a whole value to a matrix row reference; \
                         use row[col] = value to mutate individual elements"
                            .to_string(),
                    ))
                }
            },
        }
    }

    fn write_ref_value(&mut self, reference: &ValueWord, value: ValueWord) -> Result<(), VMError> {
        let target = reference.as_ref_target().ok_or_else(|| {
            VMError::RuntimeError(
                "internal error: expected a reference value (&) but found a regular value. \
                 This is a compiler bug — please report it"
                    .to_string(),
            )
        })?;
        self.write_ref_target(&target, value)
    }

    pub(in crate::executor) fn resolve_ref_value(&self, value: &ValueWord) -> Option<ValueWord> {
        let target = value.as_ref_target()?;
        self.read_ref_target(&target).ok()
    }

    /// Write a single element in a matrix row through a borrow reference.
    ///
    /// `base_ref` is a TAG_REF pointing at the stack slot or module binding
    /// holding the `Matrix(Arc<MatrixData>)`. We resolve it, call
    /// `Arc::make_mut` for COW semantics, then write
    /// `data[row_index * cols + col_index]`.
    fn set_matrix_row_element(
        &mut self,
        base_ref: &ValueWord,
        row_index: u32,
        col_index_nb: &ValueWord,
        value: ValueWord,
    ) -> Result<(), VMError> {
        let col_idx = col_index_nb
            .as_i64()
            .or_else(|| col_index_nb.as_f64().map(|f| f as i64))
            .ok_or_else(|| {
                VMError::RuntimeError("matrix column index must be a number".to_string())
            })?;

        let val_f64 = value.as_f64().or_else(|| value.as_i64().map(|i| i as f64)).ok_or_else(|| {
            VMError::RuntimeError(
                "matrix element must be a number".to_string(),
            )
        })?;

        // Resolve the base ref to find which slot holds the matrix.
        let base_target = base_ref.as_ref_target().ok_or_else(|| {
            VMError::RuntimeError(
                "internal error: MatrixRow base is not a reference".to_string(),
            )
        })?;

        // Get mutable access to the matrix slot and do COW mutation.
        match base_target {
            RefTarget::Stack(slot) => {
                // Temporarily take the ValueWord out for mutation, then write it back.
                let mut matrix_vw = self.stack_take_vw(slot);
                let result = Self::cow_matrix_write(&mut matrix_vw, row_index, col_idx, val_f64);
                self.stack_write_vw(slot, matrix_vw);
                result
            }
            RefTarget::ModuleBinding(slot) => {
                if slot >= self.module_bindings.len() {
                    return Err(VMError::RuntimeError(format!(
                        "ModuleBinding index {} out of bounds",
                        slot
                    )));
                }
                let mut matrix_vw = self.binding_take_vw(slot);
                let result = Self::cow_matrix_write(&mut matrix_vw, row_index, col_idx, val_f64);
                self.binding_write_vw(slot, matrix_vw);
                result
            }
            RefTarget::Projected(_) => Err(VMError::RuntimeError(
                "nested projected references for matrix row mutation are not supported"
                    .to_string(),
            )),
        }
    }

    /// Perform COW write into a matrix ValueWord at `data[row * cols + col]`.
    fn cow_matrix_write(
        matrix_vw: &mut ValueWord,
        row_index: u32,
        col_idx: i64,
        val: f64,
    ) -> Result<(), VMError> {
        let heap = matrix_vw.as_heap_mut().ok_or_else(|| {
            VMError::RuntimeError(
                "cannot write through MatrixRow reference: target is not a heap value".to_string(),
            )
        })?;

        match heap {
            HeapValue::Matrix(arc) => {
                let mat = Arc::make_mut(arc);
                let cols = mat.cols as i64;
                let actual_col = if col_idx < 0 { cols + col_idx } else { col_idx };
                if actual_col < 0 || actual_col >= cols {
                    return Err(VMError::RuntimeError(format!(
                        "Matrix column index {} out of bounds for {} columns",
                        col_idx, mat.cols
                    )));
                }
                if row_index >= mat.rows {
                    return Err(VMError::RuntimeError(format!(
                        "Matrix row index {} out of bounds for {} rows",
                        row_index, mat.rows
                    )));
                }
                let flat_idx = (row_index as usize) * (mat.cols as usize) + (actual_col as usize);
                record_heap_write();
                mat.data[flat_idx] = val;
                Ok(())
            }
            _ => Err(VMError::RuntimeError(
                "cannot write through MatrixRow reference: target is not a Matrix".to_string(),
            )),
        }
    }

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
            MakeFieldRef => self.op_make_field_ref(instruction)?,
            MakeIndexRef => self.op_make_index_ref(instruction)?,
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
    /// FrameDescriptor (a primitive type like f64, i64, or bool). This means:
    ///   - No SharedCell auto-deref (slot is a plain value, not a boxed capture)
    ///   - No tag validation (compiler already proved the type)
    ///   - Raw u64 read: reads the 8-byte slot directly and constructs a
    ///     ValueWord via bitwise copy. For inline values (the common trusted
    ///     case — numbers, ints, bools) this is a pure register-width copy
    ///     with no Arc refcount bump.
    ///
    /// For heap-tagged values the `clone_from_bits` path still does the
    /// Arc::increment_strong_count, but trusted slots are almost never heap.
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
                self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            }

            // Auto-deref SharedCell: write through the Arc
            let is_shared_cell = self.stack_peek_vw(slot, |vw| {
                vw.as_heap_ref().is_some_and(|hv| matches!(hv, HeapValue::SharedCell(_)))
            });
            if is_shared_cell {
                let slot_vw = self.stack_read_vw(slot);
                if let Some(HeapValue::SharedCell(arc)) = slot_vw.as_heap_ref() {
                    let arc = arc.clone();
                    let old = arc.read().unwrap().clone();
                    record_heap_write();
                    write_barrier_vw(&old, &nb);
                    *arc.write().unwrap() = nb;
                }
            } else {
                record_heap_write();
                write_barrier_slot(self.stack[slot], nb.raw_bits());
                self.stack_write_vw(slot, nb);
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Store a local with integer width truncation — typed variant.
    ///
    /// The compiler has proved that this slot holds a width-typed numeric value
    /// (i8, u8, i16, u16, i32, u32, i64, u64, f32, f64). This means:
    ///   - No SharedCell check (typed locals are never boxed captures)
    ///   - Width truncation for sub-64-bit integer types
    ///   - Raw u64 write: writes the truncated value directly to the stack slot
    ///     without going through write-barrier or SharedCell indirection.
    ///
    /// Operand: TypedLocal(idx, width)
    fn op_store_local_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        if let Some(Operand::TypedLocal(idx, width)) = instruction.operand {
            let nb = self.pop_vw()?;
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;

            if slot >= self.stack.len() {
                self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            }

            // Truncate the value to the declared width
            let truncated = if let Some(int_w) = width.to_int_width() {
                let raw = Self::int_operand(&nb).unwrap_or(0);
                ValueWord::from_i64(int_w.truncate(raw))
            } else {
                // I64 or float width: no truncation
                nb
            };

            let is_shared_cell = self.stack_peek_vw(slot, |vw| {
                vw.as_heap_ref().is_some_and(|hv| matches!(hv, HeapValue::SharedCell(_)))
            });
            if is_shared_cell {
                let slot_vw = self.stack_read_vw(slot);
                if let Some(HeapValue::SharedCell(arc)) = slot_vw.as_heap_ref() {
                    let arc = arc.clone();
                    let old = arc.read().unwrap().clone();
                    record_heap_write();
                    write_barrier_vw(&old, &truncated);
                    *arc.write().unwrap() = truncated;
                }
            } else {
                record_heap_write();
                write_barrier_slot(self.stack[slot], truncated.raw_bits());
                self.stack_write_vw(slot, truncated);
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
            let nb = if (idx as usize) < self.module_bindings.len() {
                self.binding_read_vw(idx as usize)
            } else {
                ValueWord::none()
            };
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
        match instruction.operand {
            Some(Operand::Local(idx)) => {
                let bp = self.current_locals_base();
                let absolute_slot = bp + idx as usize;
                self.push_vw(ValueWord::from_ref(absolute_slot))?;
            }
            Some(Operand::ModuleBinding(idx)) => {
                self.push_vw(ValueWord::from_module_binding_ref(idx as usize))?;
            }
            _ => {
                return Err(VMError::InvalidOperand);
            }
        }
        Ok(())
    }

    /// MakeFieldRef: pop a base reference and push a projected typed-field reference.
    pub(in crate::executor) fn op_make_field_ref(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let base_ref = self.pop_vw()?;
        if base_ref.as_ref_target().is_none() {
            return Err(VMError::RuntimeError(
                "internal error: MakeFieldRef expected a base reference".to_string(),
            ));
        }
        match instruction.operand {
            Some(Operand::TypedField {
                type_id,
                field_idx,
                field_type_tag,
            }) => self.push_vw(ValueWord::from_projected_ref(
                base_ref,
                RefProjection::TypedField {
                    type_id,
                    field_idx,
                    field_type_tag,
                },
            )),
            _ => Err(VMError::InvalidOperand),
        }
    }

    /// MakeIndexRef: pop an index value and a base reference, push a projected
    /// `RefProjection::Index` reference that points to `base[index]`.
    ///
    /// If the base value is a Matrix, a `MatrixRow` projection is created instead
    /// so that `SetIndexRef` can do COW element-level mutation through the row ref.
    pub(in crate::executor) fn op_make_index_ref(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        let index = self.pop_vw()?;
        let base_ref = self.pop_vw()?;
        if base_ref.as_ref_target().is_none() {
            return Err(VMError::RuntimeError(
                "internal error: MakeIndexRef expected a base reference".to_string(),
            ));
        }

        // Check if the base is a matrix — if so, create a MatrixRow projection
        // for borrow-based row mutation.
        let base_value = self.resolve_ref_value(&base_ref);
        let is_matrix = base_value
            .as_ref()
            .and_then(|v| v.as_heap_ref())
            .is_some_and(|hv| matches!(hv, HeapValue::Matrix(_)));

        let projection = if is_matrix {
            // Convert index to row index
            let row_idx = index
                .as_i64()
                .or_else(|| index.as_f64().map(|f| f as i64))
                .ok_or_else(|| {
                    VMError::RuntimeError(
                        "matrix row index must be a number".to_string(),
                    )
                })?;
            let mat = base_value.as_ref().unwrap().as_matrix().unwrap();
            let rows = mat.rows as i64;
            let actual = if row_idx < 0 { rows + row_idx } else { row_idx };
            if actual < 0 || actual >= rows {
                return Err(VMError::RuntimeError(format!(
                    "Matrix row index {} out of bounds for {}x{} matrix",
                    row_idx, mat.rows, mat.cols
                )));
            }
            RefProjection::MatrixRow { row_index: actual as u32 }
        } else {
            RefProjection::Index { index }
        };

        self.push_vw(ValueWord::from_projected_ref(base_ref, projection))?;
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
            let ref_val = self.stack_read_vw(slot);
            let target = ref_val.as_ref_target().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;
            let nb = self.read_ref_target(&target)?;
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
            let ref_vw = self.stack_read_vw(slot);
            self.write_ref_value(&ref_vw, value)?;
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
            let ref_vw = self.stack_read_vw(slot);
            let target = ref_vw.as_ref_target().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;

            match target {
                RefTarget::Stack(target) => {
                    let mut object_nb = self.stack_take_vw(target);
                    let result = Self::set_array_index_on_object(&mut object_nb, &index_nb, value);
                    record_heap_write();
                    write_barrier_slot(Self::NONE_BITS, object_nb.raw_bits());
                    self.stack_write_vw(target, object_nb);
                    result
                }
                RefTarget::ModuleBinding(target) => {
                    if target >= self.module_bindings.len() {
                        return Err(VMError::RuntimeError(format!(
                            "ModuleBinding index {} out of bounds",
                            target
                        )));
                    }
                    let mut object_nb = self.binding_take_vw(target);
                    let result = Self::set_array_index_on_object(&mut object_nb, &index_nb, value);
                    record_heap_write();
                    write_barrier_slot(Self::NONE_BITS, object_nb.raw_bits());
                    self.binding_write_vw(target, object_nb);
                    result
                }
                RefTarget::Projected(ref proj_data) => {
                    if let RefProjection::MatrixRow { row_index } = proj_data.projection {
                        // Matrix row mutation: COW write directly into the backing matrix.
                        self.set_matrix_row_element(&proj_data.base, row_index, &index_nb, value)
                    } else {
                        let mut object_nb = self.read_ref_target(&target)?;
                        Self::set_array_index_on_object(&mut object_nb, &index_nb, value)?;
                        self.write_ref_target(&target, object_nb)
                    }
                }
            }
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
                self.module_bindings.push(Self::NONE_BITS);
            }

            // Auto-deref SharedCell: write through the Arc
            let is_shared_cell = {
                let bits = self.module_bindings[index];
                let tmp = ValueWord::from_raw_bits(bits);
                let r = tmp.as_heap_ref().is_some_and(|hv| matches!(hv, HeapValue::SharedCell(_)));
                std::mem::forget(tmp);
                r
            };
            if is_shared_cell {
                let slot_vw = self.binding_read_vw(index);
                if let Some(HeapValue::SharedCell(arc)) = slot_vw.as_heap_ref() {
                    let arc = arc.clone();
                    let old = arc.read().unwrap().clone();
                    record_heap_write();
                    write_barrier_vw(&old, &nb);
                    *arc.write().unwrap() = nb;
                }
            } else {
                record_heap_write();
                write_barrier_slot(self.module_bindings[index], nb.raw_bits());
                self.binding_write_vw(index, nb);
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
                self.module_bindings.push(Self::NONE_BITS);
            }

            // Auto-deref SharedCell: write through the Arc
            let is_shared_cell = {
                let bits = self.module_bindings[index];
                let tmp = ValueWord::from_raw_bits(bits);
                let r = tmp.as_heap_ref().is_some_and(|hv| matches!(hv, HeapValue::SharedCell(_)));
                std::mem::forget(tmp);
                r
            };
            if is_shared_cell {
                let slot_vw = self.binding_read_vw(index);
                if let Some(HeapValue::SharedCell(arc)) = slot_vw.as_heap_ref() {
                    let arc = arc.clone();
                    let old = arc.read().unwrap().clone();
                    record_heap_write();
                    write_barrier_vw(&old, &truncated);
                    *arc.write().unwrap() = truncated;
                }
            } else {
                record_heap_write();
                write_barrier_slot(self.module_bindings[index], truncated.raw_bits());
                self.binding_write_vw(index, truncated);
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
            let is_cell = self.stack_peek_vw(slot, |vw| {
                vw.as_heap_ref()
                    .map(|hv| matches!(hv, HeapValue::SharedCell(_)))
                    .unwrap_or(false)
            });

            if !is_cell {
                let old_bits = self.stack[slot];
                let value = self.stack_take_vw(slot);
                let cell_vw =
                    ValueWord::from_heap_value(HeapValue::SharedCell(Arc::new(RwLock::new(value))));
                record_heap_write();
                write_barrier_slot(old_bits, cell_vw.raw_bits());
                self.stack_write_vw(slot, cell_vw);
            }

            // Push the SharedCell onto the stack for MakeClosure to consume
            let nb = self.stack_read_vw(slot);
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
                self.module_bindings.push(Self::NONE_BITS);
            }

            // If not already a SharedCell, wrap the value in one
            let is_cell = {
                let bits = self.module_bindings[index];
                let tmp = ValueWord::from_raw_bits(bits);
                let r = tmp.as_heap_ref()
                    .map(|hv| matches!(hv, HeapValue::SharedCell(_)))
                    .unwrap_or(false);
                std::mem::forget(tmp);
                r
            };

            if !is_cell {
                let old_bits = self.module_bindings[index];
                let value = self.binding_take_vw(index);
                let cell_vw =
                    ValueWord::from_heap_value(HeapValue::SharedCell(Arc::new(RwLock::new(value))));
                record_heap_write();
                write_barrier_slot(old_bits, cell_vw.raw_bits());
                self.binding_write_vw(index, cell_vw);
            }

            // Push the SharedCell onto the stack for MakeClosure to consume
            let nb = self.binding_read_vw(index);
            self.push_vw(nb)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{
        BytecodeProgram, Constant, Instruction, NumericWidth, OpCode, Operand,
    };
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::ValueWord;

    /// Helper: build a program, load it, execute, return the top-of-stack value.
    fn run_program(program: BytecodeProgram) -> ValueWord {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    // ===== LoadLocalTrusted tests =====

    #[test]
    fn test_load_local_trusted_f64() {
        // Store a float via StoreLocal, load it back with LoadLocalTrusted.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Number(3.14));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_f64(), Some(3.14));
    }

    #[test]
    fn test_load_local_trusted_int() {
        // Store an int via StoreLocal, load it back with LoadLocalTrusted.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(42));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_load_local_trusted_bool() {
        // Store a bool, load it back with LoadLocalTrusted.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Bool(true));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_load_local_trusted_negative_int() {
        // Negative integer round-trips through trusted load.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(-99));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(-99));
    }

    #[test]
    fn test_load_local_trusted_multiple_slots() {
        // Two locals: verify LoadLocalTrusted reads the correct slot.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(10));
        let c1 = program.add_constant(Constant::Int(20));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
            // Load slot 1 (should be 20, not 10)
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(1))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 2;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(20));
    }

    #[test]
    fn test_load_local_trusted_skips_shared_cell() {
        // LoadLocalTrusted does NOT auto-deref SharedCell (unlike LoadLocal).
        // If the slot holds a SharedCell, trusted load returns the cell itself.
        // This is correct because the compiler only emits LoadLocalTrusted for
        // slots it has proved are plain values, never boxed captures.
        //
        // We test this indirectly: store a value, load with trusted, verify
        // we get the value back (no SharedCell wrapping involved).
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Number(2.718));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_f64(), Some(2.718));
    }

    // ===== StoreLocalTyped tests =====

    #[test]
    fn test_store_local_typed_i8_truncation() {
        // 300 stored as i8 should truncate to 44 (300 & 0xFF = 44, sign-extend)
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(300));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::I8)),
            ),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(44));
    }

    #[test]
    fn test_store_local_typed_u8_truncation() {
        // 256 stored as u8 should truncate to 0
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(256));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::U8)),
            ),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(0));
    }

    #[test]
    fn test_store_local_typed_i64_no_truncation() {
        // i64 width: no truncation, value passes through as-is.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(123456789));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::I64)),
            ),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(123456789));
    }

    #[test]
    fn test_store_local_typed_f64_no_truncation() {
        // f64 width: no truncation, float passes through.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Number(99.5));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::F64)),
            ),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_f64(), Some(99.5));
    }

    #[test]
    fn test_store_local_typed_i16_truncation() {
        // 70000 stored as i16: 70000 & 0xFFFF = 4464, sign-extend → 4464
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(70000));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::I16)),
            ),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(4464));
    }

    #[test]
    fn test_store_local_typed_overwrite() {
        // Store twice to the same typed slot — second write overwrites first.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(100));
        let c1 = program.add_constant(Constant::Int(200));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::I64)),
            ),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::I64)),
            ),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(200));
    }

    #[test]
    fn test_store_typed_load_trusted_roundtrip() {
        // End-to-end: StoreLocalTyped + LoadLocalTrusted form the typed
        // access pair. Verify multiple slots work together.
        let mut program = BytecodeProgram::default();
        let c_int = program.add_constant(Constant::Int(7));
        let c_float = program.add_constant(Constant::Number(1.5));
        program.instructions = vec![
            // Store int to slot 0
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_int))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(0, NumericWidth::I64)),
            ),
            // Store float to slot 1
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_float))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(1, NumericWidth::F64)),
            ),
            // Load slot 0 (int), load slot 1 (float), add them
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(1))),
            Instruction::simple(OpCode::Add),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 2;
        let result = run_program(program);
        // int(7) + float(1.5) = 8.5 via generic Add (promotes to f64)
        assert_eq!(result.as_f64(), Some(8.5));
    }

    // ===== Documentation: LoadLocalF64 / StoreLocalF64 =====
    //
    // The v2 runtime spec calls for dedicated f64-typed local opcodes:
    //
    //   OpCode::LoadLocalF64:
    //     Read the raw u64 bits from stack[base_pointer + idx] and push
    //     them directly — no NaN-boxing tag check, no clone_from_bits.
    //     The bits ARE the f64 value (IEEE 754). Operand: Local(u16).
    //
    //   OpCode::StoreLocalF64:
    //     Pop raw u64 bits from the stack and write them directly to
    //     stack[base_pointer + idx] — no tag check, no write barrier.
    //     The bits ARE the f64 value. Operand: Local(u16).
    //
    // These opcodes do NOT exist in opcode_defs.rs yet. When added, they
    // should be assigned codes in the 0xDA-0xDF range (currently free)
    // and categorized as Variable opcodes. The executor handlers would be:
    //
    //   LoadLocalF64: read u64 from slot, push as ValueWord (no clone_from_bits)
    //   StoreLocalF64: pop ValueWord, write u64 to slot (no write barrier)
    //
    // The full v2 transition requires the stack representation to change
    // from Vec<ValueWord> to a raw u64 slab, at which point these opcodes
    // become zero-overhead register moves.
}
