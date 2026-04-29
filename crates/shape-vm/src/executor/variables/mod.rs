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
use shape_value::value_word_drop::vw_drop;
use shape_value::{RefProjection, VMError, ValueWord, ValueWordExt};
use std::sync::Arc;
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
            // Wave E+3: typed local load/store opcodes (0x16C..=0x181).
            LoadLocalI64 => self.op_load_local_i64(instruction)?,
            LoadLocalU64 => self.op_load_local_u64(instruction)?,
            LoadLocalF64 => self.op_load_local_f64(instruction)?,
            LoadLocalI32 => self.op_load_local_i32(instruction)?,
            LoadLocalU32 => self.op_load_local_u32(instruction)?,
            LoadLocalI16 => self.op_load_local_i16(instruction)?,
            LoadLocalU16 => self.op_load_local_u16(instruction)?,
            LoadLocalI8 => self.op_load_local_i8(instruction)?,
            LoadLocalU8 => self.op_load_local_u8(instruction)?,
            LoadLocalBool => self.op_load_local_bool(instruction)?,
            LoadLocalPtr => self.op_load_local_ptr(instruction)?,
            StoreLocalI64 => self.op_store_local_i64(instruction)?,
            StoreLocalU64 => self.op_store_local_u64(instruction)?,
            StoreLocalF64 => self.op_store_local_f64(instruction)?,
            StoreLocalI32 => self.op_store_local_i32(instruction)?,
            StoreLocalU32 => self.op_store_local_u32(instruction)?,
            StoreLocalI16 => self.op_store_local_i16(instruction)?,
            StoreLocalU16 => self.op_store_local_u16(instruction)?,
            StoreLocalI8 => self.op_store_local_i8(instruction)?,
            StoreLocalU8 => self.op_store_local_u8(instruction)?,
            StoreLocalBool => self.op_store_local_bool(instruction)?,
            StoreLocalPtr => self.op_store_local_ptr(instruction)?,
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
            LoadOwnedMutableCapture => self.op_load_owned_mutable_capture(instruction)?,
            StoreOwnedMutableCapture => self.op_store_owned_mutable_capture(instruction)?,
            // D.1: typed OwnedMutable capture opcodes (0x140..=0x155).
            LoadOwnedMutableCaptureI64 => {
                self.op_load_owned_mutable_capture_i64(instruction)?
            }
            LoadOwnedMutableCaptureU64 => {
                self.op_load_owned_mutable_capture_u64(instruction)?
            }
            LoadOwnedMutableCaptureF64 => {
                self.op_load_owned_mutable_capture_f64(instruction)?
            }
            LoadOwnedMutableCaptureI32 => {
                self.op_load_owned_mutable_capture_i32(instruction)?
            }
            LoadOwnedMutableCaptureU32 => {
                self.op_load_owned_mutable_capture_u32(instruction)?
            }
            LoadOwnedMutableCaptureI16 => {
                self.op_load_owned_mutable_capture_i16(instruction)?
            }
            LoadOwnedMutableCaptureU16 => {
                self.op_load_owned_mutable_capture_u16(instruction)?
            }
            LoadOwnedMutableCaptureI8 => self.op_load_owned_mutable_capture_i8(instruction)?,
            LoadOwnedMutableCaptureU8 => self.op_load_owned_mutable_capture_u8(instruction)?,
            LoadOwnedMutableCaptureBool => {
                self.op_load_owned_mutable_capture_bool(instruction)?
            }
            LoadOwnedMutableCapturePtr => {
                self.op_load_owned_mutable_capture_ptr(instruction)?
            }
            StoreOwnedMutableCaptureI64 => {
                self.op_store_owned_mutable_capture_i64(instruction)?
            }
            StoreOwnedMutableCaptureU64 => {
                self.op_store_owned_mutable_capture_u64(instruction)?
            }
            StoreOwnedMutableCaptureF64 => {
                self.op_store_owned_mutable_capture_f64(instruction)?
            }
            StoreOwnedMutableCaptureI32 => {
                self.op_store_owned_mutable_capture_i32(instruction)?
            }
            StoreOwnedMutableCaptureU32 => {
                self.op_store_owned_mutable_capture_u32(instruction)?
            }
            StoreOwnedMutableCaptureI16 => {
                self.op_store_owned_mutable_capture_i16(instruction)?
            }
            StoreOwnedMutableCaptureU16 => {
                self.op_store_owned_mutable_capture_u16(instruction)?
            }
            StoreOwnedMutableCaptureI8 => {
                self.op_store_owned_mutable_capture_i8(instruction)?
            }
            StoreOwnedMutableCaptureU8 => {
                self.op_store_owned_mutable_capture_u8(instruction)?
            }
            StoreOwnedMutableCaptureBool => {
                self.op_store_owned_mutable_capture_bool(instruction)?
            }
            StoreOwnedMutableCapturePtr => {
                self.op_store_owned_mutable_capture_ptr(instruction)?
            }
            LoadSharedCapture => self.op_load_shared_capture(instruction)?,
            StoreSharedCapture => self.op_store_shared_capture(instruction)?,
            // D.2: typed Shared capture opcodes (0x156..=0x16B).
            LoadSharedCaptureI64 => self.op_load_shared_capture_i64(instruction)?,
            LoadSharedCaptureU64 => self.op_load_shared_capture_u64(instruction)?,
            LoadSharedCaptureF64 => self.op_load_shared_capture_f64(instruction)?,
            LoadSharedCaptureI32 => self.op_load_shared_capture_i32(instruction)?,
            LoadSharedCaptureU32 => self.op_load_shared_capture_u32(instruction)?,
            LoadSharedCaptureI16 => self.op_load_shared_capture_i16(instruction)?,
            LoadSharedCaptureU16 => self.op_load_shared_capture_u16(instruction)?,
            LoadSharedCaptureI8 => self.op_load_shared_capture_i8(instruction)?,
            LoadSharedCaptureU8 => self.op_load_shared_capture_u8(instruction)?,
            LoadSharedCaptureBool => self.op_load_shared_capture_bool(instruction)?,
            LoadSharedCapturePtr => self.op_load_shared_capture_ptr(instruction)?,
            StoreSharedCaptureI64 => self.op_store_shared_capture_i64(instruction)?,
            StoreSharedCaptureU64 => self.op_store_shared_capture_u64(instruction)?,
            StoreSharedCaptureF64 => self.op_store_shared_capture_f64(instruction)?,
            StoreSharedCaptureI32 => self.op_store_shared_capture_i32(instruction)?,
            StoreSharedCaptureU32 => self.op_store_shared_capture_u32(instruction)?,
            StoreSharedCaptureI16 => self.op_store_shared_capture_i16(instruction)?,
            StoreSharedCaptureU16 => self.op_store_shared_capture_u16(instruction)?,
            StoreSharedCaptureI8 => self.op_store_shared_capture_i8(instruction)?,
            StoreSharedCaptureU8 => self.op_store_shared_capture_u8(instruction)?,
            StoreSharedCaptureBool => self.op_store_shared_capture_bool(instruction)?,
            StoreSharedCapturePtr => self.op_store_shared_capture_ptr(instruction)?,
            AllocSharedLocal => self.op_alloc_shared_local(instruction)?,
            LoadSharedLocal => self.op_load_shared_local(instruction)?,
            StoreSharedLocal => self.op_store_shared_local(instruction)?,
            DropSharedLocal => self.op_drop_shared_local(instruction)?,
            AllocSharedModuleBinding => self.op_alloc_shared_module_binding(instruction)?,
            LoadSharedModuleBinding => self.op_load_shared_module_binding(instruction)?,
            StoreSharedModuleBinding => self.op_store_shared_module_binding(instruction)?,
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
                        // WB2.2 retain-on-read: `Upvalue::get()` bumps the
                        // refcount of heap-tagged captures, so the stack
                        // push receives an independent owning share. For
                        // raw pointer captures (`OwnedMutable` / `Shared`)
                        // the bits are not heap-tagged and `vw_clone` in
                        // `get()` is a no-op.
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
                        // WB2 retain-on-read: the write barrier reads the
                        // old value for inspection only — use `get_raw()`
                        // to avoid leaking a retain that has no matching
                        // release.
                        let old_raw = upvalue.get_raw();
                        write_barrier_vw(&old_raw, &value_nb);
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

    /// Resolve the `ClosureLayout` for the currently-executing closure
    /// frame (to consult `capture_inner_kind` for legacy
    /// `Load/StoreOwnedMutableCapture` dispatch).
    ///
    /// Returns `None` if the frame is not a closure call (no
    /// `function_id` registered) or the function has no layout (a
    /// non-closure function executing the legacy opcode is a compiler
    /// bug).
    #[inline]
    fn current_closure_layout(
        &self,
    ) -> Option<std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>> {
        let frame = self.call_stack.last()?;
        let func_id = frame.function_id?;
        self.program
            .closure_function_layouts
            .get(func_id as usize)
            .and_then(|l| l.as_ref())
            .cloned()
    }

    /// `LoadOwnedMutableCapture { idx }`: read the typed cell at
    /// capture `idx` and push the result onto the stack as a
    /// `ValueWord`.
    ///
    /// # Wave D (D.3) — type-aware dispatch
    ///
    /// The cell stores a native typed value matching
    /// `layout.capture_inner_kind(idx)` (an `i64`, `f64`, `bool`, …),
    /// allocated by `closure_raw::alloc_owned_mutable_<kind>` in
    /// `op_make_closure`. The legacy stack contract still requires
    /// pushing a `ValueWord`, so we read the typed value via Wave B's
    /// `read_owned_mutable_<kind>` helper and re-encode it. After Wave
    /// E flips the bytecode emitter to use D.1's typed
    /// `Load/StoreOwnedMutableCapture<Kind>` opcodes, this re-encoding
    /// boundary becomes dead code (cleaned up in Wave G).
    ///
    /// # Safety
    ///
    /// The capture at `idx` must have `CaptureKind::OwnedMutable` and
    /// the upvalue slot must contain a non-null pointer obtained from
    /// the matching `closure_raw::alloc_owned_mutable_<kind>` for
    /// `capture_inner_kind(idx)`. The pointer is valid for the
    /// closure's refcounted lifetime; it is released exactly once by
    /// `release_typed_closure` via the matching `Box::from_raw`.
    fn op_load_owned_mutable_capture(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_raw::{
            read_owned_mutable_bool, read_owned_mutable_f64, read_owned_mutable_i8,
            read_owned_mutable_i16, read_owned_mutable_i32, read_owned_mutable_i64,
            read_owned_mutable_ptr, read_owned_mutable_u8, read_owned_mutable_u16,
            read_owned_mutable_u32, read_owned_mutable_u64,
        };
        use shape_value::v2::struct_layout::FieldKind;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let layout = self.current_closure_layout().ok_or_else(|| {
            VMError::RuntimeError(
                "LoadOwnedMutableCapture without registered ClosureLayout".to_string(),
            )
        })?;
        let kind = layout.capture_inner_kind(idx as usize);
        // SAFETY: `cell_ptr` was produced by the matching
        // `closure_raw::alloc_owned_mutable_<kind>(initial)` in
        // `op_make_closure`. The interior FieldKind is determined by
        // `layout.capture_inner_kind(idx)`; reading via the matching
        // typed helper is in-bounds and aligned. The closure's
        // refcounted block keeps the box alive for the duration of
        // this handler invocation.
        let vw_bits: u64 = unsafe {
            match kind {
                FieldKind::I64 => ValueWord::from_i64(read_owned_mutable_i64(cell_ptr as *mut i64)),
                FieldKind::U64 => {
                    let v = read_owned_mutable_u64(cell_ptr as *mut u64);
                    // u64 → ValueWord via `from_i64`; values > i64::MAX
                    // round-trip through BigInt via the existing
                    // i48-overflow path (matches `write_capture_typed`'s
                    // U64 lossy semantics).
                    ValueWord::from_i64(v as i64)
                }
                FieldKind::F64 => ValueWord::from_f64(read_owned_mutable_f64(cell_ptr as *mut f64)),
                FieldKind::I32 => {
                    ValueWord::from_i64(read_owned_mutable_i32(cell_ptr as *mut i32) as i64)
                }
                FieldKind::U32 => {
                    ValueWord::from_i64(read_owned_mutable_u32(cell_ptr as *mut u32) as i64)
                }
                FieldKind::I16 => {
                    ValueWord::from_i64(read_owned_mutable_i16(cell_ptr as *mut i16) as i64)
                }
                FieldKind::U16 => {
                    ValueWord::from_i64(read_owned_mutable_u16(cell_ptr as *mut u16) as i64)
                }
                FieldKind::I8 => {
                    ValueWord::from_i64(read_owned_mutable_i8(cell_ptr as *mut i8) as i64)
                }
                FieldKind::U8 => {
                    ValueWord::from_i64(read_owned_mutable_u8(cell_ptr as *mut u8) as i64)
                }
                FieldKind::Bool => {
                    ValueWord::from_bool(read_owned_mutable_bool(cell_ptr as *mut bool))
                }
                FieldKind::Ptr => {
                    // Pass-through: the cell holds the raw 8-byte
                    // heap-pointer bit pattern (an Arc share). Reading
                    // it without a clone delegates the share-management
                    // contract to the caller exactly as the pre-D.3
                    // path did — the load does NOT bump the refcount.
                    read_owned_mutable_ptr(cell_ptr as *mut u64)
                }
            }
        };
        self.push_raw_u64(vw_bits)
    }

    /// `StoreOwnedMutableCapture { idx }`: pop a ValueWord and write it
    /// through the typed cell behind capture `idx`.
    ///
    /// # Wave D (D.3) — type-aware dispatch
    ///
    /// Symmetric counterpart to `op_load_owned_mutable_capture`: the
    /// popped ValueWord is decoded to the cell's native interior type
    /// before writing via Wave B's `write_owned_mutable_<kind>` helper.
    /// The decode strategy mirrors `op_make_closure`'s OwnedMutable
    /// allocation path (`vw.as_i64()` / `vw.as_number_coerce()` /
    /// `vw.as_bool()` / `vw.as_u64_value()`) so a
    /// `Load → Store → Load` round-trip preserves the value.
    ///
    /// # Safety
    ///
    /// Same invariants as `op_load_owned_mutable_capture`. For
    /// `FieldKind::Ptr` the previous cell payload is a heap-refcount
    /// share — this handler releases the old share via `vw_drop` before
    /// writing the new bits, mirroring the immutable-Ptr drop semantics
    /// `release_typed_closure` enforces on closure teardown.
    fn op_store_owned_mutable_capture(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_raw::{
            read_owned_mutable_ptr, write_owned_mutable_bool, write_owned_mutable_f64,
            write_owned_mutable_i8, write_owned_mutable_i16, write_owned_mutable_i32,
            write_owned_mutable_i64, write_owned_mutable_ptr, write_owned_mutable_u8,
            write_owned_mutable_u16, write_owned_mutable_u32, write_owned_mutable_u64,
        };
        use shape_value::v2::struct_layout::FieldKind;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let layout = self.current_closure_layout().ok_or_else(|| {
            VMError::RuntimeError(
                "StoreOwnedMutableCapture without registered ClosureLayout".to_string(),
            )
        })?;
        let kind = layout.capture_inner_kind(idx as usize);
        record_heap_write();
        let vw: ValueWord = new_bits;
        // SAFETY: `cell_ptr` was produced by the matching
        // `closure_raw::alloc_owned_mutable_<kind>(initial)` in
        // `op_make_closure`; the interior FieldKind is determined by
        // `layout.capture_inner_kind(idx)`. Each write helper performs
        // an aligned in-bounds store into the typed `Box<T>`.
        unsafe {
            match kind {
                FieldKind::I64 => write_owned_mutable_i64(
                    cell_ptr as *mut i64,
                    vw.as_i64().unwrap_or(0),
                ),
                FieldKind::U64 => write_owned_mutable_u64(
                    cell_ptr as *mut u64,
                    vw.as_u64_value().unwrap_or(0),
                ),
                FieldKind::F64 => write_owned_mutable_f64(
                    cell_ptr as *mut f64,
                    vw.as_number_coerce().unwrap_or(0.0),
                ),
                FieldKind::I32 => write_owned_mutable_i32(
                    cell_ptr as *mut i32,
                    vw.as_i64().unwrap_or(0) as i32,
                ),
                FieldKind::U32 => write_owned_mutable_u32(
                    cell_ptr as *mut u32,
                    vw.as_i64().unwrap_or(0) as u32,
                ),
                FieldKind::I16 => write_owned_mutable_i16(
                    cell_ptr as *mut i16,
                    vw.as_i64().unwrap_or(0) as i16,
                ),
                FieldKind::U16 => write_owned_mutable_u16(
                    cell_ptr as *mut u16,
                    vw.as_i64().unwrap_or(0) as u16,
                ),
                FieldKind::I8 => write_owned_mutable_i8(
                    cell_ptr as *mut i8,
                    vw.as_i64().unwrap_or(0) as i8,
                ),
                FieldKind::U8 => write_owned_mutable_u8(
                    cell_ptr as *mut u8,
                    vw.as_i64().unwrap_or(0) as u8,
                ),
                FieldKind::Bool => write_owned_mutable_bool(
                    cell_ptr as *mut bool,
                    vw.as_bool().unwrap_or(false),
                ),
                FieldKind::Ptr => {
                    // Release the previous heap-refcount share before
                    // overwriting the cell, mirroring the immutable-Ptr
                    // drop-on-replace semantics that
                    // `release_typed_closure` enforces on closure
                    // teardown.
                    let prev = read_owned_mutable_ptr(cell_ptr as *mut u64);
                    vw_drop(prev);
                    write_owned_mutable_ptr(cell_ptr as *mut u64, new_bits);
                }
            }
        }
        Ok(())
    }

    // ── Phase 3c Wave D.1: per-FieldKind typed OwnedMutable capture handlers ──
    //
    // Each handler resolves the upvalue's raw `*mut <T>` cell pointer and
    // delegates to the per-FieldKind `read_owned_mutable_<kind>` /
    // `write_owned_mutable_<kind>` helper in `shape_value::v2::closure_raw`.
    // The cell holds a native scalar (i64/u64/f64/i32/u32/i16/u16/i8/u8/
    // bool/ptr-shaped u64) without any tag overhead — the typed counterpart
    // of the legacy single-form `op_load_owned_mutable_capture` /
    // `op_store_owned_mutable_capture` (which assume `*mut ValueWord`).
    //
    // Stack convention: 8-byte register-shaped values. Sub-i64 ints are
    // sign- or zero-extended into the 8-byte slot via `push_raw_u64`,
    // matching the existing typed-opcode convention (see
    // `op_store_local_typed`, the typed Shared handlers in D.2, and the
    // C.2 JIT FFI lowering in `crates/shape-jit/src/ffi/object/closure.rs`).
    // Stores pop the 8-byte slot via `pop_raw_u64` and truncate to the
    // declared width before writing the cell.
    //
    // SAFETY (common to every handler below):
    //   * `cell_ptr` is the raw u64 bits placed in `frame.upvalues[idx]`
    //     by `op_make_closure`. It was minted via
    //     `Box::into_raw(Box::new(initial))` for the matching native type
    //     by `closure_raw::alloc_owned_mutable_<kind>`. Exactly one
    //     closure owns the box; no aliasing or sharing semantics apply.
    //   * The compiler emits the typed opcode only when the capture's
    //     `CaptureKind` is `OwnedMutable` and the cell's interior
    //     `FieldKind` matches `<Kind>`. Mismatches are a compiler bug.
    //   * Null-check the pointer up-front so a layout/encoding bug
    //     surfaces as a clean runtime error rather than a UB read.

    fn op_load_owned_mutable_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: see common invariants on the section header.
        // `read_owned_mutable_i64` performs an aligned 8-byte load.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i64(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_owned_mutable_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u64(cell_ptr) };
        self.push_raw_u64(value)
    }

    fn op_load_owned_mutable_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut f64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_f64(cell_ptr) };
        self.push_raw_u64(value.to_bits())
    }

    fn op_load_owned_mutable_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Sign-extend i32 to i64
        // for the 8-byte stack slot.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i32(cell_ptr) };
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_owned_mutable_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Zero-extend u32 to u64.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u32(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_owned_mutable_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Sign-extend i16 to i64.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i16(cell_ptr) };
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_owned_mutable_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Zero-extend u16 to u64.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u16(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_owned_mutable_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Sign-extend i8 to i64.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i8(cell_ptr) };
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_owned_mutable_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Zero-extend u8 to u64.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u8(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_owned_mutable_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut bool;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. Push the bool as a 0/1
        // u64 — matches D.2's typed-bool convention; Wave E will rewire to
        // `push_raw_bool` once typed-bool stack helpers are unified.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_bool(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_owned_mutable_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: section header invariants apply. The 8-byte payload is
        // a ValueWord bit pattern carrying a NaN-boxed Arc/Box pointer.
        // The helper does NOT clone/retain — refcount semantics are the
        // caller's responsibility. Wave E will pair the Load with
        // `vw_clone` / `vw_drop` at the IR level (matches the
        // c-stdlib-msgpack pattern from commit afb1651, mirroring
        // `op_load_shared_capture_ptr` in D.2).
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_ptr(cell_ptr) };
        self.push_raw_u64(value)
    }

    fn op_store_owned_mutable_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i64;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = f64::from_bits(self.pop_raw_u64()?);
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut f64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_f64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop the 8-byte slot and truncate to i32 (low 32 bits) — matches
        // the typed-opcode truncation convention in `op_store_local_typed`.
        let new_value = self.pop_raw_u64()? as i32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Treat any nonzero bit pattern as true — mirrors D.2's
        // `op_store_shared_capture_bool` convention.
        let new_value = self.pop_raw_u64()? != 0;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut bool;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_bool(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply. `write_owned_mutable_ptr`
        // does NOT release the previous payload nor retain the new one
        // (matches `read_owned_mutable_ptr` symmetry). Refcount management
        // is the responsibility of the compiler / IR (Wave E).
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_ptr(cell_ptr, new_bits) };
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

    // ── Track D.2: per-FieldKind typed Shared capture handlers ─────────
    //
    // Each handler resolves the upvalue's raw `*const SharedCell` bits and
    // delegates to the lock-gated `read_shared_<kind>` /
    // `write_shared_<kind>` helper in `shape_value::v2::closure_raw`.
    // Critical invariant: the helpers acquire the cell's
    // `parking_lot::Mutex` internally, so the handler MUST NOT take the
    // lock externally — that would deadlock on a non-reentrant mutex.
    //
    // The pushed/popped 8-byte stack value is the raw native bit pattern
    // matching the cell's interior `FieldKind`. For widths < 8 bytes the
    // helper sign-/zero-extends to a 64-bit register-shaped value before
    // the handler stores those bits via `push_raw_u64`. The compiler
    // emitter (Wave E) and the JIT FFI lowering (C.2) already share this
    // convention — see `crates/shape-jit/src/ffi/object/closure.rs:660`+.
    //
    // SAFETY (common to every handler below):
    //   * `cell_ptr` is the raw u64 bits placed in `frame.upvalues[idx]`
    //     by `op_make_closure`. It was minted via
    //     `Arc::into_raw(Arc::new(SharedCell::new(...)))`; the
    //     surrounding closure block owns one strong-count share which
    //     keeps the cell alive for this call frame's lifetime.
    //   * The compiler emits the typed opcode only when the capture's
    //     `CaptureKind` is `Shared` and the cell's interior `FieldKind`
    //     matches. Mismatches are a compiler bug.

    fn op_load_shared_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: see common invariants on the section header.
        // `read_shared_i64` acquires the parking_lot mutex internally,
        // performs an aligned 8-byte load, releases, and returns. We
        // must NOT take the lock externally.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i64(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_shared_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u64(cell_ptr) };
        self.push_raw_u64(value)
    }

    fn op_load_shared_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_f64(cell_ptr) };
        self.push_raw_u64(value.to_bits())
    }

    fn op_load_shared_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. `read_shared_i32`
        // returns a 4-byte value sign-extended (by the writer) from the
        // 8-byte payload; we sign-extend to i64 here to fit the
        // 8-byte stack slot.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i32(cell_ptr) };
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_shared_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. Zero-extend the u32
        // to u64 for the 8-byte stack slot.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u32(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_shared_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. Sign-extend i16 to
        // i64 for the 8-byte stack slot.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i16(cell_ptr) };
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_shared_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. Zero-extend.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u16(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_shared_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. Sign-extend i8 to
        // i64.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i8(cell_ptr) };
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_shared_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. Zero-extend u8 to
        // u64.
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u8(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_shared_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. The helper reads the
        // low byte of the payload (0 ⇒ false; non-zero ⇒ true). We
        // store the resulting bool on the stack as a 0 or 1 in the
        // 8-byte slot via `push_raw_u64` to keep the slot's bit pattern
        // unambiguously typed for the typed-Bool reader (Wave E /
        // future JIT lowering will rewire to `push_raw_bool` once
        // typed-bool stack helpers are unified).
        let value = unsafe { shape_value::v2::closure_raw::read_shared_bool(cell_ptr) };
        self.push_raw_u64(value as u64)
    }

    fn op_load_shared_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. The 8-byte payload
        // is a ValueWord bit pattern carrying a NaN-boxed Arc/Box
        // pointer. The helper does NOT clone/retain — refcount
        // semantics are the caller's responsibility. Wave E will pair
        // the Load with `vw_clone` / `vw_drop` at the IR level
        // (matching the c-stdlib-msgpack pattern from commit afb1651).
        let value = unsafe { shape_value::v2::closure_raw::read_shared_ptr(cell_ptr) };
        self.push_raw_u64(value)
    }

    fn op_store_shared_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i64;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply. `write_shared_i64`
        // takes the lock internally — DO NOT take it here.
        unsafe { shape_value::v2::closure_raw::write_shared_i64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_u64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = f64::from_bits(self.pop_raw_u64()?);
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_f64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop as raw u64; truncate to i32 (the low 4 bytes carry the
        // signed value, matching the JIT's `i32` ABI).
        let new_value = self.pop_raw_u64()? as i64 as i32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply. `write_shared_i32`
        // sign-extends the value to 8 bytes inside the cell.
        unsafe { shape_value::v2::closure_raw::write_shared_i32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_u32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i64 as i16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_i16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_u16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i64 as i8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_i8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_u8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop the 8-byte slot; treat any nonzero bit pattern as true to
        // mirror `read_shared_bool`'s "any non-zero byte" semantics.
        let new_value = self.pop_raw_u64()? != 0;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: section header invariants apply.
        unsafe { shape_value::v2::closure_raw::write_shared_bool(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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
        // SAFETY: section header invariants apply. `write_shared_ptr`
        // does NOT release the previous payload nor retain the new one
        // (matches `read_shared_ptr` symmetry). Refcount management is
        // the responsibility of the compiler / IR (see Wave E plan).
        unsafe { shape_value::v2::closure_raw::write_shared_ptr(cell_ptr, new_bits) };
        Ok(())
    }

    // ── Track A.1C.1: outer-scope `var` Shared-cell lifecycle ──────────
    //
    // `AllocSharedLocal` / `LoadSharedLocal` / `StoreSharedLocal` /
    // `DropSharedLocal` are the outer-scope counterparts of A.1B's
    // `LoadSharedCapture` / `StoreSharedCapture`. A.1B operates on the
    // raw `*const SharedCell` pointer bits held in a **closure's upvalue
    // slot**. A.1C.1 operates on the raw `*const SharedCell` pointer
    // bits held in the declaring frame's **local stack slot**.
    //
    // Shared contract with A.1B: both sides treat the raw u64 in the
    // slot as an `Arc::into_raw`-produced pointer. `AllocSharedLocal`
    // produces exactly one strong-count share and parks it in the local
    // slot; `DropSharedLocal` releases exactly that share via
    // `Arc::from_raw`. Additional strong shares (one per capturing
    // closure) are minted by the closure-build path (A.1B
    // `op_make_closure`) via `Arc::increment_strong_count` — those are
    // reclaimed by `release_typed_closure` on closure drop, completely
    // independent of the outer-scope lifecycle handled here.
    //
    // SAFETY invariants for every handler below:
    //
    // - The compiler (A.1C.2, still pending) emits these opcodes only
    //   on slots whose `BindingStorageClass` is `Shared`. Until that
    //   lands, only unit tests and hand-assembled bytecode exercise
    //   these opcodes.
    // - The slot's raw u64 is produced exclusively by
    //   `AllocSharedLocal`. It is the sole allocator. Any other writer
    //   (e.g. `StoreLocal` targeting the same slot) violates the
    //   Shared contract — the compiler must never emit that mix.
    // - The pointer is released exactly once by `DropSharedLocal`.
    //   After Drop, the slot is overwritten with `NONE_BITS` to mark
    //   it spent; re-reading the slot via `LoadSharedLocal` /
    //   `StoreSharedLocal` after Drop is a compiler bug (reports a
    //   null-pointer runtime error at the interpreter level).
    // - Concurrent access across closures is permitted: each closure
    //   holds its own `Arc` strong share and takes the parking_lot
    //   mutex for lock-gated read/write. The mutex is the sole legal
    //   read/write path — interpreter code never dereferences the
    //   pointer without first taking the lock.

    /// `AllocSharedLocal { slot }`: pop the initial value, allocate a
    /// fresh `Arc<SharedCell>`, write the `Arc::into_raw` pointer bits
    /// into local slot `slot`.
    ///
    /// # Safety
    ///
    /// This is the sole allocator for a Shared local slot. The produced
    /// `*const SharedCell` pointer is:
    ///   * 8-byte aligned (Rust global allocator + `parking_lot::Mutex`'s
    ///     alignment).
    ///   * Non-null (Rust's `Arc::new` never returns null).
    ///   * Owned by slot `slot` — exactly one strong-count share is
    ///     parked in the slot.
    ///
    /// The slot is treated as opaque raw pointer bits; it MUST NOT be
    /// read via `LoadLocal` / `StoreLocal` until after `DropSharedLocal`
    /// overwrites it with `NONE_BITS`. Any previous occupant of the
    /// slot is silently overwritten — the compiler guarantees the slot
    /// is freshly introduced (not reused) when this opcode is emitted.
    fn op_alloc_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        use std::sync::Arc as StdArc;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop the initial ValueWord bits from the stack — these become
        // the inner payload of the new mutex.
        let initial_bits = self.pop_raw_u64()?;

        // Allocate the Arc<SharedCell> and convert into a raw pointer.
        // `Arc::into_raw` keeps one strong-count share alive;
        // `DropSharedLocal` is responsible for reclaiming it later.
        //
        // A.1E: SharedCell is now a `#[repr(C)]` struct with a hand-rolled
        // spinlock at offset 0 and the ValueWord payload at offset 8. The
        // JIT's inline lock/unlock paths depend on this fixed layout —
        // see `closure_layout.rs::SharedCell` for the SAFETY contract.
        let arc: StdArc<SharedCell> = StdArc::new(SharedCell::new(initial_bits));
        let cell_ptr: *const SharedCell = StdArc::into_raw(arc);

        // Compute the absolute stack index for the requested local slot.
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }

        // Park the raw pointer bits in the slot. We intentionally do NOT
        // route this write through `stack_write_raw` — that helper
        // interprets the old slot bits as a `ValueWord` and drops them
        // (which would double-free if the slot previously held a
        // Shared-cell pointer). The compiler contract is: the slot is
        // freshly introduced, so the old bits are `NONE_BITS` (or
        // uninitialised — still inline, no Drop glue).
        record_heap_write();
        self.stack[slot] = cell_ptr as u64;
        Ok(())
    }

    /// `LoadSharedLocal { slot }`: read the `*const SharedCell` bits
    /// from local slot `slot`, acquire the mutex for a read, clone the
    /// inner ValueWord bits, drop the guard, push the value onto the
    /// stack.
    ///
    /// # Safety
    ///
    /// The slot at `slot` must hold non-null `*const SharedCell` bits
    /// that were produced by a prior `AllocSharedLocal` on the same
    /// slot and not yet consumed by `DropSharedLocal`. The reborrow
    /// via `&*cell_ptr` is scoped to this handler invocation; the Arc
    /// strong-count share stays with the slot (this handler does not
    /// retain/release). The mutex is the sole legal read path — no
    /// reader bypasses the lock.
    fn op_load_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "LoadSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        let bits = self.stack[slot];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "LoadSharedLocal: Shared local pointer is null (not initialised or already dropped)"
                    .to_string(),
            ));
        }
        // SAFETY: `cell_ptr` was produced by `Arc::into_raw(Arc::new(...))`
        // in `op_alloc_shared_local` and the Arc strong share owned by
        // this slot keeps the allocation alive. We reborrow via
        // `&*cell_ptr` for the duration of this handler only; the
        // reference does not escape. The mutex mediates concurrency
        // with any capturing closure's `LoadSharedCapture` /
        // `StoreSharedCapture` as well as any other
        // `LoadSharedLocal` / `StoreSharedLocal` on this slot.
        let value_bits = unsafe {
            let cell: &SharedCell = &*cell_ptr;
            let guard = cell.lock();
            let bits = *guard;
            drop(guard);
            bits
        };
        self.push_raw_u64(value_bits)
    }

    /// `StoreSharedLocal { slot }`: pop a ValueWord, read the
    /// `*const SharedCell` bits from local slot `slot`, acquire the
    /// mutex for a write, overwrite the inner ValueWord bits, drop the
    /// guard. The slot's pointer bits are NOT modified.
    ///
    /// # Safety
    ///
    /// Same invariants as `op_load_shared_local`. The mutex serialises
    /// concurrent writers from other closures / the declaring frame.
    /// The Arc strong-count share owned by the slot is untouched.
    fn op_store_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "StoreSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        let bits = self.stack[slot];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "StoreSharedLocal: Shared local pointer is null (not initialised or already dropped)"
                    .to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: same invariants as `op_load_shared_local`. We take
        // the mutex for exclusive access, overwrite the 8-byte
        // ValueWord payload, then drop the guard. The slot's pointer
        // bits stay exactly as `op_alloc_shared_local` installed them —
        // this write only touches the interior of the mutex.
        unsafe {
            let cell: &SharedCell = &*cell_ptr;
            let mut guard = cell.lock();
            *guard = new_bits;
            drop(guard);
        }
        Ok(())
    }

    /// `DropSharedLocal { slot }`: read the `*const SharedCell` bits
    /// from local slot `slot`, reconstruct `Arc::from_raw`, drop the
    /// Arc (one atomic strong-count decrement), then overwrite the slot
    /// with the null pointer (`0u64`) to mark it spent.
    ///
    /// # Safety
    ///
    /// This is the sole releaser for the outer-scope Arc strong share
    /// allocated by `AllocSharedLocal`. The pointer at slot `slot` must
    /// have been installed by a prior `AllocSharedLocal` on the same
    /// slot and not yet consumed. Double-drop is a compiler bug and
    /// would be a use-after-free on the second call — the null-pointer
    /// guard at least prevents segfaulting; it does not recover
    /// correctness.
    ///
    /// The slot is rewritten to `0u64` (a genuine null pointer — NOT a
    /// NaN-tagged ValueWord) so any subsequent `LoadSharedLocal` /
    /// `StoreSharedLocal` on this slot reports a null-pointer runtime
    /// error rather than silently dereferencing freed memory. The VM's
    /// top-level `Drop` invokes `ValueWord::from_raw_bits(0).drop()`,
    /// which is a no-op (bit pattern `0` decodes as a heap-tagged
    /// pointer whose payload is null; the ValueWord drop glue checks
    /// for null before any refcount traffic — see
    /// `shape_value::value_word`).
    fn op_drop_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        use std::sync::Arc as StdArc;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "DropSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        let bits = self.stack[slot];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "DropSharedLocal: Shared local pointer is null (not initialised or already dropped)"
                    .to_string(),
            ));
        }
        // Mark the slot spent BEFORE reclaiming the Arc so any
        // reentrant access on this slot sees a genuine null pointer
        // rather than dangling bits. We use `0u64` (not `NONE_BITS`)
        // because the Load/Store/Drop handlers perform a raw
        // `cell_ptr.is_null()` check — `NONE_BITS` is a non-zero
        // NaN-tagged sentinel and would bypass that check and lead to
        // a spurious dereference. The VM's top-level `Drop` treats
        // bits=0 as a heap-tagged-null ValueWord whose drop is a no-op.
        self.stack[slot] = 0u64;
        // SAFETY: `cell_ptr` was produced by
        // `Arc::into_raw(Arc::new(...))` in `op_alloc_shared_local` and
        // this call site is the unique drop point for that share.
        // `Arc::from_raw` transfers ownership back into the Arc handle,
        // and the `drop` call decrements the strong count. Any
        // additional closures holding capture-side shares retain their
        // own independent strong counts, so the underlying
        // `SharedCell` stays alive as long as at least one strong
        // share exists.
        unsafe {
            drop(StdArc::from_raw(cell_ptr));
        }
        Ok(())
    }

    // ── Track A.1C.3: Shared module-binding opcode handlers ────────────
    //
    // Module-binding parallel to the Shared local opcodes above. The
    // addressing mode is `Operand::ModuleBinding(idx)` instead of
    // `Operand::Local(idx)`; the underlying Arc / mutex mechanics are
    // identical. Released once, at VM drop, via
    // `shared_module_bindings` tracking.

    /// `AllocSharedModuleBinding { idx }`: pop a ValueWord as the
    /// initial value, allocate a fresh `Arc<parking_lot::Mutex<ValueWord>>`,
    /// and store the `Arc::into_raw` pointer bits into
    /// `module_bindings[idx]`. Registers `idx` in
    /// `self.shared_module_bindings` so VM drop reclaims the Arc.
    ///
    /// # Safety
    ///
    /// Sole allocator for Shared module bindings. The compiler emits
    /// this exactly once per promoted module-binding slot (before any
    /// closure capture of that slot). After this opcode, every read
    /// and write to the slot must use `LoadSharedModuleBinding` /
    /// `StoreSharedModuleBinding`; plain `LoadModuleBinding` /
    /// `StoreModuleBinding` would read raw pointer bits as a ValueWord
    /// (silent corruption). The slot's prior ValueWord is dropped as
    /// part of `binding_take_raw`.
    fn op_alloc_shared_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        use std::sync::Arc as StdArc;

        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        // Ensure the slot exists.
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        // Pop the initial ValueWord bits. The stack slot is released
        // (caller's responsibility: the compiler emitted
        // `LoadModuleBinding` → `AllocSharedModuleBinding`, transferring
        // ownership of the current ValueWord to the stack).
        let initial_bits = self.pop_raw_u64()?;

        // Drop the existing ValueWord in the slot before installing raw
        // pointer bits. This is important if the slot already held e.g.
        // a heap pointer — those bits must not be reinterpreted as a
        // raw SharedCell pointer.
        let old_bits = self.module_bindings[index];
        self.module_bindings[index] = Self::NONE_BITS;
        // FR.3: real release (was no-op drop of Copy u64).
        vw_drop(old_bits);

        // Allocate the Arc<SharedCell>. A.1E: SharedCell is a
        // `#[repr(C)]` hand-rolled-spinlock struct, see
        // `closure_layout.rs::SharedCell`.
        let arc: StdArc<SharedCell> = StdArc::new(SharedCell::new(initial_bits));
        let cell_ptr: *const SharedCell = StdArc::into_raw(arc);

        // Install raw pointer bits. Intentionally NOT via
        // `binding_write_raw` (which would drop the prior ValueWord as
        // if it were a ValueWord — we already did that above, and we
        // do NOT want the raw pointer bits routed through ValueWord
        // drop glue on any future write).
        record_heap_write();
        self.module_bindings[index] = cell_ptr as u64;
        self.shared_module_bindings.insert(index);
        Ok(())
    }

    /// `LoadSharedModuleBinding { idx }`: read the `*const SharedCell`
    /// bits from `module_bindings[idx]`, acquire the mutex for a read,
    /// clone the inner ValueWord bits, drop the guard, push onto the
    /// stack.
    ///
    /// # Safety
    ///
    /// The slot must hold non-null `*const SharedCell` bits installed
    /// by a prior `AllocSharedModuleBinding` on the same slot. The Arc
    /// strong-count share owned by the slot keeps the allocation alive
    /// for the VM's lifetime.
    fn op_load_shared_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        if index >= self.module_bindings.len() {
            return Err(VMError::RuntimeError(format!(
                "LoadSharedModuleBinding: slot {} out of bounds (module_bindings len {})",
                index,
                self.module_bindings.len()
            )));
        }
        let bits = self.module_bindings[index];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "LoadSharedModuleBinding: Shared module binding pointer is null"
                    .to_string(),
            ));
        }
        // SAFETY: `cell_ptr` was produced by
        // `Arc::into_raw(Arc::new(...))` in
        // `op_alloc_shared_module_binding`; the Arc share owned by the
        // module-bindings slot keeps the allocation alive. Reborrowed
        // for the duration of this handler only.
        let value_bits = unsafe {
            let cell: &SharedCell = &*cell_ptr;
            let guard = cell.lock();
            let bits = *guard;
            drop(guard);
            bits
        };
        self.push_raw_u64(value_bits)
    }

    /// `StoreSharedModuleBinding { idx }`: pop a ValueWord, read the
    /// `*const SharedCell` bits from `module_bindings[idx]`, acquire
    /// the mutex for a write, overwrite the inner ValueWord bits, drop
    /// the guard. The slot's pointer bits are NOT modified.
    fn op_store_shared_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let index = idx as usize;
        if index >= self.module_bindings.len() {
            return Err(VMError::RuntimeError(format!(
                "StoreSharedModuleBinding: slot {} out of bounds (module_bindings len {})",
                index,
                self.module_bindings.len()
            )));
        }
        let bits = self.module_bindings[index];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "StoreSharedModuleBinding: Shared module binding pointer is null"
                    .to_string(),
            ));
        }
        record_heap_write();
        // SAFETY: same invariants as `op_load_shared_module_binding`.
        // Mutex serialises writers; the Arc share owned by the module-
        // bindings slot is untouched.
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

            // Track A.1C.3: the SharedCell wrapper is retired. Every
            // LoadLocal slot now holds either a plain ValueWord (the
            // common case) or, for `AllocSharedLocal`-promoted slots,
            // raw `*const SharedCell` pointer bits that the compiler
            // would never emit `LoadLocal` on — those slots are only
            // accessed via `Load/StoreSharedLocal`.
            let nb = unsafe { ValueWord::clone_from_bits(bits) };
            self.push_raw_u64(nb)?;
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
        use shape_value::tag_bits::{get_tag, is_tagged, TAG_BOOL, TAG_INT};
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
            //
            // BUG4: the `trusted` contract is that the *compiler* promises the
            // kind, but across call boundaries the FrameDescriptor's slot-kind
            // inference can over-specify (e.g. claim `Float64` for a parameter
            // that is actually populated with a TAG_INT value from the caller).
            // Guard each typed fast path with a runtime tag check that matches
            // the encoding — on mismatch, fall through to the legacy
            // `clone_from_bits` path, which handles every inline-scalar /
            // heap encoding uniformly. This mirrors the runtime guards in
            // `op_load_local` and prevents reinterpreting an i48-tagged slot
            // as a raw f64 (which would produce NaN).
            let kind = self
                .current_frame_descriptor()
                .map(|fd| fd.slot(idx as usize))
                .unwrap_or(SlotKind::Unknown);
            match kind {
                SlotKind::Float64 if !is_tagged(bits) => {
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
                    return self.push_raw_u64(bits);
                }
                _ => {
                    // Tag/kind mismatch, non-scalar slot, or Unknown — fall
                    // through to the legacy refcount-aware path.
                }
            }
            let nb = unsafe { ValueWord::clone_from_bits(bits) };
            self.push_raw_u64(nb)?;
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

            // Track A.1C.3: SharedCell wrapper retired. Plain move
            // semantics — zero the source slot and push the bits. The
            // compiler never emits `LoadLocalMove` on
            // `AllocSharedLocal`-promoted slots.
            self.stack[slot] = Self::NONE_BITS;
            self.push_raw_u64(bits)?;
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

            // Track A.1C.3: SharedCell wrapper retired. Plain clone —
            // bump refcount for heap values.
            let cloned = raw_helpers::clone_raw_bits(bits);
            self.push_raw_u64(cloned)?;
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

            // Track A.1C.3: SharedCell wrapper retired. Drop the old
            // value and store the new one directly.
            if is_tagged(old_bits) && get_tag(old_bits) == TAG_HEAP {
                raw_helpers::drop_raw_bits(old_bits);
            }
            record_heap_write();
            write_barrier_slot(old_bits, new_bits);
            self.stack[slot] = new_bits;
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

            // Track A.1C.3: SharedCell wrapper retired. Plain store —
            // the compiler never emits `StoreLocal` on
            // `AllocSharedLocal`-promoted slots.
            let nb = self.pop_raw_u64()?;
            record_heap_write();
            write_barrier_slot(self.stack[slot], nb.raw_bits());
            self.stack_write_raw(slot, nb);
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

            // Track A.1C.3: SharedCell wrapper retired.
            record_heap_write();
            write_barrier_slot(self.stack[slot], truncated.raw_bits());
            self.stack_write_raw(slot, truncated);
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    // ===== Wave E+3: per-FieldKind typed local load/store handlers =====
    //
    // Typed counterparts of `op_load_local` / `op_store_local`. Each
    // handler reads/writes the local slot at `bp + idx` directly as raw
    // 8-byte bits, bypassing ValueWord wrapping, NaN-box tag checks, and
    // SharedCell auto-deref. The compiler proves the slot kind matches
    // `<Kind>` before emitting the typed opcode; mismatches are a
    // compiler bug.
    //
    // For sub-i64 integer kinds (I32/U32/I16/U16/I8/U8): on store, the
    // popped 8-byte slot is truncated to the declared width and
    // sign/zero-extended back into 8 bytes for storage (matches D.1 store
    // truncation). On load, the slot's bits already carry the
    // matching-Kind encoding written by a paired Store, so a raw read +
    // sign/zero-extend reconstructs the original i64-shaped value.
    //
    // `Bool` follows the same convention as D.1's StoreOwnedMutableBool:
    // any nonzero pop is treated as true; the slot stores 0 or 1.
    //
    // `Ptr` is raw bit-level pass-through. Neither Load nor Store
    // performs `vw_clone` / `vw_drop`. The IR pairs each typed Ptr
    // load/store with the matching retain/release before/after — see
    // commit afb1651 (c-stdlib-msgpack pattern).

    fn op_load_local_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI64 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // SAFETY: compiler proved the slot's kind is I64 — the bits were
        // written by a matching-Kind StoreLocalI64. Read raw 8 bytes.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_raw_u64(bits)
    }

    fn op_load_local_u64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU64 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_raw_u64(bits)
    }

    fn op_load_local_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalF64 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_raw_u64(bits)
    }

    fn op_load_local_i32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI32 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Read low 4 bytes as i32, sign-extend to i64 for the 8-byte stack slot.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as i32;
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_local_u32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU32 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Read low 4 bytes as u32, zero-extend to u64.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as u32;
        self.push_raw_u64(value as u64)
    }

    fn op_load_local_i16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI16 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Read low 2 bytes as i16, sign-extend to i64.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as i16;
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_local_u16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU16 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Read low 2 bytes as u16, zero-extend to u64.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as u16;
        self.push_raw_u64(value as u64)
    }

    fn op_load_local_i8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI8 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Read low byte as i8, sign-extend to i64.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as i8;
        self.push_raw_u64(value as i64 as u64)
    }

    fn op_load_local_u8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU8 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Read low byte as u8, zero-extend to u64.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as u8;
        self.push_raw_u64(value as u64)
    }

    fn op_load_local_bool(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalBool slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // The slot was written by a paired StoreLocalBool that canonicalized
        // to 0 or 1 in the low byte. Pass the raw bits through unchanged.
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_raw_u64(bits)
    }

    fn op_load_local_ptr(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalPtr slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // Raw 8-byte read. Refcount semantics are the IR's responsibility
        // — the handler does NOT clone/retain. The IR pairs LoadLocalPtr
        // with `vw_clone` (matches the c-stdlib-msgpack pattern in commit
        // afb1651, mirroring `op_load_owned_mutable_capture_ptr`).
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_raw_u64(bits)
    }

    fn op_store_local_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        // SAFETY: compiler proved the slot's kind is I64. No SharedCell
        // wrap, no refcount on the old bits — scalar i64 has no
        // ownership.
        self.stack[slot] = new_bits;
        Ok(())
    }

    fn op_store_local_u64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_bits;
        Ok(())
    }

    fn op_store_local_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_bits;
        Ok(())
    }

    fn op_store_local_i32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Truncate to i32 (low 4 bytes), sign-extend back to i64 for storage.
        let new_value = self.pop_raw_u64()? as i32;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value as i64 as u64;
        Ok(())
    }

    fn op_store_local_u32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Truncate to u32 (low 4 bytes), zero-extend back to u64 for storage.
        let new_value = self.pop_raw_u64()? as u32;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value as u64;
        Ok(())
    }

    fn op_store_local_i16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i16;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value as i64 as u64;
        Ok(())
    }

    fn op_store_local_u16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u16;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value as u64;
        Ok(())
    }

    fn op_store_local_i8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as i8;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value as i64 as u64;
        Ok(())
    }

    fn op_store_local_u8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_value = self.pop_raw_u64()? as u8;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value as u64;
        Ok(())
    }

    fn op_store_local_bool(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Any nonzero pop ⇒ true (matches D.1's StoreOwnedMutableCaptureBool
        // convention). Canonicalize to 0 or 1 for storage.
        let new_value = (self.pop_raw_u64()? != 0) as u64;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        self.stack[slot] = new_value;
        Ok(())
    }

    fn op_store_local_ptr(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let new_bits = self.pop_raw_u64()?;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
        }
        record_heap_write();
        // SAFETY: caller's IR has paired this store with a vw_drop earlier
        // (or this is the first write to a fresh slot). The handler does
        // NOT release the previous payload nor retain the new one —
        // refcount semantics are the IR's responsibility. Matches D.1 /
        // D.2 Ptr semantics.
        self.stack[slot] = new_bits;
        Ok(())
    }

    /// Load value from a module_binding variable slot.
    pub(in crate::executor) fn op_load_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::ModuleBinding(idx)) = instruction.operand {
            // WB2.2 retain-on-read: push an owning share onto the stack
            // so heap-tagged bindings are not aliased across the stack
            // slot and the binding slot. Phase 3 enables stack
            // `vw_drop_slice` on teardown; without this retain, the
            // stack's release would decrement an Arc the binding still
            // holds.
            let nb = if (idx as usize) < self.module_bindings.len() {
                self.binding_read_owned(idx as usize)
            } else {
                ValueWord::none()
            };
            // Track A.1C.3: SharedCell auto-deref retired. Shared
            // module-binding slots are read via
            // `LoadSharedModuleBinding`; plain `LoadModuleBinding` only
            // reaches unpromoted slots holding a plain ValueWord (or,
            // at closure-creation time, raw Arc pointer bits that the
            // consumer — `op_make_closure` — interprets via the
            // `ClosureLayout::capture_storage_kind` Shared branch).
            self.push_raw_u64(nb)?;
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

            // Track A.1C.3: SharedCell auto-deref retired. The compiler
            // routes promoted module-binding writes through
            // `StoreSharedModuleBinding`; plain `StoreModuleBinding`
            // only fires on unpromoted slots.
            record_heap_write();
            write_barrier_slot(self.module_bindings[index], nb.raw_bits());
            self.binding_write_raw(index, nb);
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

            // Track A.1C.3: SharedCell auto-deref retired.
            record_heap_write();
            write_barrier_slot(self.module_bindings[index], truncated.raw_bits());
            self.binding_write_raw(index, truncated);
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    // ── Wave E+3: per-FieldKind typed module-binding opcodes ────────────
    //
    // Twenty-two handlers — eleven Loads and eleven Stores — paired with
    // the opcode definitions at 0x182..=0x197 in `bytecode/opcode_defs.rs`.
    // The handlers mirror the legacy `op_load_module_binding` /
    // `op_store_module_binding` shape but read/write the slot's 8 raw
    // bytes directly without `vw_clone` / `vw_drop`: the typed contract
    // says the slot holds a raw native value (i64/u64/f64/i32/.../bool),
    // so heap-pointer refcount traffic is encoded in the IR around these
    // opcodes rather than inside them. For Ptr the same rule applies —
    // Wave E+4 wraps the Load/Store pair with `vw_clone`/`vw_drop`
    // (matches the c-stdlib-msgpack pattern from commit afb1651, mirroring
    // Wave D's typed OwnedMutable Ptr handlers).
    //
    // Stores grow `module_bindings` to fit `idx` if necessary (matches
    // legacy `op_store_module_binding`). Loads return `0` (raw bits) for
    // out-of-bounds slots — there is no in-band error; OOB indicates a
    // compiler bug since the static contract requires the slot to be
    // initialised by a matching typed Store before the Load fires.

    fn op_load_module_binding_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        self.push_raw_u64(bits)
    }

    fn op_load_module_binding_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        self.push_raw_u64(bits)
    }

    fn op_load_module_binding_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        self.push_raw_u64(bits)
    }

    fn op_load_module_binding_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Sign-extend low 4 bytes to i64 — matches Wave D's
        // `op_load_owned_mutable_capture_i32` width-extension convention.
        let value = bits as i32 as i64 as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Zero-extend low 4 bytes.
        let value = bits as u32 as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Sign-extend low 2 bytes to i64.
        let value = bits as i16 as i64 as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Zero-extend low 2 bytes.
        let value = bits as u16 as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Sign-extend low byte to i64.
        let value = bits as i8 as i64 as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Zero-extend low byte.
        let value = bits as u8 as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // Push 0/1 — any non-zero low byte ⇒ true. Mirrors Wave D's
        // bool stack convention.
        let value = ((bits as u8) != 0) as u64;
        self.push_raw_u64(value)
    }

    fn op_load_module_binding_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        let bits = if index < self.module_bindings.len() {
            self.binding_read_raw(index).into_raw_bits()
        } else {
            0u64
        };
        // The 8-byte payload is a ValueWord bit pattern carrying a
        // NaN-boxed Arc/Box pointer. The handler does NOT clone/retain —
        // refcount semantics are the caller's (Wave E+4 IR) responsibility.
        self.push_raw_u64(bits)
    }

    fn op_store_module_binding_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()?;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        // Direct slot write — typed contract guarantees prior value is
        // either NONE_BITS (initial) or a raw native value from a
        // previous typed Store. No `vw_drop` of the prior value.
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()?;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()?;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Truncate to i32, sign-extend back to i64 for the 8-byte slot.
        let value = self.pop_raw_u64()? as i32 as i64 as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Truncate to u32, zero-extend.
        let value = self.pop_raw_u64()? as u32 as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()? as i16 as i64 as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()? as u16 as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()? as i8 as i64 as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()? as u8 as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Any non-zero bit pattern ⇒ true.
        let value = (self.pop_raw_u64()? != 0) as u64;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        self.module_bindings[index] = value;
        Ok(())
    }

    fn op_store_module_binding_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let value = self.pop_raw_u64()?;
        let index = idx as usize;
        while self.module_bindings.len() <= index {
            self.module_bindings.push(Self::NONE_BITS);
        }
        record_heap_write();
        // The 8-byte payload is a ValueWord bit pattern carrying a
        // NaN-boxed Arc/Box pointer. The handler does NOT release the
        // previous payload nor retain the new one — refcount semantics
        // are the caller's (Wave E+4 IR) responsibility, mirroring Wave
        // D's `op_store_owned_mutable_capture_ptr`.
        self.module_bindings[index] = value;
        Ok(())
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

    // RETIRED: test_store_typed_load_trusted_roundtrip
    // The strict-typing sweep deleted the *Dynamic arithmetic opcodes;
    // mixing int + float at the bytecode level no longer has a path. The
    // typed `StoreLocalTyped + LoadLocalTrusted` pair is exercised by the
    // surrounding tests using single-typed slots.

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
        //
        // B6.4: `ValueWord = u64` has no Drop, so `mem::forget` /
        // `drop` would be no-ops. The poisoning is actually done by
        // `stack_take_raw` itself (writes `NONE_BITS` into slot 0); the
        // returned bits are discarded with `let _ =`.
        let _ = vm.stack_take_raw(0);
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
            closure_heap_bits: None,
        });
    }

    /// Wave D (D.3): variant of `push_synthetic_frame_with_upvalues`
    /// that registers a `ClosureLayout` for `function_id` on the VM's
    /// program and pushes a frame whose `function_id = Some(fid)`. The
    /// type-aware legacy `Load/StoreOwnedMutableCapture` handlers
    /// consult `current_closure_layout()` (resolves via `function_id` →
    /// `program.closure_function_layouts[fid]`) to dispatch on
    /// `capture_inner_kind`, so any test exercising those legacy
    /// opcodes must use this helper instead of
    /// `push_synthetic_frame_with_upvalues`.
    fn push_synthetic_closure_frame_with_layout(
        vm: &mut VirtualMachine,
        fid: u16,
        layout: shape_value::v2::closure_layout::ClosureLayout,
        upvalues: Vec<Upvalue>,
    ) {
        while vm.program.closure_function_layouts.len() < fid as usize + 1 {
            vm.program.closure_function_layouts.push(None);
        }
        vm.program.closure_function_layouts[fid as usize] = Some(StdArc::new(layout));
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: vm.sp,
            locals_count: 0,
            function_id: Some(fid),
            upvalues: Some(upvalues),
            blob_hash: None,
            closure_heap_bits: None,
        });
    }

    #[test]
    fn a1b_load_owned_mutable_capture_derefs_box_cell() {
        // Wave D (D.3): the cell now stores a native i64 (allocated by
        // `closure_raw::alloc_owned_mutable_i64`), and the legacy
        // `op_load_owned_mutable_capture` dispatches on
        // `layout.capture_inner_kind(idx)` — so the synthetic frame
        // must register a matching layout via
        // `push_synthetic_closure_frame_with_layout`.
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::closure_raw::alloc_owned_mutable_i64;
        use shape_value::v2::concrete_type::ConcreteType;
        let mut vm = fresh_vm_for_capture_opcode_test();
        let cell: *mut i64 = alloc_owned_mutable_i64(42);
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::OwnedMutable]);
        push_synthetic_closure_frame_with_layout(
            &mut vm,
            0,
            layout,
            vec![Upvalue::new(cell as u64)],
        );

        let instr = Instruction::new(
            OpCode::LoadOwnedMutableCapture,
            Some(Operand::Local(0)),
        );
        vm.op_load_owned_mutable_capture(&instr).unwrap();

        let out = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(out).as_i64(), Some(42));

        // SAFETY: `cell` came from `alloc_owned_mutable_i64` (which
        // wraps `Box::into_raw(Box::new(initial))`); we reclaim it via
        // the matching `Box::<i64>::from_raw` exactly once.
        unsafe {
            drop(Box::from_raw(cell));
        }
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_store_owned_mutable_capture_writes_through_box_cell() {
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::closure_raw::{alloc_owned_mutable_i64, read_owned_mutable_i64};
        use shape_value::v2::concrete_type::ConcreteType;
        let mut vm = fresh_vm_for_capture_opcode_test();
        let cell: *mut i64 = alloc_owned_mutable_i64(1);
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::OwnedMutable]);
        push_synthetic_closure_frame_with_layout(
            &mut vm,
            0,
            layout,
            vec![Upvalue::new(cell as u64)],
        );

        // Push new value 999 onto the stack and StoreOwnedMutableCapture.
        vm.push_raw_u64(ValueWord::from_i64(999).into_raw_bits())
            .unwrap();
        let instr = Instruction::new(
            OpCode::StoreOwnedMutableCapture,
            Some(Operand::Local(0)),
        );
        vm.op_store_owned_mutable_capture(&instr).unwrap();

        // SAFETY: cell is still live. Read the cell back via the typed
        // helper to confirm the native write landed.
        unsafe {
            assert_eq!(read_owned_mutable_i64(cell), 999);
            drop(Box::from_raw(cell));
        }
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_owned_mutable_roundtrip_load_store_load() {
        // Full roundtrip: load initial value, store a new value, load
        // it back.
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::closure_raw::alloc_owned_mutable_i64;
        use shape_value::v2::concrete_type::ConcreteType;
        let mut vm = fresh_vm_for_capture_opcode_test();
        let cell: *mut i64 = alloc_owned_mutable_i64(10);
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::OwnedMutable]);
        push_synthetic_closure_frame_with_layout(
            &mut vm,
            0,
            layout,
            vec![Upvalue::new(cell as u64)],
        );

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

        // SAFETY: reclaim via Box<i64>::from_raw matching the alloc.
        unsafe {
            drop(Box::from_raw(cell));
        }
        vm.call_stack.pop();
    }

    #[test]
    fn a1b_load_shared_capture_locks_and_returns_inner() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let external: StdArc<SharedCell> =
            StdArc::new(SharedCell::new(ValueWord::from_i64(77)));
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
            StdArc::new(SharedCell::new(ValueWord::from_i64(0)));
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
            StdArc::new(SharedCell::new(ValueWord::from_i64(0)));

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
        //
        // Wave D (D.3): the OwnedMutable cell is now allocated via the
        // typed `alloc_owned_mutable_i64` helper (native i64 storage),
        // and the legacy `Load/StoreOwnedMutableCapture` handlers
        // dispatch on `layout.capture_inner_kind(idx)` — so the
        // synthetic frame must register a matching 4-capture layout.
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::closure_raw::alloc_owned_mutable_i64;
        use shape_value::v2::concrete_type::ConcreteType;
        let mut vm = fresh_vm_for_capture_opcode_test();
        let box_cell: *mut i64 = alloc_owned_mutable_i64(200);
        let external: StdArc<SharedCell> =
            StdArc::new(SharedCell::new(ValueWord::from_i64(300)));
        let arc_raw: *const SharedCell = StdArc::into_raw(StdArc::clone(&external));
        let layout = ClosureLayout::from_capture_types(
            &[
                ConcreteType::F64,
                ConcreteType::I64,
                ConcreteType::I64,
                ConcreteType::I64,
            ],
            &[
                CaptureKind::Immutable,
                CaptureKind::OwnedMutable,
                CaptureKind::Shared,
                CaptureKind::Immutable,
            ],
        );
        push_synthetic_closure_frame_with_layout(
            &mut vm,
            0,
            layout,
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

    // ── Track A.1C.1: outer-scope `var` Shared-cell lifecycle tests ───
    //
    // These tests exercise `AllocSharedLocal` / `LoadSharedLocal` /
    // `StoreSharedLocal` / `DropSharedLocal` directly on a synthetic
    // call frame that owns a handful of local slots. Unlike the A.1B
    // capture-side tests above, the pointer bits land in the **stack
    // slot** (`stack[base_pointer + idx]`) rather than in an upvalue.
    // The helper below reserves `num_locals` stack slots behind the
    // frame's `base_pointer` so that Local(idx) addressing for
    // idx < num_locals is in-bounds.
    //
    // After each test we `DropSharedLocal` every slot we allocated, so
    // the Arcs are reclaimed cleanly — the harness does not run
    // scope-exit bytecode automatically, so leak-free teardown is the
    // test's responsibility.

    /// Push a synthetic call frame that owns `num_locals` stack slots.
    /// Slots are pre-filled with `NONE_BITS` so `AllocSharedLocal` can
    /// overwrite them without tripping the "old occupant" drop path.
    fn push_synthetic_frame_with_locals(vm: &mut VirtualMachine, num_locals: usize) {
        let base_pointer = vm.sp;
        for _ in 0..num_locals {
            // Keep `sp` and `stack.len()` in sync so
            // `current_locals_base()` plus Local(idx) addresses are
            // within `self.stack`.
            vm.push_raw_u64(VirtualMachine::NONE_BITS).unwrap();
        }
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer,
            locals_count: num_locals,
            function_id: None,
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
        });
    }

    #[test]
    fn a1c1_alloc_load_shared_local_roundtrip() {
        // AllocSharedLocal installs a SharedCell in slot 0 initialised
        // from the top-of-stack ValueWord; LoadSharedLocal reads the
        // same cell back.
        let mut vm = fresh_vm_for_capture_opcode_test();
        push_synthetic_frame_with_locals(&mut vm, 1);

        // Push the initial value, then AllocSharedLocal { slot: 0 }.
        vm.push_raw_u64(ValueWord::from_i64(42).into_raw_bits())
            .unwrap();
        let alloc_instr = Instruction::new(OpCode::AllocSharedLocal, Some(Operand::Local(0)));
        vm.op_alloc_shared_local(&alloc_instr).unwrap();

        // Verify that slot 0 now holds a non-null raw pointer.
        let bp = vm.current_locals_base();
        let slot_bits = vm.stack[bp];
        assert_ne!(slot_bits, 0, "slot 0 should hold a non-null SharedCell pointer");

        // LoadSharedLocal { slot: 0 } → 42.
        let load_instr = Instruction::new(OpCode::LoadSharedLocal, Some(Operand::Local(0)));
        vm.op_load_shared_local(&load_instr).unwrap();
        let out = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(out).as_i64(), Some(42));

        // Teardown: drop the Shared cell so the Arc is reclaimed.
        let drop_instr = Instruction::new(OpCode::DropSharedLocal, Some(Operand::Local(0)));
        vm.op_drop_shared_local(&drop_instr).unwrap();
        vm.call_stack.pop();
    }

    #[test]
    fn a1c1_store_load_shared_local_roundtrip() {
        // After Alloc, StoreSharedLocal overwrites the mutex contents;
        // a subsequent LoadSharedLocal observes the new value.
        let mut vm = fresh_vm_for_capture_opcode_test();
        push_synthetic_frame_with_locals(&mut vm, 1);

        vm.push_raw_u64(ValueWord::from_i64(1).into_raw_bits())
            .unwrap();
        let alloc_instr = Instruction::new(OpCode::AllocSharedLocal, Some(Operand::Local(0)));
        vm.op_alloc_shared_local(&alloc_instr).unwrap();

        // Store 100.
        vm.push_raw_u64(ValueWord::from_i64(100).into_raw_bits())
            .unwrap();
        let store_instr = Instruction::new(OpCode::StoreSharedLocal, Some(Operand::Local(0)));
        vm.op_store_shared_local(&store_instr).unwrap();

        // Load → 100.
        let load_instr = Instruction::new(OpCode::LoadSharedLocal, Some(Operand::Local(0)));
        vm.op_load_shared_local(&load_instr).unwrap();
        let out = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(out).as_i64(), Some(100));

        // Teardown.
        let drop_instr = Instruction::new(OpCode::DropSharedLocal, Some(Operand::Local(0)));
        vm.op_drop_shared_local(&drop_instr).unwrap();
        vm.call_stack.pop();
    }

    #[test]
    fn a1c1_drop_shared_local_releases_arc() {
        // Allocate a Shared cell via AllocSharedLocal, mint an
        // independent external Arc share observing the same underlying
        // cell, then DropSharedLocal — the external Arc's strong count
        // must drop by exactly 1.
        let mut vm = fresh_vm_for_capture_opcode_test();
        push_synthetic_frame_with_locals(&mut vm, 1);

        vm.push_raw_u64(ValueWord::from_i64(7).into_raw_bits())
            .unwrap();
        let alloc_instr = Instruction::new(OpCode::AllocSharedLocal, Some(Operand::Local(0)));
        vm.op_alloc_shared_local(&alloc_instr).unwrap();

        // Read the raw pointer that AllocSharedLocal parked in slot 0.
        let bp = vm.current_locals_base();
        let cell_ptr = vm.stack[bp] as *const SharedCell;
        assert!(!cell_ptr.is_null());

        // Mint an external Arc share without removing the slot's share.
        // `Arc::increment_strong_count` bumps the count by 1; then
        // `Arc::from_raw` reconstructs an owning handle consuming the
        // bumped share (see the idiom documented in `Arc::from_raw`).
        let external: StdArc<SharedCell> = unsafe {
            StdArc::increment_strong_count(cell_ptr);
            StdArc::from_raw(cell_ptr)
        };
        // Strong count: 1 (slot) + 1 (external) = 2.
        assert_eq!(StdArc::strong_count(&external), 2);

        // DropSharedLocal must release the slot's share.
        let drop_instr = Instruction::new(OpCode::DropSharedLocal, Some(Operand::Local(0)));
        vm.op_drop_shared_local(&drop_instr).unwrap();

        // Only the external share remains.
        assert_eq!(StdArc::strong_count(&external), 1);

        // The spent slot holds null pointer bits (`0u64`), NOT
        // `NONE_BITS` — this is intentional so the null-guard in
        // `op_load_shared_local` / `op_store_shared_local` catches
        // accidental reuse. See the safety note on
        // `op_drop_shared_local`.
        assert_eq!(vm.stack[bp], 0u64);

        // Subsequent Load on the spent slot returns a runtime error
        // rather than segfaulting.
        let load_instr = Instruction::new(OpCode::LoadSharedLocal, Some(Operand::Local(0)));
        let err = vm.op_load_shared_local(&load_instr).unwrap_err();
        match err {
            shape_value::VMError::RuntimeError(msg) => {
                assert!(msg.contains("null"), "expected null-pointer error, got: {}", msg)
            }
            other => panic!("expected RuntimeError, got {:?}", other),
        }

        drop(external);
        vm.call_stack.pop();
    }

    #[test]
    fn a1c1_shared_local_lock_is_parking_lot() {
        // Smoke test that the parking_lot mutex correctly serialises
        // concurrent readers/writers without deadlocking. This is NOT a
        // loom test — it just exercises the read/write path enough to
        // catch an obvious contention bug (e.g. using a non-reentrant
        // std::sync::RwLock incorrectly).
        let mut vm = fresh_vm_for_capture_opcode_test();
        push_synthetic_frame_with_locals(&mut vm, 1);

        vm.push_raw_u64(ValueWord::from_i64(0).into_raw_bits())
            .unwrap();
        let alloc_instr = Instruction::new(OpCode::AllocSharedLocal, Some(Operand::Local(0)));
        vm.op_alloc_shared_local(&alloc_instr).unwrap();

        // Grab a raw share for a side thread.
        let bp = vm.current_locals_base();
        let cell_ptr = vm.stack[bp] as *const SharedCell;
        let external_arc: StdArc<SharedCell> = unsafe {
            StdArc::increment_strong_count(cell_ptr);
            StdArc::from_raw(cell_ptr)
        };

        // The worker writes through its own Arc handle while the main
        // thread interleaves reads and writes through the VM opcodes.
        // We only care that the loop terminates — no deadlock.
        let writer_arc = StdArc::clone(&external_arc);
        let worker = std::thread::spawn(move || {
            for i in 0..100 {
                let mut guard = writer_arc.lock();
                *guard = ValueWord::from_i64(i as i64).into_raw_bits();
                drop(guard);
            }
        });

        let load_instr = Instruction::new(OpCode::LoadSharedLocal, Some(Operand::Local(0)));
        let store_instr = Instruction::new(OpCode::StoreSharedLocal, Some(Operand::Local(0)));
        for i in 0..100 {
            vm.push_raw_u64(ValueWord::from_i64(i as i64 * 2).into_raw_bits())
                .unwrap();
            vm.op_store_shared_local(&store_instr).unwrap();
            vm.op_load_shared_local(&load_instr).unwrap();
            // Drain the pushed value so the stack doesn't grow unbounded.
            let _ = vm.pop_raw_u64().unwrap();
        }

        worker.join().unwrap();

        // Final read to confirm the cell is still live and lockable.
        vm.op_load_shared_local(&load_instr).unwrap();
        let final_bits = vm.pop_raw_u64().unwrap();
        // The final value could come from either the main thread or
        // the worker — both wrote `i64`-encoded ValueWords, so the
        // decode must succeed.
        assert!(ValueWord::from_raw_bits(final_bits).as_i64().is_some());

        // Teardown.
        let drop_instr = Instruction::new(OpCode::DropSharedLocal, Some(Operand::Local(0)));
        vm.op_drop_shared_local(&drop_instr).unwrap();
        drop(external_arc);
        vm.call_stack.pop();
    }

    #[test]
    fn a1c1_multiple_slots_independent() {
        // Two slots, each holding its own Shared cell. Writes to one
        // slot must NOT observably affect the other.
        let mut vm = fresh_vm_for_capture_opcode_test();
        push_synthetic_frame_with_locals(&mut vm, 2);

        // Slot 0 ← 11, Slot 1 ← 22.
        vm.push_raw_u64(ValueWord::from_i64(11).into_raw_bits())
            .unwrap();
        vm.op_alloc_shared_local(&Instruction::new(
            OpCode::AllocSharedLocal,
            Some(Operand::Local(0)),
        ))
        .unwrap();
        vm.push_raw_u64(ValueWord::from_i64(22).into_raw_bits())
            .unwrap();
        vm.op_alloc_shared_local(&Instruction::new(
            OpCode::AllocSharedLocal,
            Some(Operand::Local(1)),
        ))
        .unwrap();

        // The two slots must hold distinct pointers.
        let bp = vm.current_locals_base();
        assert_ne!(vm.stack[bp], vm.stack[bp + 1]);

        // Write 111 to slot 0, observe slot 1 is still 22.
        vm.push_raw_u64(ValueWord::from_i64(111).into_raw_bits())
            .unwrap();
        vm.op_store_shared_local(&Instruction::new(
            OpCode::StoreSharedLocal,
            Some(Operand::Local(0)),
        ))
        .unwrap();
        vm.op_load_shared_local(&Instruction::new(
            OpCode::LoadSharedLocal,
            Some(Operand::Local(1)),
        ))
        .unwrap();
        let slot1 = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(slot1).as_i64(), Some(22));

        // Write 222 to slot 1, observe slot 0 is 111.
        vm.push_raw_u64(ValueWord::from_i64(222).into_raw_bits())
            .unwrap();
        vm.op_store_shared_local(&Instruction::new(
            OpCode::StoreSharedLocal,
            Some(Operand::Local(1)),
        ))
        .unwrap();
        vm.op_load_shared_local(&Instruction::new(
            OpCode::LoadSharedLocal,
            Some(Operand::Local(0)),
        ))
        .unwrap();
        let slot0 = vm.pop_raw_u64().unwrap();
        assert_eq!(ValueWord::from_raw_bits(slot0).as_i64(), Some(111));

        // Final values check.
        vm.op_load_shared_local(&Instruction::new(
            OpCode::LoadSharedLocal,
            Some(Operand::Local(1)),
        ))
        .unwrap();
        assert_eq!(
            ValueWord::from_raw_bits(vm.pop_raw_u64().unwrap()).as_i64(),
            Some(222)
        );

        // Teardown: drop both cells.
        vm.op_drop_shared_local(&Instruction::new(
            OpCode::DropSharedLocal,
            Some(Operand::Local(0)),
        ))
        .unwrap();
        vm.op_drop_shared_local(&Instruction::new(
            OpCode::DropSharedLocal,
            Some(Operand::Local(1)),
        ))
        .unwrap();
        vm.call_stack.pop();
    }

    // ── Track D.2: tests for the typed Shared capture handlers ─────────
    //
    // Each test uses an `Arc<SharedCell>` allocated as an "external"
    // observer plus one `Arc::into_raw`-produced share parked in a
    // synthetic frame's upvalue slot. The interior `FieldKind` is
    // implicit in which `read/write_shared_<kind>` helper we call —
    // the cell itself is just an 8-byte payload + lock byte.
    //
    // Each test follows the same pattern: Load → assert initial value;
    // Store new value; Load → assert new value; observe via the
    // external Arc; reclaim the Arc strong-count share; pop frame.
    //
    // The mandated four kinds (I64, F64, Bool, Ptr) cover every
    // corner of the helper set:
    //   * I64 — full 8-byte signed payload (no sign-extension dance).
    //   * F64 — bitwise-identical f64 round-trip.
    //   * Bool — narrow payload + the "any non-zero byte ⇒ true"
    //          read convention.
    //   * Ptr — raw 8-byte payload round-trip with NO retain/release
    //          in the handler (matches the helper's contract).
    //
    // Locking sanity: every test executes Load → Store → Load on the
    // same cell. `read_shared_<kind>` / `write_shared_<kind>` acquire
    // and release the cell's mutex inside each call. If the handler
    // accidentally took the lock externally we would deadlock on the
    // second call (single-threaded). The tests passing means no
    // double-locking occurs.

    #[test]
    fn d2_load_store_load_shared_capture_i64() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let initial: i64 = -987_654_321;
        let external: StdArc<SharedCell> =
            StdArc::new(SharedCell::new(ValueWord::from_i64(0)));
        // SAFETY: cell is fresh, no other reader/writer; we mint a
        // typed initial value through the lock-gated helper.
        unsafe {
            shape_value::v2::closure_raw::write_shared_i64(
                StdArc::as_ptr(&external) as *const SharedCell,
                initial,
            );
        }
        let closure_share: StdArc<SharedCell> = StdArc::clone(&external);
        let cell_ptr: *const SharedCell = StdArc::into_raw(closure_share);
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![Upvalue::new(cell_ptr as u64)],
        );

        // Round 1: Load — expect initial.
        let load = Instruction::new(
            OpCode::LoadSharedCaptureI64,
            Some(Operand::Local(0)),
        );
        vm.op_load_shared_capture_i64(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap() as i64, initial);

        // Round 2: Store new value.
        let new_val: i64 = i64::MAX - 7;
        vm.push_raw_u64(new_val as u64).unwrap();
        let store = Instruction::new(
            OpCode::StoreSharedCaptureI64,
            Some(Operand::Local(0)),
        );
        vm.op_store_shared_capture_i64(&store).unwrap();

        // Round 3: Load — expect new_val.
        vm.op_load_shared_capture_i64(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap() as i64, new_val);

        // External observer sees new_val.
        // SAFETY: external is a live Arc; helper acquires its own
        // lock; cell points at the same allocation as cell_ptr.
        let observed = unsafe {
            shape_value::v2::closure_raw::read_shared_i64(
                StdArc::as_ptr(&external) as *const SharedCell,
            )
        };
        assert_eq!(observed, new_val);

        // SAFETY: cell_ptr came from Arc::into_raw, one share.
        unsafe { drop(StdArc::from_raw(cell_ptr)) };
        assert_eq!(StdArc::strong_count(&external), 1);
        vm.call_stack.pop();
    }

    #[test]
    fn d2_load_store_load_shared_capture_f64() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let initial: f64 = std::f64::consts::PI;
        let external: StdArc<SharedCell> =
            StdArc::new(SharedCell::new(ValueWord::from_i64(0)));
        // SAFETY: see I64 test rationale.
        unsafe {
            shape_value::v2::closure_raw::write_shared_f64(
                StdArc::as_ptr(&external) as *const SharedCell,
                initial,
            );
        }
        let closure_share: StdArc<SharedCell> = StdArc::clone(&external);
        let cell_ptr: *const SharedCell = StdArc::into_raw(closure_share);
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![Upvalue::new(cell_ptr as u64)],
        );

        let load = Instruction::new(
            OpCode::LoadSharedCaptureF64,
            Some(Operand::Local(0)),
        );
        vm.op_load_shared_capture_f64(&load).unwrap();
        assert_eq!(f64::from_bits(vm.pop_raw_u64().unwrap()), initial);

        let new_val: f64 = -1.5e+200;
        vm.push_raw_u64(new_val.to_bits()).unwrap();
        let store = Instruction::new(
            OpCode::StoreSharedCaptureF64,
            Some(Operand::Local(0)),
        );
        vm.op_store_shared_capture_f64(&store).unwrap();

        vm.op_load_shared_capture_f64(&load).unwrap();
        assert_eq!(f64::from_bits(vm.pop_raw_u64().unwrap()), new_val);

        let observed = unsafe {
            shape_value::v2::closure_raw::read_shared_f64(
                StdArc::as_ptr(&external) as *const SharedCell,
            )
        };
        assert_eq!(observed, new_val);

        // SAFETY: cell_ptr came from Arc::into_raw, one share.
        unsafe { drop(StdArc::from_raw(cell_ptr)) };
        assert_eq!(StdArc::strong_count(&external), 1);
        vm.call_stack.pop();
    }

    #[test]
    fn d2_load_store_load_shared_capture_bool() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let external: StdArc<SharedCell> =
            StdArc::new(SharedCell::new(ValueWord::from_i64(0)));
        // SAFETY: typed init of bool = true.
        unsafe {
            shape_value::v2::closure_raw::write_shared_bool(
                StdArc::as_ptr(&external) as *const SharedCell,
                true,
            );
        }
        let closure_share: StdArc<SharedCell> = StdArc::clone(&external);
        let cell_ptr: *const SharedCell = StdArc::into_raw(closure_share);
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![Upvalue::new(cell_ptr as u64)],
        );

        let load = Instruction::new(
            OpCode::LoadSharedCaptureBool,
            Some(Operand::Local(0)),
        );
        vm.op_load_shared_capture_bool(&load).unwrap();
        // Helper returned true → handler pushed 1 in low byte.
        assert_eq!(vm.pop_raw_u64().unwrap(), 1);

        // Store false.
        vm.push_raw_u64(0).unwrap();
        let store = Instruction::new(
            OpCode::StoreSharedCaptureBool,
            Some(Operand::Local(0)),
        );
        vm.op_store_shared_capture_bool(&store).unwrap();
        vm.op_load_shared_capture_bool(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), 0);

        let observed_false = unsafe {
            shape_value::v2::closure_raw::read_shared_bool(
                StdArc::as_ptr(&external) as *const SharedCell,
            )
        };
        assert!(!observed_false);

        // Edge case: handler treats any non-zero popped slot as true,
        // matching `read_shared_bool`'s "non-zero byte ⇒ true"
        // convention. Verify by pushing a high-byte-only bit pattern.
        vm.push_raw_u64(0x0100_0000_0000_0000).unwrap();
        vm.op_store_shared_capture_bool(&store).unwrap();
        let observed_after = unsafe {
            shape_value::v2::closure_raw::read_shared_bool(
                StdArc::as_ptr(&external) as *const SharedCell,
            )
        };
        // `write_shared_bool` canonicalises to byte 1, so the
        // external read sees true.
        assert!(observed_after);

        // SAFETY: cell_ptr came from Arc::into_raw, one share.
        unsafe { drop(StdArc::from_raw(cell_ptr)) };
        assert_eq!(StdArc::strong_count(&external), 1);
        vm.call_stack.pop();
    }

    #[test]
    fn d2_load_store_load_shared_capture_ptr() {
        let mut vm = fresh_vm_for_capture_opcode_test();

        // The handler treats the 8-byte payload as opaque bits — no
        // retain/release. Use leaked Box<u8> addresses as carrier
        // bits to test the byte-equal round-trip without entangling
        // refcount glue (that's Wave E's IR concern).
        let leak_a: *mut u8 = Box::into_raw(Box::new(0xAAu8));
        let leak_b: *mut u8 = Box::into_raw(Box::new(0xBBu8));
        let bits_initial: u64 = leak_a as u64;
        let bits_new: u64 = leak_b as u64;

        let external: StdArc<SharedCell> =
            StdArc::new(SharedCell::new(ValueWord::from_i64(0)));
        // SAFETY: typed init.
        unsafe {
            shape_value::v2::closure_raw::write_shared_ptr(
                StdArc::as_ptr(&external) as *const SharedCell,
                bits_initial,
            );
        }
        let closure_share: StdArc<SharedCell> = StdArc::clone(&external);
        let cell_ptr: *const SharedCell = StdArc::into_raw(closure_share);
        push_synthetic_frame_with_upvalues(
            &mut vm,
            vec![Upvalue::new(cell_ptr as u64)],
        );

        let load = Instruction::new(
            OpCode::LoadSharedCapturePtr,
            Some(Operand::Local(0)),
        );
        vm.op_load_shared_capture_ptr(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), bits_initial);

        vm.push_raw_u64(bits_new).unwrap();
        let store = Instruction::new(
            OpCode::StoreSharedCapturePtr,
            Some(Operand::Local(0)),
        );
        vm.op_store_shared_capture_ptr(&store).unwrap();

        vm.op_load_shared_capture_ptr(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), bits_new);

        let observed = unsafe {
            shape_value::v2::closure_raw::read_shared_ptr(
                StdArc::as_ptr(&external) as *const SharedCell,
            )
        };
        assert_eq!(observed, bits_new);

        // SAFETY: cell_ptr came from Arc::into_raw, one share. The
        // Box<u8> leaks were minted by Box::into_raw above; we reclaim
        // them exactly once each here.
        unsafe {
            drop(StdArc::from_raw(cell_ptr));
            drop(Box::from_raw(leak_a));
            drop(Box::from_raw(leak_b));
        }
        assert_eq!(StdArc::strong_count(&external), 1);
        vm.call_stack.pop();
    }

    // ── Phase 3c Wave D.1: tests for the typed OwnedMutable capture handlers ──
    //
    // Each test allocates a fresh native typed cell via the matching
    // `closure_raw::alloc_owned_mutable_<kind>` helper and parks the
    // resulting `*mut <T>` raw pointer in a synthetic frame's upvalue
    // slot. The handler reads/writes the cell directly (no Arc, no
    // mutex — OwnedMutable cells are exclusively owned by exactly one
    // closure).
    //
    // The four mandated kinds (I64, F64, Bool, Ptr) cover every
    // corner of the per-FieldKind helper set:
    //   * I64 — full 8-byte signed payload (no sign-extension dance).
    //   * F64 — bitwise-identical f64 round-trip.
    //   * Bool — narrow payload + the "any non-zero byte ⇒ true"
    //          read convention (matches D.2's Shared-Bool tests).
    //   * Ptr — raw 8-byte ValueWord-bits round-trip with NO
    //          retain/release in the handler. We materialise a
    //          ValueWord carrying a heap share, round-trip its raw
    //          bits, and verify that NO refcount imbalance occurred.
    //
    // Each test pattern: Load → assert initial value; Store new value;
    // Load → assert new value; reclaim the cell via `Box::from_raw`
    // matching the helper's allocator; pop the synthetic frame so the
    // VM's own Drop doesn't trip on the synthetic upvalue holding raw
    // pointer bits.

    #[test]
    fn d1_load_store_load_owned_mutable_capture_i64() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let initial: i64 = -1_234_567_890_123;
        let cell: *mut i64 =
            shape_value::v2::closure_raw::alloc_owned_mutable_i64(initial);
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        // Load1 → initial.
        let load = Instruction::new(
            OpCode::LoadOwnedMutableCaptureI64,
            Some(Operand::Local(0)),
        );
        vm.op_load_owned_mutable_capture_i64(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap() as i64, initial);

        // Store new value.
        let new_val: i64 = i64::MAX - 42;
        vm.push_raw_u64(new_val as u64).unwrap();
        let store = Instruction::new(
            OpCode::StoreOwnedMutableCaptureI64,
            Some(Operand::Local(0)),
        );
        vm.op_store_owned_mutable_capture_i64(&store).unwrap();

        // Load2 → new_val.
        vm.op_load_owned_mutable_capture_i64(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap() as i64, new_val);

        // Direct cell observation. SAFETY: cell is live.
        unsafe { assert_eq!(*cell, new_val) };

        // Reclaim. SAFETY: cell came from alloc_owned_mutable_i64.
        unsafe { drop(Box::from_raw(cell)) };
        vm.call_stack.pop();
    }

    #[test]
    fn d1_load_store_load_owned_mutable_capture_f64() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        let initial: f64 = std::f64::consts::PI;
        let cell: *mut f64 =
            shape_value::v2::closure_raw::alloc_owned_mutable_f64(initial);
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        // Load1 → initial.
        let load = Instruction::new(
            OpCode::LoadOwnedMutableCaptureF64,
            Some(Operand::Local(0)),
        );
        vm.op_load_owned_mutable_capture_f64(&load).unwrap();
        assert_eq!(f64::from_bits(vm.pop_raw_u64().unwrap()), initial);

        // Store new value (bitwise-precise f64).
        let new_val: f64 = -2.718281828459045_f64;
        vm.push_raw_u64(new_val.to_bits()).unwrap();
        let store = Instruction::new(
            OpCode::StoreOwnedMutableCaptureF64,
            Some(Operand::Local(0)),
        );
        vm.op_store_owned_mutable_capture_f64(&store).unwrap();

        // Load2 → new_val (bitwise-identical via to_bits / from_bits).
        vm.op_load_owned_mutable_capture_f64(&load).unwrap();
        assert_eq!(
            f64::from_bits(vm.pop_raw_u64().unwrap()).to_bits(),
            new_val.to_bits()
        );

        // Direct cell observation. SAFETY: cell is live.
        unsafe { assert_eq!((*cell).to_bits(), new_val.to_bits()) };

        // Reclaim.
        unsafe { drop(Box::from_raw(cell)) };
        vm.call_stack.pop();
    }

    #[test]
    fn d1_load_store_load_owned_mutable_capture_bool() {
        let mut vm = fresh_vm_for_capture_opcode_test();
        // Initialise to `true`; flip to `false`; then flip back via a
        // non-1 nonzero pop pattern to exercise the "any nonzero ⇒ true"
        // reader contract.
        let cell: *mut bool =
            shape_value::v2::closure_raw::alloc_owned_mutable_bool(true);
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        let load = Instruction::new(
            OpCode::LoadOwnedMutableCaptureBool,
            Some(Operand::Local(0)),
        );
        let store = Instruction::new(
            OpCode::StoreOwnedMutableCaptureBool,
            Some(Operand::Local(0)),
        );

        // Load1 → true (encoded as 1u64 in the slot).
        vm.op_load_owned_mutable_capture_bool(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), 1);

        // Store false (push 0u64).
        vm.push_raw_u64(0).unwrap();
        vm.op_store_owned_mutable_capture_bool(&store).unwrap();

        // Load2 → false.
        vm.op_load_owned_mutable_capture_bool(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), 0);

        // Store true via a non-1 nonzero u64 (e.g. 0xDEAD_BEEF). The
        // handler treats any nonzero as true.
        vm.push_raw_u64(0xDEAD_BEEF).unwrap();
        vm.op_store_owned_mutable_capture_bool(&store).unwrap();

        // Load3 → true (encoded back as 1u64).
        vm.op_load_owned_mutable_capture_bool(&load).unwrap();
        assert_eq!(vm.pop_raw_u64().unwrap(), 1);

        // Direct cell observation. SAFETY: cell is live.
        unsafe { assert!(*cell) };

        // Reclaim.
        unsafe { drop(Box::from_raw(cell)) };
        vm.call_stack.pop();
    }

    #[test]
    fn d1_load_store_load_owned_mutable_capture_ptr_no_arc_imbalance() {
        // Materialise a heap-tagged ValueWord (a `String` share),
        // park its raw u64 bits inside a Ptr cell, exercise the
        // Load/Store handlers, and verify the strong count of the
        // outside-held Arc is unchanged across each handler call.
        //
        // Per `read_owned_mutable_ptr` / `write_owned_mutable_ptr`'s
        // contract — and matching D.2's `op_load_shared_capture_ptr`
        // — the handlers must NOT clone/retain the bits on Load and
        // must NOT release the previous payload on Store. Refcount
        // accounting is the IR's responsibility (Wave E pairs Load
        // with `vw_clone` and Store with `vw_drop`, mirroring the
        // c-stdlib-msgpack pattern from commit afb1651).
        //
        // We use `ValueWord::from_string(Arc<String>)` for the
        // payload — the Arc strong count is the load-bearing
        // refcount we observe before/after each handler call. Long
        // strings (> 32 bytes) bypass `string_intern`'s pool so the
        // counts remain predictable.

        let mut vm = fresh_vm_for_capture_opcode_test();

        // Mint share #1 of `payload_a` and freeze its bits.
        let payload_a: StdArc<String> = StdArc::new(
            "hello-d1-ptr-a-payload-longer-than-intern-threshold".to_string(),
        );
        let initial_bits = ValueWord::from_string(payload_a.clone()).into_raw_bits();
        // payload_a strong count: 1 (external) + 1 (initial_bits) = 2.
        let baseline_a = StdArc::strong_count(&payload_a);
        assert_eq!(baseline_a, 2);

        // Park initial_bits in a fresh Ptr cell.
        let cell: *mut u64 = shape_value::v2::closure_raw::alloc_owned_mutable_ptr(initial_bits);
        push_synthetic_frame_with_upvalues(&mut vm, vec![Upvalue::new(cell as u64)]);

        let load = Instruction::new(
            OpCode::LoadOwnedMutableCapturePtr,
            Some(Operand::Local(0)),
        );
        let store = Instruction::new(
            OpCode::StoreOwnedMutableCapturePtr,
            Some(Operand::Local(0)),
        );

        // ── Load1: handler must not clone. Strong count unchanged. ──
        vm.op_load_owned_mutable_capture_ptr(&load).unwrap();
        let loaded_bits_1 = vm.pop_raw_u64().unwrap();
        assert_eq!(
            StdArc::strong_count(&payload_a),
            baseline_a,
            "LoadPtr must not clone the heap share"
        );
        // Loaded bits are byte-equal to initial_bits.
        assert_eq!(loaded_bits_1, initial_bits);

        // ── Store: handler must not release the previous payload. ──
        let payload_b: StdArc<String> = StdArc::new(
            "hello-d1-ptr-b-payload-longer-than-intern-threshold".to_string(),
        );
        let new_bits = ValueWord::from_string(payload_b.clone()).into_raw_bits();
        let baseline_b = StdArc::strong_count(&payload_b);
        assert_eq!(baseline_b, 2);

        vm.push_raw_u64(new_bits).unwrap();
        vm.op_store_owned_mutable_capture_ptr(&store).unwrap();
        // payload_b strong count: still 2 (external + cell-resident).
        assert_eq!(
            StdArc::strong_count(&payload_b),
            baseline_b,
            "StorePtr must not retain the new share"
        );
        // payload_a strong count: ALSO still 2 — the handler did NOT
        // release the previous payload (the original initial_bits
        // share is now leaked by the cell overwrite; we reclaim it
        // explicitly below).
        assert_eq!(
            StdArc::strong_count(&payload_a),
            baseline_a,
            "StorePtr must not release the previous share"
        );

        // ── Load2: handler must not clone the new payload. ──
        vm.op_load_owned_mutable_capture_ptr(&load).unwrap();
        let loaded_bits_2 = vm.pop_raw_u64().unwrap();
        assert_eq!(loaded_bits_2, new_bits);
        assert_eq!(
            StdArc::strong_count(&payload_b),
            baseline_b,
            "LoadPtr must not clone the heap share"
        );

        // ── Cleanup. ──
        // ValueWord is a u64 alias with no Drop glue — releasing a
        // share requires an explicit `vw_drop` of the raw bits.
        // 1. The cell currently holds `new_bits` (one share of
        //    payload_b). vw_drop the bits to release the cell's share.
        // 2. The original `initial_bits` share of payload_a was
        //    leaked when the Store overwrote the cell. vw_drop the
        //    leaked bits.
        // 3. Reclaim the Box<u64> via Box::from_raw.
        use shape_value::value_word_drop::vw_drop;
        // SAFETY: cell is live; `*cell` reads the current bits.
        let live_cell_bits = unsafe { *cell };
        vw_drop(live_cell_bits);
        // payload_b: 2 → 1 (only `external` remains).
        assert_eq!(StdArc::strong_count(&payload_b), 1);
        vw_drop(initial_bits);
        // payload_a: 2 → 1 (only `external` remains).
        assert_eq!(StdArc::strong_count(&payload_a), 1);

        // SAFETY: cell came from `alloc_owned_mutable_ptr` =
        // `Box::into_raw(Box::new(initial_bits: u64))`. Reclaim once.
        unsafe { drop(Box::from_raw(cell)) };
        vm.call_stack.pop();
    }
}
