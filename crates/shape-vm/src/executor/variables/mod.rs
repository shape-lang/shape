//! Variable operations for the VM executor
//!
//! Handles: LoadLocal, StoreLocal, LoadModuleBinding, StoreModuleBinding, LoadClosure, StoreClosure, CloseUpvalue

use crate::executor::objects::object_creation::clone_slots_with_update;
use crate::executor::objects::raw_helpers;
use crate::executor::typed_object_ops::{read_slot_fast, tag_to_field_type};
use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
    memory::{record_heap_write, write_barrier_slot, write_barrier_vw},
};
use shape_value::heap_value::HeapValue;
use shape_value::nanboxed::RefTarget;
use shape_value::{RefProjection, VMError, ValueWord, ValueWordExt};
use std::sync::{Arc, RwLock};
impl VirtualMachine {
    pub(in crate::executor) fn read_ref_target(
        &self,
        target: &RefTarget,
    ) -> Result<ValueWord, VMError> {
        match target {
            RefTarget::Stack(slot) => {
                if *slot < self.stack.len() {
                    Ok(self.stack_read_raw(*slot))
                } else {
                    Ok(ValueWord::none())
                }
            }
            RefTarget::ModuleBinding(slot) => {
                if *slot < self.module_bindings.len() {
                    Ok(self.binding_read_raw(*slot))
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
                    let bits = raw_helpers::unwrap_annotated_bits(base_value.raw_bits());
                    if let Some((_schema_id, slots, heap_mask)) =
                        raw_helpers::extract_typed_object(bits)
                    {
                        let index = *field_idx as usize;
                        if index < slots.len() {
                            let is_heap = (heap_mask & (1u64 << index)) != 0;
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
                    let unwrapped_bits = raw_helpers::unwrap_annotated_bits(base_value.raw_bits());
                    let base_value = if unwrapped_bits != base_value.raw_bits() {
                        unsafe { ValueWord::clone_from_bits(unwrapped_bits) }
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
                    if let Some(mat_arc) = raw_helpers::extract_matrix_arc(base_value.raw_bits()) {
                        let cols = mat_arc.cols;
                        let offset = *row_index * cols;
                        if *row_index < mat_arc.rows {
                            return Ok(ValueWord::from_heap_value(
                                HeapValue::TypedArray(shape_value::TypedArrayData::FloatSlice {
                                    parent: mat_arc,
                                    offset,
                                    len: cols,
                                }),
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
                self.stack_write_raw(*target, value);
                Ok(())
            }
            RefTarget::ModuleBinding(target) => {
                if *target >= self.module_bindings.len() {
                    self.module_bindings
                        .resize_with(*target + 1, || Self::NONE_BITS);
                }
                write_barrier_slot(self.module_bindings[*target], value.raw_bits());
                self.binding_write_raw(*target, value);
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
                    let bits = raw_helpers::unwrap_annotated_bits(base_value.raw_bits());
                    if let Some((schema_id, slots, heap_mask)) =
                        raw_helpers::extract_typed_object(bits)
                    {
                        let field_type = tag_to_field_type(*field_type_tag);
                        let (new_slots, new_mask) = clone_slots_with_update(
                            slots,
                            heap_mask,
                            *field_idx as usize,
                            &value,
                            field_type.as_ref(),
                        );
                        return self.write_ref_value(
                            &data.base,
                            ValueWord::from_heap_value(HeapValue::TypedObject {
                                schema_id,
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
                    let unwrapped_bits = raw_helpers::unwrap_annotated_bits(base_value.raw_bits());
                    let mut base_value = if unwrapped_bits != base_value.raw_bits() {
                        unsafe { ValueWord::clone_from_bits(unwrapped_bits) }
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
                let mut matrix_vw = self.stack_take_raw(slot);
                let result = Self::cow_matrix_write(&mut matrix_vw, row_index, col_idx, val_f64);
                self.stack_write_raw(slot, matrix_vw);
                result
            }
            RefTarget::ModuleBinding(slot) => {
                if slot >= self.module_bindings.len() {
                    return Err(VMError::RuntimeError(format!(
                        "ModuleBinding index {} out of bounds",
                        slot
                    )));
                }
                let mut matrix_vw = self.binding_take_raw(slot);
                let result = Self::cow_matrix_write(&mut matrix_vw, row_index, col_idx, val_f64);
                self.binding_write_raw(slot, matrix_vw);
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
            HeapValue::TypedArray(shape_value::TypedArrayData::Matrix(arc)) => {
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
            LoadLocalMove => self.op_load_local_move(instruction)?,
            LoadLocalClone => self.op_load_local_clone(instruction)?,
            StoreLocal => self.op_store_local(instruction)?,
            StoreLocalTyped => self.op_store_local_typed(instruction)?,
            StoreLocalDrop => self.op_store_local_drop(instruction)?,
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
            LoadCaptureMutPtrF64 => self.op_load_capture_mut_ptr_f64(instruction)?,
            LoadCaptureMutPtrI64 => self.op_load_capture_mut_ptr_i64(instruction)?,
            LoadCaptureMutPtrI32 => self.op_load_capture_mut_ptr_i32(instruction)?,
            LoadCaptureMutPtrBool => self.op_load_capture_mut_ptr_bool(instruction)?,
            LoadCaptureMutPtrPtr => self.op_load_capture_mut_ptr_ptr(instruction)?,
            StoreCaptureMutPtrF64 => self.op_store_capture_mut_ptr_f64(instruction)?,
            StoreCaptureMutPtrI64 => self.op_store_capture_mut_ptr_i64(instruction)?,
            StoreCaptureMutPtrI32 => self.op_store_capture_mut_ptr_i32(instruction)?,
            StoreCaptureMutPtrBool => self.op_store_capture_mut_ptr_bool(instruction)?,
            StoreCaptureMutPtrPtr => self.op_store_capture_mut_ptr_ptr(instruction)?,
            LoadOwnedMutableCapture => self.op_load_owned_mutable_capture(instruction)?,
            StoreOwnedMutableCapture => self.op_store_owned_mutable_capture(instruction)?,
            LoadSharedCapture => self.op_load_shared_capture(instruction)?,
            StoreSharedCapture => self.op_store_shared_capture(instruction)?,
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
                        self.push_raw_u64(value_nb)?;
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
            let value_nb = self.pop_raw_u64()?;
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
        // H3: closures capture ValueWords directly; shared mutability, when
        // required, rides on a `HeapValue::SharedCell` inside the captured
        // ValueWord, so "closing" is implicit on capture.
        Ok(())
    }

    // ── Closure Spec Phase D / H3: typed mutable-capture pointer access ──
    //
    // The interpreter backing for `LoadCaptureMutPtrT` / `StoreCaptureMutPtrT`
    // is a `HeapValue::SharedCell`-backed `Upvalue` (H3 collapsed the
    // former mutable-upvalue enum variant into a single-`ValueWord` upvalue
    // whose `get`/`set` transparently deref through a SharedCell when one is
    // present). Phase D's invariant is
    // that the compiler has proven (a) the closure is non-escaping,
    // (b) the outer slot has `BindingStorageClass::LocalMutablePtr`,
    // (c) the MIR solver registered an exclusive loan on the outer slot
    // for the closure's lifetime, and (d) the ValueWord stored in the
    // shared cell carries the declared encoding (F64/I64/I32/Bool/Ptr).
    // The typed opcodes skip the tag dispatch on read.
    //
    // Phase E replaces this path with a real raw `*mut T` into a Cranelift
    // `StackSlot`. The opcode-level ABI stays identical — only the executor
    // implementation changes.

    /// Read the raw ValueWord stored behind the mutable capture pointer at
    /// `upvalue_idx`. Auto-dereferences `SharedCell`-wrapped upvalues, which
    /// is how the interpreter simulates a typed `*mut T` in Phase D.
    ///
    /// # Safety
    ///
    /// The caller must have verified (via the compiler storage plan) that
    /// the capture at `upvalue_idx` is `LocalMutablePtr` and the ValueWord
    /// encoding matches the opcode's declared type. The MIR solver has
    /// registered an exclusive loan on the outer slot spanning the closure's
    /// lifetime, so no outer read/write can race the access.
    #[inline]
    fn read_capture_mut_cell(&self, upvalue_idx: u16) -> Result<ValueWord, VMError> {
        let frame = self.call_stack.last().ok_or_else(|| {
            VMError::RuntimeError("closure capture read outside a call frame".to_string())
        })?;
        let upvalues = frame.upvalues.as_ref().ok_or_else(|| {
            VMError::RuntimeError(
                "closure capture read in a frame without upvalues".to_string(),
            )
        })?;
        let upvalue = upvalues.get(upvalue_idx as usize).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "capture index {} not found in closure",
                upvalue_idx
            ))
        })?;
        // `Upvalue::get()` auto-derefs through a SharedCell when the
        // upvalue's ValueWord is the boxed outer slot; for Phase D bindings
        // that's exactly what the upstream `BoxLocal`-emitting compiler
        // arranges, so the returned value is the underlying scalar, not the
        // SharedCell wrapper. H3 collapsed the former mutable-upvalue enum
        // variant: the SharedCell ValueWord itself rides inside the single
        // `Upvalue` payload now.
        Ok(upvalue.get())
    }

    /// Write a ValueWord back through the mutable capture pointer at
    /// `upvalue_idx`. See `read_capture_mut_cell` for the safety preconditions.
    #[inline]
    fn write_capture_mut_cell(
        &mut self,
        upvalue_idx: u16,
        value: ValueWord,
    ) -> Result<(), VMError> {
        let frame = self.call_stack.last_mut().ok_or_else(|| {
            VMError::RuntimeError("closure capture write outside a call frame".to_string())
        })?;
        let upvalues = frame.upvalues.as_mut().ok_or_else(|| {
            VMError::RuntimeError(
                "closure capture write in a frame without upvalues".to_string(),
            )
        })?;
        let upvalue = upvalues.get_mut(upvalue_idx as usize).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "capture index {} not found in closure",
                upvalue_idx
            ))
        })?;
        record_heap_write();
        write_barrier_vw(&upvalue.get(), &value);
        upvalue.set(value);
        Ok(())
    }

    /// `LoadCaptureMutPtrF64 { idx }`: read the f64 stored behind capture `idx`.
    fn op_load_capture_mut_ptr_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.read_capture_mut_cell(idx)?;
        // SAFETY: compiler-proved f64 encoding in the capture cell. Fall back
        // via ValueWord bits to remain correct if the cell is slightly more
        // general (e.g. the SharedCell-based Phase D backing may hold a
        // non-canonical encoding; `as_f64` handles both). Phase E will read
        // raw f64 bits from the typed stack slot directly.
        let f = value
            .as_f64()
            .or_else(|| value.as_i64().map(|i| i as f64))
            .ok_or_else(|| {
                VMError::RuntimeError(
                    "LoadCaptureMutPtrF64: capture does not encode f64".to_string(),
                )
            })?;
        self.push_raw_f64(f)
    }

    /// `LoadCaptureMutPtrI64 { idx }`: read the i64 stored behind capture `idx`.
    fn op_load_capture_mut_ptr_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.read_capture_mut_cell(idx)?;
        // SAFETY: compiler-proved i64 encoding. Push raw u64 bits preserving
        // the i48-tagged NaN-boxed integer so downstream pop_raw_i64 decodes.
        self.push_raw_u64(value.raw_bits())
    }

    /// `LoadCaptureMutPtrI32 { idx }`: read an i32 stored behind capture `idx`.
    fn op_load_capture_mut_ptr_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.read_capture_mut_cell(idx)?;
        // i32 is stored in the same i48 NaN-boxed integer encoding as i64
        // for Phase D. Preserve the bit pattern.
        self.push_raw_u64(value.raw_bits())
    }

    /// `LoadCaptureMutPtrBool { idx }`: read the bool stored behind capture `idx`.
    fn op_load_capture_mut_ptr_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.read_capture_mut_cell(idx)?;
        // SAFETY: compiler-proved bool encoding. push_raw_u64 preserves the
        // TAG_BOOL pattern so downstream pop_raw_bool can decode.
        self.push_raw_u64(value.raw_bits())
    }

    /// `LoadCaptureMutPtrPtr { idx }`: read a heap pointer (e.g. TypedArray,
    /// String, Struct) stored behind capture `idx`.
    fn op_load_capture_mut_ptr_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.read_capture_mut_cell(idx)?;
        // Heap pointer. `Upvalue::get` already bumped the Arc via clone, so
        // the caller owns the returned ValueWord. Push as raw bits.
        self.push_raw_u64(value.raw_bits())
    }

    /// `StoreCaptureMutPtrF64 { idx }`: pop f64 and write through capture `idx`.
    fn op_store_capture_mut_ptr_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let f = self.pop_raw_f64()?;
        let value = ValueWord::from_f64(f);
        self.write_capture_mut_cell(idx, value)
    }

    /// `StoreCaptureMutPtrI64 { idx }`: pop i64 and write through capture `idx`.
    fn op_store_capture_mut_ptr_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let raw = self.pop_raw_u64()?;
        // The raw bits already encode i64 via the i48 NaN-box. Reconstruct
        // a ValueWord without decoding to preserve the tag.
        let value = ValueWord::from_raw_bits(raw);
        self.write_capture_mut_cell(idx, value)
    }

    /// `StoreCaptureMutPtrI32 { idx }`: pop i32 and write through capture `idx`.
    fn op_store_capture_mut_ptr_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let raw = self.pop_raw_u64()?;
        let value = ValueWord::from_raw_bits(raw);
        self.write_capture_mut_cell(idx, value)
    }

    /// `StoreCaptureMutPtrBool { idx }`: pop bool and write through capture `idx`.
    fn op_store_capture_mut_ptr_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let raw = self.pop_raw_u64()?;
        let value = ValueWord::from_raw_bits(raw);
        self.write_capture_mut_cell(idx, value)
    }

    /// `StoreCaptureMutPtrPtr { idx }`: pop heap pointer and write through capture `idx`.
    fn op_store_capture_mut_ptr_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let raw = self.pop_raw_u64()?;
        let value = ValueWord::from_raw_bits(raw);
        self.write_capture_mut_cell(idx, value)
    }

    // ── Track A.1B: CaptureKind::OwnedMutable / Shared interpreter path ──
    //
    // The upvalue slot for A.1B's mutable/shared captures holds *raw
    // pointer bits* (not a ValueWord payload the way `Upvalue::get/set`
    // expect). We therefore read the slot's raw u64 directly and bypass
    // `Upvalue::get()` — the auto-deref on `HeapValue::SharedCell` is
    // specific to the retired-in-A.1C legacy mutable-capture fallback and
    // MUST NOT run on an `OwnedMutable` / `Shared` upvalue (whose bits
    // are not a NaN-tagged heap pointer in the first place).
    //
    // SAFETY invariants for every handler below:
    //
    // - The compiler (A.1C) emits these opcodes only for captures whose
    //   `ClosureLayout::capture_storage_kind(i)` is the matching variant.
    //   `op_make_closure` (A.1B) and the closure call plumbing preserve
    //   the raw pointer bits through `frame.upvalues[i]`.
    // - The upvalue's raw u64 is either `Box::into_raw(Box::new(ValueWord))`
    //   (for OwnedMutable) or `Arc::into_raw(Arc::new(SharedCell))` (for
    //   Shared). Both are 8-aligned allocations produced from Rust's
    //   global allocator and live for the closure's refcounted lifetime.
    // - The pointer is released exactly once by `release_typed_closure`
    //   via `Box::from_raw` / `Arc::from_raw` when the closure's refcount
    //   hits zero; interpreter reads/writes do NOT consume a reference
    //   count and do NOT transfer ownership.

    /// Read the raw pointer bits stored behind upvalue `upvalue_idx`.
    /// Bypasses `Upvalue::get()` so `HeapValue::SharedCell` auto-deref
    /// does NOT run on bits that encode a raw `*mut ValueWord` /
    /// `*const SharedCell`. Used only by
    /// `LoadOwnedMutableCapture` / `StoreOwnedMutableCapture` /
    /// `LoadSharedCapture` / `StoreSharedCapture`.
    #[inline]
    fn read_capture_raw_pointer_bits(&self, upvalue_idx: u16) -> Result<u64, VMError> {
        let frame = self.call_stack.last().ok_or_else(|| {
            VMError::RuntimeError(
                "mutable/shared capture access outside a call frame".to_string(),
            )
        })?;
        let upvalues = frame.upvalues.as_ref().ok_or_else(|| {
            VMError::RuntimeError(
                "mutable/shared capture access in a frame without upvalues".to_string(),
            )
        })?;
        let upvalue = upvalues.get(upvalue_idx as usize).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "capture index {} not found in closure",
                upvalue_idx
            ))
        })?;
        // NOTE: intentionally not calling `Upvalue::get()` — see module
        // header above. We want the raw bits (a pointer), not a
        // SharedCell-auto-dereffed ValueWord.
        //
        // Upvalue's inner ValueWord is a `u64` alias (see
        // `shape_value::value_word`), so cloning & decoding bits is
        // zero-cost. We read via clone() → raw_bits() because Upvalue's
        // only public accessor for the inner is `.get()` (which
        // auto-dereffs). Workaround: take the inner by Clone. The clone
        // of Upvalue clones the inner u64 bits without invoking
        // SharedCell semantics — Upvalue::new(value) just stashes the
        // value.
        Ok(upvalue.clone_inner_bits_for_raw_pointer_access())
    }

    /// `LoadOwnedMutableCapture { idx }`: read the ValueWord held in the
    /// `*mut ValueWord` box cell behind capture `idx`. Pushes the inner
    /// value onto the stack as raw bits.
    ///
    /// # Safety
    ///
    /// The capture at `idx` must have `CaptureKind::OwnedMutable`. The
    /// upvalue slot must contain a non-null pointer obtained from
    /// `Box::into_raw(Box::new(ValueWord))`. The pointer is valid for the
    /// closure's refcounted lifetime; it is released exactly once by
    /// `release_typed_closure` via `Box::from_raw`.
    fn op_load_owned_mutable_capture(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut ValueWord;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: `cell_ptr` was produced by `Box::into_raw(Box::new(vw))`
        // (see `op_make_closure` A.1B allocation path). It is 8-aligned
        // (Box alignment for `u64`), non-null (checked above), and
        // exclusively owned by this closure's refcounted block — no
        // aliasing or sharing semantics apply to OwnedMutable. Reading 8
        // bytes through it produces a valid `ValueWord` u64. The lifetime
        // is bounded by the surrounding closure call frame's upvalues
        // vec, which keeps the ClosureRaw block alive (and therefore the
        // box alive) for the duration of this call.
        let value = unsafe { std::ptr::read(cell_ptr) };
        self.push_raw_u64(value)
    }

    /// `StoreOwnedMutableCapture { idx }`: pop a ValueWord and write it
    /// through the `*mut ValueWord` box cell behind capture `idx`.
    ///
    /// # Safety
    ///
    /// Same invariants as `op_load_owned_mutable_capture`. The old cell
    /// contents are overwritten in place — if the old contents were a
    /// heap-tagged ValueWord share, the caller is responsible for
    /// ensuring the write does not leak (typical case: integer/float
    /// capture, inline bits have no Drop glue).
    fn op_store_owned_mutable_capture(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut ValueWord;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: same invariants as `op_load_owned_mutable_capture`. The
        // 8-byte write at `cell_ptr` replaces the previous ValueWord bit
        // pattern in-place. Alignment and provenance are upheld by the
        // Box allocator. `new_bits` is a valid ValueWord u64 produced by
        // the executor.
        unsafe {
            std::ptr::write(cell_ptr, new_bits);
        }
        Ok(())
    }

    /// `LoadSharedCapture { idx }`: acquire the parking_lot mutex behind
    /// capture `idx`, clone the inner ValueWord bits, drop the guard, and
    /// push the value onto the stack.
    ///
    /// # Safety
    ///
    /// The capture at `idx` must have `CaptureKind::Shared`. The upvalue
    /// slot must contain a non-null `*const SharedCell` obtained from
    /// `Arc::into_raw(Arc::new(parking_lot::Mutex::new(ValueWord)))`. The
    /// strong-count share is held by the closure (the block owns the
    /// `Arc::into_raw`-produced share); this handler only reborrows the
    /// underlying allocation via `&*cell_ptr` — the reference's lifetime
    /// is bounded by this handler invocation. No retain/release traffic.
    fn op_load_shared_capture(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        // SAFETY: `cell_ptr` was produced by `Arc::into_raw(Arc::new(...))`
        // (see A.1B `op_make_closure` allocation path). It is 8-aligned,
        // non-null, and represents a live Arc strong-count share owned by
        // the surrounding ClosureRaw block (kept alive for the duration
        // of this call frame). Reborrowing via `&*cell_ptr` is sound as
        // long as we don't outlive the Arc — we drop the reference at
        // the end of this function, well before `release_typed_closure`
        // runs.
        let value_bits = unsafe {
            let cell: &SharedCell = &*cell_ptr;
            let guard = cell.lock();
            let bits = *guard;
            drop(guard);
            bits
        };
        self.push_raw_u64(value_bits)
    }

    /// `StoreSharedCapture { idx }`: pop a ValueWord, acquire the
    /// parking_lot mutex behind capture `idx`, overwrite the inner
    /// ValueWord bits, and drop the guard.
    ///
    /// # Safety
    ///
    /// Same invariants as `op_load_shared_capture`. The mutex serialises
    /// concurrent writers; the Arc strong-count is not modified by this
    /// handler.
    fn op_store_shared_capture(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: same invariants as `op_load_shared_capture`. We take
        // the mutex for exclusive access, overwrite the 8-byte ValueWord
        // payload, then drop the guard. The Arc strong-count share owned
        // by the closure remains intact.
        unsafe {
            let cell: &SharedCell = &*cell_ptr;
            let mut guard = cell.lock();
            *guard = new_bits;
            drop(guard);
        }
        Ok(())
    }

    /// Load value from a local variable slot (register window on the unified stack).
    ///
    /// Optimized: reads the raw u64 bits directly via pointer to skip bounds
    /// checks and Option wrapping. For inline values (numbers, ints, bools —
    /// the common case), constructs a ValueWord by bit-copying without going
    /// through clone dispatch. Only heap-tagged values take the clone path.
    ///
    /// **Stage 2.1**: When the FrameDescriptor proves the slot kind is a
    /// scalar (Float64/Int64/Bool) AND the slot bits are not heap-tagged
    /// (i.e. not wrapped in a SharedCell for closure capture), the handler
    /// pushes raw bits directly without constructing a ValueWord. This gives
    /// downstream typed handlers raw values they can `pop_raw_*` without
    /// unwrapping. The TAG_HEAP runtime check is necessary because mutable
    /// captured locals can be wrapped in SharedCell after the frame layout
    /// is fixed — the FrameDescriptor doesn't track that wrapping.
    ///
    /// If the slot contains a SharedCell (boxed local for mutable closure capture),
    /// the inner value is read through the Arc transparently.
    pub(in crate::executor) fn op_load_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use crate::type_tracking::SlotKind;
        use shape_value::tag_bits::{get_tag, is_tagged, TAG_BOOL, TAG_INT};

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
            // window which is pre-allocated on the stack.
            let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };

            // Stage 2.1 smart fast path. Skip ValueWord construction entirely
            // when the FrameDescriptor proves the slot kind is a scalar AND
            // the runtime bits actually carry that encoding. The runtime tag
            // check is intentionally strict because op_load_local is the
            // untrusted variant: the compiler is sometimes optimistic about
            // slot kinds and the actual bits may be a different encoding
            // (e.g. TAG_INT in a Float64 slot, or a SharedCell wrap that
            // happens at closure construction). Mismatches fall through to
            // the legacy clone_from_bits path which preserves correctness.
            let kind = self
                .current_frame_descriptor()
                .map(|fd| fd.slot(idx as usize))
                .unwrap_or(SlotKind::Unknown);
            match kind {
                SlotKind::Float64 if !is_tagged(bits) => {
                    // Plain f64 bits (NaN canonicalized to a non-tagged
                    // pattern). Push raw without ValueWord wrapping.
                    return self.push_raw_f64(f64::from_bits(bits));
                }
                SlotKind::Int64 | SlotKind::IntSize
                    if is_tagged(bits) && get_tag(bits) == TAG_INT =>
                {
                    // Slot holds i48-tagged bits; push_raw_u64 preserves
                    // the encoding so downstream pop_raw_i64 can decode.
                    return self.push_raw_u64(bits);
                }
                SlotKind::Bool if is_tagged(bits) && get_tag(bits) == TAG_BOOL => {
                    // Slot holds TAG_BOOL bits; push_raw_u64 preserves
                    // the encoding so downstream pop_raw_bool can decode.
                    return self.push_raw_u64(bits);
                }
                _ => {
                    // Tag/kind mismatch, non-scalar slot, or Unknown — fall
                    // through to the legacy clone_from_bits + SharedCell path.
                }
            }

            // Legacy path: clone_from_bits + SharedCell auto-deref.
            if let Some(arc) = raw_helpers::extract_shared_cell(bits) {
                let inner = arc.read().unwrap().clone();
                self.push_raw_u64(inner)?;
            } else {
                let nb = unsafe { ValueWord::clone_from_bits(bits) };
                self.push_raw_u64(nb)?;
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
    /// **Wave C Phase C1**: When the FrameDescriptor proves the slot is a
    /// scalar (Float64/Int64/Bool), the handler skips ValueWord wrapping
    /// entirely and uses `push_raw_*` directly. This is the foundation that
    /// enables downstream typed handlers (e.g. `exec_typed_arithmetic`) to
    /// avoid `pop_vw`/`pop_vw().as_*_unchecked()` patterns. For Heap-typed
    /// slots the legacy `clone_from_bits` + `push_vw` path is preserved
    /// (Arc refcount bump is required).
    #[inline(always)]
    pub(in crate::executor) fn op_load_local_trusted(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use crate::type_tracking::SlotKind;
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
            let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };

            // Wave C Phase C1: smart raw push when FrameDescriptor proves the
            // slot kind is a scalar. Avoids ValueWord wrapping for the hot path.
            let kind = self
                .current_frame_descriptor()
                .map(|fd| fd.slot(idx as usize))
                .unwrap_or(SlotKind::Unknown);
            match kind {
                SlotKind::Float64 => {
                    self.push_raw_f64(f64::from_bits(bits))?;
                }
                SlotKind::Int64 | SlotKind::IntSize => {
                    // The slot holds an i48-tagged NaN-boxed integer.
                    // pop_raw_i64 expects the same encoding, so push_raw_u64
                    // (which writes raw bits without canonicalization) is the
                    // correct symmetric op.
                    self.push_raw_u64(bits)?;
                }
                SlotKind::Bool => {
                    // The slot holds a NaN-boxed bool. push_raw_u64 preserves
                    // the encoding so downstream pop_raw_bool can decode.
                    self.push_raw_u64(bits)?;
                }
                _ => {
                    // Heap or unknown slot — preserve legacy refcount-aware path.
                    let nb = unsafe { ValueWord::clone_from_bits(bits) };
                    self.push_raw_u64(nb)?;
                }
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Load local with Move semantics. The source slot is zeroed (set to
    /// NONE_BITS) so it cannot be used again. This avoids refcount
    /// manipulation entirely — the value is transferred, not cloned.
    ///
    /// If the slot contains a SharedCell (mutable closure capture), moving
    /// out would invalidate other closures that share the cell. In that
    /// case we fall back to clone behaviour (read through the Arc, bump
    /// refcount) and leave the cell in place.
    fn op_load_local_move(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "LoadLocalMove slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            let bits = self.stack[slot];

            // SharedCell guard: can't move out of a shared cell — other
            // closures may still reference it. Fall back to clone.
            if let Some(arc) = raw_helpers::extract_shared_cell(bits) {
                let inner = arc.read().unwrap().clone();
                self.push_raw_u64(inner)?;
            } else {
                // Transfer: zero the source slot (NONE_BITS is an inline tag,
                // no heap allocation, no drop needed) and push the raw bits.
                self.stack[slot] = Self::NONE_BITS;
                self.push_raw_u64(bits)?;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Load local with Clone semantics. The source stays live.
    /// For heap-tagged values, this bumps the Arc refcount.
    ///
    /// This is semantically equivalent to the current LoadLocal behaviour
    /// (always clones), but is emitted explicitly by the compiler when it
    /// knows the value is still needed after the load.
    ///
    /// Like LoadLocal, SharedCell auto-deref is performed: if the slot
    /// contains a SharedCell the inner value is read through the Arc.
    fn op_load_local_clone(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "LoadLocalClone slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            let bits = self.stack[slot];

            // SharedCell auto-deref: read through the Arc.
            if let Some(arc) = raw_helpers::extract_shared_cell(bits) {
                let inner = arc.read().unwrap().clone();
                self.push_raw_u64(inner)?;
            } else {
                // Clone: bump refcount for heap values.
                let cloned = raw_helpers::clone_raw_bits(bits);
                self.push_raw_u64(cloned)?;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Store to local, dropping the old value first.
    /// Used for reassignment where the old value needs cleanup.
    ///
    /// For heap-tagged old values whose Arc refcount reaches zero, the
    /// HeapValue is freed immediately. For inline old values (int, bool,
    /// f64, unit, none) the "drop" is a no-op since they carry no heap
    /// resource.
    ///
    /// If the slot contains a SharedCell (mutable closure capture), the
    /// new value is written through the Arc so all holders see the update.
    fn op_store_local_drop(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::tag_bits::{get_tag, is_tagged, TAG_HEAP};

        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;

            if slot >= self.stack.len() {
                self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            }

            let new_bits = self.pop_raw_u64()?;
            let old_bits = self.stack[slot];

            // SharedCell auto-deref: write through the Arc.
            if let Some(arc) = raw_helpers::extract_shared_cell(old_bits) {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &new_bits);
                *arc.write().unwrap() = new_bits;
            } else {
                // Drop the old value: decrement Arc refcount for heap-tagged values.
                if is_tagged(old_bits) && get_tag(old_bits) == TAG_HEAP {
                    raw_helpers::drop_raw_bits(old_bits);
                }
                // Store new value directly.
                record_heap_write();
                write_barrier_slot(old_bits, new_bits);
                self.stack[slot] = new_bits;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Store value to a local variable slot (register window on the unified stack).
    ///
    /// **Stage 2.1**: When the FrameDescriptor proves the slot kind is a
    /// scalar (Float64/Int64/Bool), the old slot bits are not heap-tagged
    /// (so no SharedCell), and the new top-of-stack bits are also not
    /// heap-tagged, the handler writes raw bits directly without going
    /// through ValueWord pop/construction. The SharedCell check via slot
    /// bits is necessary because mutable captured locals can be wrapped
    /// after the FrameDescriptor is fixed.
    ///
    /// If the slot contains a SharedCell (boxed local for mutable closure capture),
    /// the value is written through the Arc so all holders see the update.
    pub(in crate::executor) fn op_store_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use crate::type_tracking::SlotKind;
        use shape_value::tag_bits::{get_tag, is_tagged, TAG_BOOL, TAG_HEAP, TAG_INT};

        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;

            // Ensure stack is large enough (should already be, but safety check)
            if slot >= self.stack.len() {
                self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            }

            // Stage 2.1 smart fast path: skip ValueWord pop/construction when
            // the FrameDescriptor proves the slot kind is a scalar, the old
            // slot bits are not heap-tagged (so no SharedCell to write
            // through), AND the new top-of-stack bits actually carry the
            // declared scalar encoding. The runtime tag check on the new
            // value mirrors op_load_local: the compiler is sometimes
            // optimistic about slot kinds.
            let old_bits = self.stack[slot];
            let old_is_heap = is_tagged(old_bits) && get_tag(old_bits) == TAG_HEAP;
            if !old_is_heap && self.sp > 0 {
                let kind = self
                    .current_frame_descriptor()
                    .map(|fd| fd.slot(idx as usize))
                    .unwrap_or(SlotKind::Unknown);
                let new_bits = self.stack[self.sp - 1];
                let new_matches_kind = match kind {
                    SlotKind::Float64 => !is_tagged(new_bits),
                    SlotKind::Int64 | SlotKind::IntSize => {
                        is_tagged(new_bits) && get_tag(new_bits) == TAG_INT
                    }
                    SlotKind::Bool => is_tagged(new_bits) && get_tag(new_bits) == TAG_BOOL,
                    _ => false,
                };
                if new_matches_kind {
                    // Pop top of stack without constructing a ValueWord.
                    self.stack[self.sp - 1] = Self::NONE_BITS;
                    self.sp -= 1;
                    // Direct raw write — both old and new are scalar.
                    record_heap_write();
                    self.stack[slot] = new_bits;
                    return Ok(());
                }
            }

            // Legacy path: pop ValueWord and handle SharedCell auto-deref.
            let nb = self.pop_raw_u64()?;

            // Auto-deref SharedCell: write through the Arc
            if let Some(arc) = raw_helpers::extract_shared_cell(self.stack[slot]) {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &nb);
                *arc.write().unwrap() = nb;
            } else {
                record_heap_write();
                write_barrier_slot(self.stack[slot], nb.raw_bits());
                self.stack_write_raw(slot, nb);
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
            let nb = self.pop_raw_u64()?;
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

            if let Some(arc) = raw_helpers::extract_shared_cell(self.stack[slot]) {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &truncated);
                *arc.write().unwrap() = truncated;
            } else {
                record_heap_write();
                write_barrier_slot(self.stack[slot], truncated.raw_bits());
                self.stack_write_raw(slot, truncated);
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
                self.binding_read_raw(idx as usize)
            } else {
                ValueWord::none()
            };
            // Auto-deref SharedCell
            if let Some(arc) = raw_helpers::extract_shared_cell(nb.raw_bits()) {
                let inner = arc.read().unwrap().clone();
                self.push_raw_u64(inner)?;
            } else {
                self.push_raw_u64(nb)?;
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
                self.push_raw_u64(ValueWord::from_ref(absolute_slot))?;
            }
            Some(Operand::ModuleBinding(idx)) => {
                self.push_raw_u64(ValueWord::from_module_binding_ref(idx as usize))?;
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
        let base_ref = self.pop_raw_u64()?;
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
            }) => self.push_raw_u64(ValueWord::from_projected_ref(
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
        let index = self.pop_raw_u64()?;
        let base_ref = self.pop_raw_u64()?;
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
            .and_then(|v| raw_helpers::extract_matrix(v.raw_bits()))
            .is_some();

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

        self.push_raw_u64(ValueWord::from_projected_ref(base_ref, projection))?;
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
            let ref_val = self.stack_read_raw(slot);
            let target = ref_val.as_ref_target().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;
            let nb = self.read_ref_target(&target)?;
            self.push_raw_u64(nb)?;
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
            let value = self.pop_raw_u64()?;
            let bp = self.current_locals_base();
            let slot = bp + ref_slot as usize;
            let ref_vw = self.stack_read_raw(slot);
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
            let value = self.pop_raw_u64()?;
            let index_nb = self.pop_raw_u64()?;
            let bp = self.current_locals_base();
            let slot = bp + ref_slot as usize;
            let ref_vw = self.stack_read_raw(slot);
            let target = ref_vw.as_ref_target().ok_or_else(|| {
                VMError::RuntimeError(
                    "internal error: expected a reference value (&) but found a regular value. \
                     This is a compiler bug — please report it"
                        .to_string(),
                )
            })?;

            match target {
                RefTarget::Stack(target) => {
                    let mut object_nb = self.stack_take_raw(target);
                    let result = Self::set_array_index_on_object(&mut object_nb, &index_nb, value);
                    record_heap_write();
                    write_barrier_slot(Self::NONE_BITS, object_nb.raw_bits());
                    self.stack_write_raw(target, object_nb);
                    result
                }
                RefTarget::ModuleBinding(target) => {
                    if target >= self.module_bindings.len() {
                        return Err(VMError::RuntimeError(format!(
                            "ModuleBinding index {} out of bounds",
                            target
                        )));
                    }
                    let mut object_nb = self.binding_take_raw(target);
                    let result = Self::set_array_index_on_object(&mut object_nb, &index_nb, value);
                    record_heap_write();
                    write_barrier_slot(Self::NONE_BITS, object_nb.raw_bits());
                    self.binding_write_raw(target, object_nb);
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
            let nb = self.pop_raw_u64()?;
            let index = idx as usize;

            // Ensure module_bindings vector is large enough
            while self.module_bindings.len() <= index {
                self.module_bindings.push(Self::NONE_BITS);
            }

            // Auto-deref SharedCell: write through the Arc
            if let Some(arc) = raw_helpers::extract_shared_cell(self.module_bindings[index]) {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &nb);
                *arc.write().unwrap() = nb;
            } else {
                record_heap_write();
                write_barrier_slot(self.module_bindings[index], nb.raw_bits());
                self.binding_write_raw(index, nb);
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
            let nb = self.pop_raw_u64()?;
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
            if let Some(arc) = raw_helpers::extract_shared_cell(self.module_bindings[index]) {
                let arc = arc.clone();
                let old = arc.read().unwrap().clone();
                record_heap_write();
                write_barrier_vw(&old, &truncated);
                *arc.write().unwrap() = truncated;
            } else {
                record_heap_write();
                write_barrier_slot(self.module_bindings[index], truncated.raw_bits());
                self.binding_write_raw(index, truncated);
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
            let is_cell = raw_helpers::extract_shared_cell(self.stack[slot]).is_some();

            if !is_cell {
                let old_bits = self.stack[slot];
                let value = self.stack_take_raw(slot);
                let cell_vw =
                    ValueWord::from_heap_value(HeapValue::SharedCell(Arc::new(RwLock::new(value))));
                record_heap_write();
                write_barrier_slot(old_bits, cell_vw.raw_bits());
                self.stack_write_raw(slot, cell_vw);
            }

            // Push the SharedCell onto the stack for MakeClosure to consume
            let nb = self.stack_read_raw(slot);
            self.push_raw_u64(nb)?;
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
            let is_cell = raw_helpers::extract_shared_cell(self.module_bindings[index]).is_some();

            if !is_cell {
                let old_bits = self.module_bindings[index];
                let value = self.binding_take_raw(index);
                let cell_vw =
                    ValueWord::from_heap_value(HeapValue::SharedCell(Arc::new(RwLock::new(value))));
                record_heap_write();
                write_barrier_slot(old_bits, cell_vw.raw_bits());
                self.binding_write_raw(index, cell_vw);
            }

            // Push the SharedCell onto the stack for MakeClosure to consume
            let nb = self.binding_read_raw(index);
            self.push_raw_u64(nb)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    // ── V1.1B: ownership-aware local opcodes ─────────────────────────────
    //
    // Phase 1 of the ownership-aware runtime spec introduces three new
    // opcodes — `MoveLocal`, `CloneLocal`, `DropLocal` — whose handlers
    // read the local slot bits directly and delegate any refcount
    // adjustment to `raw_helpers::clone_raw_bits` / `drop_raw_bits`. The
    // compiler will begin emitting them in V1.1C behind a flag; this
    // commit only wires up the executor side so hand-crafted bytecode
    // can exercise the semantics in isolation.
    //
    // Note: These opcodes assume the compiler has proved that the slot
    // contents obey normal ownership rules — no SharedCell wrapping, no
    // second read after a move. That guarantee is V1.1C's job. The
    // handlers here do not revalidate it at runtime; the poison state
    // left behind by `MoveLocal` is conceptual.

    /// `MoveLocal(idx)` — transfer ownership from the local slot onto the
    /// VM stack.
    ///
    /// Reads the raw u64 bits of the local slot and pushes them onto the
    /// stack with **no refcount adjustment**. This is the zero-cost
    /// ownership transfer described in phase 1 of
    /// `docs/ownership-aware-runtime-v2.md`: if the slot held a heap
    /// reference, the reference count is unchanged — the stack slot now
    /// owns that reference and the local slot is conceptually poisoned.
    ///
    /// The compiler (V1.1C) guarantees no subsequent read of a moved
    /// slot, so we leave the raw bits in the local slot as-is. A
    /// later `DropLocal` on the same slot would be a double-free; the
    /// compiler's liveness analysis must never emit that pair.
    ///
    /// Stack effect: +1 push. No atomic operations.
    pub(in crate::executor) fn op_move_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "MoveLocal slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            // SAFETY: compiler-allocated local slot inside the frame's
            // pre-sized register window.
            let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
            self.push_raw_u64(bits)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// `CloneLocal(idx)` — clone the value from the local slot onto the
    /// stack, leaving the local live.
    ///
    /// Reads the slot bits and delegates to `raw_helpers::clone_raw_bits`
    /// which handles the three cases:
    ///   * inline scalar (int, f64, bool, unit, null) — copy bits, no-op;
    ///   * shared heap reference (Arc-backed) — `Arc::increment_strong_count`;
    ///   * owned heap reference (Box-backed) — deep clone into a new
    ///     owned allocation via `vw_heap_box_owned`.
    ///
    /// After the call both the local slot and the pushed stack slot own
    /// independent references (or identical inline bits) — no poisoning.
    ///
    /// Stack effect: +1 push.
    pub(in crate::executor) fn op_clone_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "CloneLocal slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
            let cloned = raw_helpers::clone_raw_bits(bits);
            self.push_raw_u64(cloned)?;
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// `DropLocal(idx)` — release the value in the local slot in place.
    ///
    /// Reads the slot bits and delegates to `raw_helpers::drop_raw_bits`
    /// which handles:
    ///   * inline scalar — no-op;
    ///   * shared heap reference — `Arc::decrement_strong_count` (frees
    ///     if refcount hits zero);
    ///   * owned heap reference — immediate `Box::from_raw` drop.
    ///
    /// After the call the slot is overwritten with `0u64` to poison it.
    /// The compiler (V1.1C) guarantees no subsequent read of a dropped
    /// slot, so the poison value is never observed by well-formed
    /// bytecode.
    ///
    /// Stack effect: 0.
    pub(in crate::executor) fn op_drop_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Local(idx)) = instruction.operand {
            let bp = self.current_locals_base();
            let slot = bp + idx as usize;
            debug_assert!(
                slot < self.stack.len(),
                "DropLocal slot {} out of bounds (stack len {})",
                slot,
                self.stack.len()
            );
            let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
            // Release any refcount / owned-box the slot is holding.
            raw_helpers::drop_raw_bits(bits);
            // Poison the slot — the plan prescribes `0u64`. Well-formed
            // bytecode will never read this back.
            unsafe { *(self.stack.as_mut_ptr().add(slot) as *mut u64) = 0u64 };
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
    use shape_value::{ValueWord, ValueWordExt};

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
            Instruction::simple(OpCode::AddDynamic),
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

    // ===== PromoteToOwned tests =====

    #[test]
    fn test_promote_to_owned_string_becomes_owned() {
        // Push a string (Arc-backed heap), promote to owned, store, load back.
        // The value should still be a readable string.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("hello".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("hello".to_string()),
            "string value should survive PromoteToOwned"
        );
    }

    #[test]
    fn test_promote_to_owned_int_is_noop() {
        // Inline int (i48) should pass through PromoteToOwned unchanged.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(42));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_promote_to_owned_float_is_noop() {
        // Float (inline f64) should pass through PromoteToOwned unchanged.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Number(3.14));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_f64(), Some(3.14));
    }

    #[test]
    fn test_promote_to_owned_bool_is_noop() {
        // Bool (inline) should pass through PromoteToOwned unchanged.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Bool(true));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_promote_to_owned_null_is_noop() {
        // Null should pass through PromoteToOwned unchanged.
        let mut program = BytecodeProgram::default();
        program.instructions = vec![
            Instruction::simple(OpCode::PushNull),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::simple(OpCode::Halt),
        ];
        let result = run_program(program);
        assert!(result.is_none(), "null should survive PromoteToOwned");
    }

    // ===== V1.1B: MoveLocal / CloneLocal / DropLocal tests =====
    //
    // These hand-crafted programs exercise the new ownership-aware opcodes
    // in isolation. V1.1C will wire compiler emission; until then no user
    // program produces these opcodes, so these tests are the only
    // coverage path.
    //
    // Key subtlety: the VM's `Drop` impl walks `stack[0..sp]` and drops
    // every ValueWord. For top-level programs the local slots live
    // inside that range. `MoveLocal` is defined to NOT adjust
    // refcounts — the source slot retains the raw bits but is
    // conceptually poisoned. A real compiler (V1.1C) guarantees no
    // subsequent drop of the poisoned slot, but the VM's blanket Drop
    // doesn't know about that — so tests that exercise MoveLocal with a
    // heap value must explicitly poison the source slot afterwards via
    // `stack_take_raw` + `mem::forget` to avoid a double-free at VM
    // drop.

    /// Inspect the Arc strong count for a heap-tagged raw u64 bit pattern
    /// without modifying it. Returns 0 if the bits are not a shared
    /// (Arc-backed) heap reference.
    #[cfg(not(feature = "gc"))]
    fn strong_count_of(bits: u64) -> usize {
        use shape_value::heap_value::HeapValue;
        use shape_value::tag_bits::{
            get_payload, get_tag, is_tagged, HEAP_OWNED_BIT, HEAP_PTR_MASK, TAG_HEAP,
        };
        if !is_tagged(bits) || get_tag(bits) != TAG_HEAP {
            return 0;
        }
        let payload = get_payload(bits);
        if (payload & HEAP_OWNED_BIT) != 0 {
            return 0;
        }
        let ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
        if ptr.is_null() {
            return 0;
        }
        // SAFETY: pointer came from a live Arc-backed heap tag.
        let arc = std::mem::ManuallyDrop::new(unsafe { std::sync::Arc::from_raw(ptr) });
        std::sync::Arc::strong_count(&arc)
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_move_local_transfers_ownership_no_refcount() {
        // Put a heap string in slot 0 (rc=1). Execute MoveLocal 0: the
        // raw bits transfer onto the stack with NO refcount bump.
        // After Halt, the stack-top value is the sole live reference
        // (rc=1), and slot 0 still holds the bits it had before the
        // move — they are now "poisoned" per the V1.1C compiler
        // contract. The test poisons slot 0 explicitly via
        // `stack_take_raw` + `mem::forget` before VM drop so the
        // blanket drop path does not double-decrement.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("move-test".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            // stack=[s]  rc=1
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            // slot0=s, stack=[], rc=1
            Instruction::new(OpCode::MoveLocal, Some(Operand::Local(0))),
            // slot0=s (poisoned), stack=[s], rc=1 (UNCHANGED)
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("move-test".to_string()),
        );
        // Only one live reference — the stack-top value that became
        // the execute() result. Move did not bump the refcount.
        let count = strong_count_of(bits);
        assert_eq!(
            count, 1,
            "MoveLocal must not bump refcount (expected 1, got {})",
            count
        );
        // Poison slot 0 before VM drop — MoveLocal leaves stale bits,
        // and the VM's blanket Drop would otherwise double-decrement
        // the Arc. In real bytecode the V1.1C compiler guarantees no
        // subsequent read or drop of the moved slot; the test has to
        // simulate that invariant manually.
        std::mem::forget(vm.stack_take_raw(0));
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_clone_local_retains_heap_ref() {
        // Store a heap string in slot 0, then CloneLocal it onto the
        // stack. After the clone, both the local slot and the stack hold
        // a reference, so the Arc strong count must be 2.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("clone-test".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            // rc=1
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            // slot0=s, stack=[], rc=1
            Instruction::new(OpCode::CloneLocal, Some(Operand::Local(0))),
            // slot0=s, stack=[s], rc=2
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("clone-test".to_string()),
        );
        let count = strong_count_of(bits);
        assert_eq!(
            count, 2,
            "CloneLocal must bump refcount: local slot + stack = 2 (got {})",
            count
        );
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_drop_local_releases_heap_ref() {
        // Observe refcount transitions across DropLocal. Use CloneLocal
        // (not Dup) to create a second owning reference so the Arc
        // refcount actually reaches 2 — Dup just copies raw bits
        // without retaining, so a Dup'd pair shares one refcount slot
        // and dropping one would free the string the other still
        // "owns".
        //
        //   PushConst "s" → stack=[s]                 rc=1
        //   StoreLocal 0  → slot0=s, stack=[]         rc=1
        //   CloneLocal 0  → slot0=s, stack=[s]        rc=2 (retain)
        //   StoreLocal 1  → slot0=s, slot1=s, stack=[] rc=2
        //   CloneLocal 1  → slot1=s, stack=[s]        rc=3 (retain so the
        //                                                   observation read
        //                                                   at the end sees
        //                                                   a fresh ref)
        //   Halt
        //
        // After Halt the stack-top result is the cloned ref. Before the
        // observation we run another program branch: actually this test
        // has to fit in one program; we verify refcount after DropLocal
        // by observing from a CloneLocal reading slot 1.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("drop-test".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            // stack=[s]                                 rc=1
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            // slot0=s, stack=[]                         rc=1
            Instruction::new(OpCode::CloneLocal, Some(Operand::Local(0))),
            // slot0=s, stack=[s]                        rc=2
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
            // slot0=s, slot1=s, stack=[]                rc=2
            Instruction::new(OpCode::DropLocal, Some(Operand::Local(0))),
            // slot0=0, slot1=s, stack=[]                rc=1 (Arc dec)
            Instruction::new(OpCode::CloneLocal, Some(Operand::Local(1))),
            // slot0=0, slot1=s, stack=[s]               rc=2
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 2;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        let bits = result.raw_bits();
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("drop-test".to_string()),
        );
        // Slot 0 was dropped, slot 1 and the stack-top result both hold
        // live references — rc=2. If DropLocal had failed to decrement,
        // we'd observe rc=3.
        let count = strong_count_of(bits);
        assert_eq!(
            count, 2,
            "DropLocal should have decremented slot 0's refcount; observed {}",
            count
        );
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_drop_local_poisons_slot_to_zero() {
        // DropLocal must overwrite the slot with 0u64 after releasing it.
        // We verify by dropping slot 0 and then loading slot 0 back via
        // LoadLocalTrusted (which reads raw bits). The resulting value's
        // raw_bits must equal 0.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("poison-test".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::DropLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        assert_eq!(
            result.raw_bits(),
            0u64,
            "DropLocal must poison the slot to 0u64"
        );
    }

    #[test]
    fn test_move_local_on_inline_value_is_zero_cost() {
        // Inline int (i48 tagged). MoveLocal should push identical bits
        // onto the stack — no heap, no refcount. We can't directly
        // observe "no allocator activity", but the bit-for-bit match
        // between the pushed value and the stored constant is sufficient
        // evidence: `clone_raw_bits` would have returned the same bits,
        // but Move must not even branch into the heap path.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(42));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::MoveLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_drop_local_on_inline_is_noop() {
        // DropLocal on an int slot: the slot is poisoned to 0, no heap
        // activity. We verify the load-back yields 0 bits.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(7));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::DropLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;
        let result = run_program(program);
        assert_eq!(
            result.raw_bits(),
            0u64,
            "DropLocal on inline slot must still zero the slot"
        );
    }

    #[test]
    fn test_move_local_sequence_end_to_end() {
        // Move an inline int from slot 0 into slot 1 via the stack —
        // this is the common compiler-emitted pattern for a last-use
        // read + rebinding. For inline values it's safe to ignore the
        // post-move state of slot 0 since no refcount is held.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(11));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::MoveLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(1))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 2;
        let result = run_program(program);
        assert_eq!(result.as_i64(), Some(11));
    }

    #[test]
    #[cfg(not(feature = "gc"))]
    fn test_promote_to_owned_sets_owned_bit() {
        // After PromoteToOwned, a freshly allocated string should have the owned bit set.
        use crate::executor::VMConfig;
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("owned_test".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::PromoteToOwned),
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = 1;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        assert!(
            shape_value::ValueBits::from_raw(result).is_heap_owned(),
            "string after PromoteToOwned should have owned bit set"
        );
        assert_eq!(
            result.as_str().map(|s| s.to_string()),
            Some("owned_test".to_string()),
        );
    }

    // ── Track A.1B: interpreter handlers for OwnedMutable / Shared ─────
    //
    // These tests synthesise a tiny program consisting of exactly the new
    // opcode(s) under test, then prime a synthetic call frame whose
    // `upvalues` hold **raw pointer bits** produced by
    // `Box::into_raw` / `Arc::into_raw`. The opcode handler recovers the
    // pointer via `Upvalue::clone_inner_bits_for_raw_pointer_access` and
    // dereferences the cell. We drop the Box / Arc explicitly at end of
    // the test — `release_typed_closure` is NOT invoked because there is
    // no real closure block in this low-level harness.

    use crate::executor::CallFrame;
    use shape_value::Upvalue;
    use shape_value::v2::closure_layout::SharedCell;
    use std::sync::Arc as StdArc;

    /// Build a minimal `BytecodeProgram` whose top-level code is just
    /// `Halt` — the program is used to construct a VM, but we immediately
    /// replace the current IP / call stack to execute a short instruction
    /// sequence stored elsewhere.
    fn fresh_vm_for_capture_opcode_test() -> VirtualMachine {
        let mut program = BytecodeProgram::default();
        program.instructions = vec![Instruction::simple(OpCode::Halt)];
        program.top_level_locals_count = 0;
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm
    }

    /// Push a synthetic call frame with the given upvalues onto the VM
    /// call stack. The frame returns to ip=0 (Halt) on ReturnValue.
    fn push_synthetic_frame_with_upvalues(vm: &mut VirtualMachine, upvalues: Vec<Upvalue>) {
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: vm.sp,
            locals_count: 0,
            function_id: None,
            upvalues: Some(upvalues),
            blob_hash: None,
        });
    }

    #[test]
    fn a1b_load_owned_mutable_capture_derefs_box_cell() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        // Allocate a Box<ValueWord> — its raw pointer goes into the
        // synthetic upvalue slot. We reclaim the Box at the end of the
        // test.
        let cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(42)));
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        let instr = Instruction::new(
            OpCode::LoadOwnedMutableCapture,
            Some(Operand::Local(0)),
        );
        vm.op_load_owned_mutable_capture(&instr).unwrap();

        let out = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(out).as_i64(), Some(42));

        // Reclaim the cell manually — no release_typed_closure in this
        // harness.
        // SAFETY: `cell` came from Box::into_raw; we reclaim it exactly
        // once.
        unsafe {
            drop(Box::from_raw(cell));
        }
        // Pop the synthetic frame so the VM's own Drop doesn't trip on
        // the synthetic upvalue holding pointer bits.
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_store_owned_mutable_capture_writes_through_box_cell() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(1)));
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        // Push new value 999 onto the stack and StoreOwnedMutableCapture.
        vm.push_raw_u64(ValueWord::from_i64(999).into_raw_bits())
            .unwrap();
        let instr = Instruction::new(
            OpCode::StoreOwnedMutableCapture,
            Some(Operand::Local(0)),
        );
        vm.op_store_owned_mutable_capture(&instr).unwrap();

        // SAFETY: cell is still live. Read the cell back to confirm the
        // write landed.
        unsafe {
            assert_eq!((*cell).as_i64(), Some(999));
            drop(Box::from_raw(cell));
        }
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_owned_mutable_roundtrip_load_store_load() {
        // Full roundtrip: load initial value, store a new value, load
        // it back.
        let mut vm = fresh_vm_for_capture_opcode_test();
        let cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(10)));
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        let load_instr = Instruction::new(
            OpCode::LoadOwnedMutableCapture,
            Some(Operand::Local(0)),
        );
        let store_instr = Instruction::new(
            OpCode::StoreOwnedMutableCapture,
            Some(Operand::Local(0)),
        );

        // Load1 → 10
        vm.op_load_owned_mutable_capture(&load_instr).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(10)
        );

        // Store 55
        vm.push_raw_u64(ValueWord::from_i64(55).into_raw_bits())
            .unwrap();
        vm.op_store_owned_mutable_capture(&store_instr).unwrap();

        // Load2 → 55
        vm.op_load_owned_mutable_capture(&load_instr).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(55)
        );

        // SAFETY: reclaim.
        unsafe {
            drop(Box::from_raw(cell));
        }
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_load_shared_capture_locks_and_returns_inner() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let external: StdArc<SharedCell> =
            StdArc::new(parking_lot::Mutex::new(ValueWord::from_i64(77)));
        let closure_share: StdArc<SharedCell> = StdArc::clone(&external);
        let cell_ptr: *const SharedCell = StdArc::into_raw(closure_share);
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![Upvalue::new(cell_ptr as u64)],
        );

        let instr = Instruction::new(OpCode::LoadSharedCapture, Some(Operand::Local(0)));
        vm.op_load_shared_capture(&instr).unwrap();

        let out = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(out).as_i64(), Some(77));

        // Reclaim the Arc share for the synthetic upvalue.
        // SAFETY: cell_ptr came from Arc::into_raw — one strong share.
        unsafe {
            drop(StdArc::from_raw(cell_ptr));
        }
        assert_eq!(StdArc::strong_count(&external), 1);
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_store_shared_capture_writes_through_mutex() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let external: StdArc<SharedCell> =
            StdArc::new(parking_lot::Mutex::new(ValueWord::from_i64(0)));
        let closure_share: StdArc<SharedCell> = StdArc::clone(&external);
        let cell_ptr: *const SharedCell = StdArc::into_raw(closure_share);
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![Upvalue::new(cell_ptr as u64)],
        );

        // Push value 31415 and StoreSharedCapture.
        vm.push_raw_u64(ValueWord::from_i64(31415).into_raw_bits())
            .unwrap();
        let instr = Instruction::new(OpCode::StoreSharedCapture, Some(Operand::Local(0)));
        vm.op_store_shared_capture(&instr).unwrap();

        // External Arc observes the write.
        assert_eq!(external.lock().as_i64(), Some(31415));

        // SAFETY: reclaim.
        unsafe {
            drop(StdArc::from_raw(cell_ptr));
        }
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_two_closures_share_var_observe_writes_across_handles() {
        // Mandatory test #3 (A.1B brief): two closures capturing the
        // SAME Arc<SharedCell>. Write through closure A's upvalue,
        // observe via closure B's upvalue.
        let mut vm = fresh_vm_for_capture_opcode_test();
        let external: StdArc<SharedCell> =
            StdArc::new(parking_lot::Mutex::new(ValueWord::from_i64(0)));

        // Two raw-ptr shares — one per "closure".
        let share_a: *const SharedCell = StdArc::into_raw(StdArc::clone(&external));
        let share_b: *const SharedCell = StdArc::into_raw(StdArc::clone(&external));
        // Both raw pointers refer to the same underlying SharedCell —
        // Arc::into_raw on cloned Arcs yields identical `*const` values.
        assert_eq!(share_a, share_b);

        // Closure A's upvalues[0] is share_a. Closure B's is share_b.
        // Write through A, read through B.
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(share_a as u64)]);
        vm.push_raw_u64(ValueWord::from_i64(100).into_raw_bits())
            .unwrap();
        let store_instr = Instruction::new(OpCode::StoreSharedCapture, Some(Operand::Local(0)));
        vm.op_store_shared_capture(&store_instr).unwrap();
        vm.call_stack.pop();

        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(share_b as u64)]);
        let load_instr = Instruction::new(OpCode::LoadSharedCapture, Some(Operand::Local(0)));
        vm.op_load_shared_capture(&load_instr).unwrap();
        let out = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(out).as_i64(), Some(100));
        vm.call_stack.pop();

        // Strong count before reclaim: 3 (external + share_a + share_b).
        assert_eq!(StdArc::strong_count(&external), 3);
        // SAFETY: reclaim both raw pointer shares.
        unsafe {
            drop(StdArc::from_raw(share_a));
            drop(StdArc::from_raw(share_b));
        }
        assert_eq!(StdArc::strong_count(&external), 1);
    }

    #[test]
    fn a1b_mixed_kinds_interleaved_load_store_roundtrip() {
        // Mandatory test #4 (A.1B brief): frame with upvalues
        // [Immutable(F64), OwnedMutable, Shared, Immutable(I64)] and
        // round-trip reads/writes on all four through the appropriate
        // opcodes. Immutable slots use LoadClosure/StoreClosure; the
        // other two use A.1B's new opcodes.
        let mut vm = fresh_vm_for_capture_opcode_test();
        let box_cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(200)));
        let external: StdArc<SharedCell> =
            StdArc::new(parking_lot::Mutex::new(ValueWord::from_i64(300)));
        let arc_raw: *const SharedCell = StdArc::into_raw(StdArc::clone(&external));
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![
                Upvalue::new(ValueWord::from_f64(1.5).into_raw_bits()),
                Upvalue::new(box_cell as u64),
                Upvalue::new(arc_raw as u64),
                Upvalue::new(ValueWord::from_i64(400).into_raw_bits()),
            ],
        );

        // Immutable slot 0: LoadClosure.
        let load_imm_0 = Instruction::new(OpCode::LoadClosure, Some(Operand::Local(0)));
        vm.op_load_closure(&load_imm_0).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_f64(),
            Some(1.5)
        );

        // Immutable slot 3: LoadClosure.
        let load_imm_3 = Instruction::new(OpCode::LoadClosure, Some(Operand::Local(3)));
        vm.op_load_closure(&load_imm_3).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(400)
        );

        // OwnedMutable slot 1: Load → 200, Store 250, Load → 250.
        let load_om = Instruction::new(
            OpCode::LoadOwnedMutableCapture,
            Some(Operand::Local(1)),
        );
        let store_om = Instruction::new(
            OpCode::StoreOwnedMutableCapture,
            Some(Operand::Local(1)),
        );
        vm.op_load_owned_mutable_capture(&load_om).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(200)
        );
        vm.push_raw_u64(ValueWord::from_i64(250).into_raw_bits())
            .unwrap();
        vm.op_store_owned_mutable_capture(&store_om).unwrap();
        vm.op_load_owned_mutable_capture(&load_om).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(250)
        );

        // Shared slot 2: Load → 300, Store 350, Load → 350.
        let load_sh = Instruction::new(OpCode::LoadSharedCapture, Some(Operand::Local(2)));
        let store_sh = Instruction::new(OpCode::StoreSharedCapture, Some(Operand::Local(2)));
        vm.op_load_shared_capture(&load_sh).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(300)
        );
        vm.push_raw_u64(ValueWord::from_i64(350).into_raw_bits())
            .unwrap();
        vm.op_store_shared_capture(&store_sh).unwrap();
        vm.op_load_shared_capture(&load_sh).unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(350)
        );
        // External Arc reads the new value.
        assert_eq!(external.lock().as_i64(), Some(350));

        // SAFETY: reclaim Box + Arc share.
        unsafe {
            drop(Box::from_raw(box_cell));
            drop(StdArc::from_raw(arc_raw));
        }
        assert_eq!(StdArc::strong_count(&external), 1);
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_null_pointer_errors_cleanly() {
        // A null pointer in the upvalue slot (should never happen in
        // production — the A.1B make_closure path guarantees non-null
        // allocations) returns a runtime error instead of segfaulting.
        let mut vm = fresh_vm_for_capture_opcode_test();
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(0)]);

        let load_om = Instruction::new(
            OpCode::LoadOwnedMutableCapture,
            Some(Operand::Local(0)),
        );
        let err = vm.op_load_owned_mutable_capture(&load_om).unwrap_err();
        match err {
            shape_value::VMError::RuntimeError(msg) => {
                assert!(msg.contains("null"), "expected null-pointer error, got: {}", msg)
            }
            other => panic!("expected RuntimeError, got {:?}", other),
        }

        let load_sh = Instruction::new(OpCode::LoadSharedCapture, Some(Operand::Local(0)));
        let err = vm.op_load_shared_capture(&load_sh).unwrap_err();
        match err {
            shape_value::VMError::RuntimeError(msg) => {
                assert!(msg.contains("null"), "expected null-pointer error, got: {}", msg)
            }
            other => panic!("expected RuntimeError, got {:?}", other),
        }

        vm.call_stack.pop();
    }
}
