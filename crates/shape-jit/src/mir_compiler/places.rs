//! Place resolution: MIR Place → Cranelift Value.
//!
//! A Place represents something that can be read from or written to:
//! - `Place::Local(slot)` → Cranelift variable
//! - `Place::Field(base, idx)` → FFI call to get/set field
//! - `Place::Index(base, operand)` → **inline** array access (no FFI call)

use cranelift::prelude::*;

use super::MirToIR;
use crate::nan_boxing::{UNIFIED_PTR_MASK, JIT_ALLOC_DATA_OFFSET};
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    // ── Inline array access helpers ──────────────────────────────────────
    // Ported from BytecodeToIR::inline_ops.rs for the MirToIR path.
    // These bypass FFI calls and emit direct Cranelift memory loads,
    // eliminating ~50-100ns per array access in hot loops.

    /// Extract the raw heap pointer from a NaN-boxed heap value.
    /// Masks off tag bits and the unified heap flag (bit 47).
    #[inline]
    fn emit_payload_ptr(&mut self, boxed: Value) -> Value {
        let ptr_mask = self.builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
        self.builder.ins().band(boxed, ptr_mask)
    }

    /// Get pointer to the JitArray/UnifiedArray data fields (past 8-byte header).
    #[inline]
    fn emit_array_ptr(&mut self, arr_boxed: Value) -> Value {
        let alloc_ptr = self.emit_payload_ptr(arr_boxed);
        self.builder.ins().iadd_imm(alloc_ptr, JIT_ALLOC_DATA_OFFSET as i64)
    }

    /// Load (data_ptr, length) from a JitArray/UnifiedArray.
    /// JitArray layout after header: data_ptr at +0, len at +8.
    #[inline]
    fn emit_array_data_and_len(&mut self, arr_boxed: Value) -> (Value, Value) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self.builder.ins().load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self.builder.ins().load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        (data_ptr, length)
    }

    /// Convert a NaN-boxed index to a raw i64.
    /// Handles both NaN-boxed f64 (number) and NaN-boxed i48 (int).
    fn emit_index_to_i64(&mut self, index_bits: Value) -> Value {
        // If bits < TAG_BASE (0xFFF8...), it's a raw f64 — bitcast and convert.
        // If bits >= TAG_BASE, it's a tagged value (int) — extract i48 payload.
        // For performance, we use bitcast → fcvt which handles the common f64 case.
        // For NaN-boxed ints, fcvt_to_sint_sat on NaN gives 0, so we also extract
        // the int payload and select based on a check.
        let tag_base = self.builder.ins().iconst(types::I64, 0xFFF8_0000_0000_0000u64 as i64);
        let is_tagged = self.builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index_bits, tag_base);

        // Float path: bitcast to f64, convert to i64
        let as_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), index_bits);
        let from_float = self.builder.ins().fcvt_to_sint_sat(types::I64, as_f64);

        // Int path: sign-extend lower 48 bits
        let shifted_left = self.builder.ins().ishl_imm(index_bits, 16);
        let from_int = self.builder.ins().sshr_imm(shifted_left, 16);

        // Select: if tagged (int), use int extraction; else use float conversion
        self.builder.ins().select(is_tagged, from_int, from_float)
    }

    /// Normalize negative array index: if idx < 0, idx = length + idx.
    #[inline]
    fn normalize_index(&mut self, idx: Value, length: Value) -> Value {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let is_negative = self.builder.ins().icmp(IntCC::SignedLessThan, idx, zero);
        let adjusted = self.builder.ins().iadd(length, idx);
        self.builder.ins().select(is_negative, adjusted, idx)
    }

    /// Bounds check: if index >= length (unsigned), return 0 (safe default).
    /// Using unsigned comparison catches both negative (wrapped) and too-large indices.
    #[inline]
    fn bounds_check(&mut self, index: Value, length: Value) -> Value {
        let in_bounds = self.builder.ins().icmp(IntCC::UnsignedLessThan, index, length);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().select(in_bounds, index, zero)
    }

    /// Inline array element read: arr[index] → direct memory load.
    /// ~8 Cranelift instructions instead of an FFI call.
    fn inline_array_get(&mut self, arr_boxed: Value, index_bits: Value) -> Value {
        let (data_ptr, length) = self.emit_array_data_and_len(arr_boxed);
        let idx_i64 = self.emit_index_to_i64(index_bits);
        let final_idx = self.normalize_index(idx_i64, length);
        let safe_idx = self.bounds_check(final_idx, length);

        // Element address: data_ptr + safe_idx * 8 (u64 slots)
        let byte_offset = self.builder.ins().ishl_imm(safe_idx, 3); // * 8
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().load(types::I64, MemFlags::trusted(), elem_addr, 0)
    }

    /// Inline array element write: arr[index] = value → direct memory store.
    fn inline_array_set(&mut self, arr_boxed: Value, index_bits: Value, val: Value) {
        let (data_ptr, length) = self.emit_array_data_and_len(arr_boxed);
        let idx_i64 = self.emit_index_to_i64(index_bits);
        let final_idx = self.normalize_index(idx_i64, length);
        let safe_idx = self.bounds_check(final_idx, length);

        let byte_offset = self.builder.ins().ishl_imm(safe_idx, 3);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().store(MemFlags::trusted(), val, elem_addr, 0);
    }

    // ── Field offset resolution ────────────────────────────────────────

    fn try_resolve_field_byte_offset(&self, field_idx: &FieldIdx) -> Option<u16> {
        let name = self.mir.field_name_table.get(field_idx)?;
        self.field_byte_offsets.get(name).copied()
    }

    fn field_idx_to_boxed_key(&self, field_idx: &FieldIdx) -> Option<u64> {
        let name = self.mir.field_name_table.get(field_idx)?;
        Some(crate::nan_boxing::box_string(name.clone()))
    }

    // ── Place resolution ─────────────────────────────────────────────────

    /// Read a value from a Place.
    pub(crate) fn read_place(&mut self, place: &Place) -> Result<Value, String> {
        match place {
            Place::Local(slot) => {
                let var = self.locals.get(slot).ok_or_else(|| {
                    format!("MirToIR: unknown local slot {}", slot)
                })?;
                Ok(self.builder.use_var(*var))
            }
            Place::Field(base, field_idx) => {
                let raw_base = self.read_place(base)?;
                // FFI field access expects NaN-boxed I64
                let base_val = self.ensure_nanboxed(raw_base);
                if let Some(byte_off) = self.try_resolve_field_byte_offset(field_idx) {
                    let offset = self.builder.ins().iconst(types::I64, byte_off as i64);
                    let inst = self.builder.ins().call(self.ffi.typed_object_get_field, &[base_val, offset]);
                    Ok(self.builder.inst_results(inst)[0])
                } else if let Some(boxed_key) = self.field_idx_to_boxed_key(field_idx) {
                    let key = self.builder.ins().iconst(types::I64, boxed_key as i64);
                    let inst = self.builder.ins().call(self.ffi.get_prop, &[base_val, key]);
                    Ok(self.builder.inst_results(inst)[0])
                } else {
                    let field = self.builder.ins().iconst(types::I64, field_idx.0 as i64);
                    let inst = self.builder.ins().call(self.ffi.get_prop, &[base_val, field]);
                    Ok(self.builder.inst_results(inst)[0])
                }
            }
            Place::Index(base, operand) => {
                let raw_base = self.read_place(base)?;
                // Inline array access operates on NaN-boxed values
                let base_val = self.ensure_nanboxed(raw_base);
                let raw_idx = self.compile_operand_raw(operand)?;
                let index_val = self.ensure_nanboxed(raw_idx);
                Ok(self.inline_array_get(base_val, index_val))
            }
            Place::Deref(inner) => {
                let ref_addr = self.read_place(inner)?;
                Ok(self.builder.ins().load(types::I64, MemFlags::new(), ref_addr, 0))
            }
        }
    }

    /// Write a value to a Place, converting to the slot's native type if needed.
    pub(crate) fn write_place(
        &mut self,
        place: &Place,
        val: Value,
    ) -> Result<(), String> {
        match place {
            Place::Local(slot) => {
                let target_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
                let var = *self.locals.get(slot).ok_or_else(|| {
                    format!("MirToIR: unknown local slot {}", slot)
                })?;
                // Convert value to match the slot's declared Cranelift type.
                let converted = self.ensure_kind(val, target_kind);
                self.builder.def_var(var, converted);
                Ok(())
            }
            Place::Field(base, field_idx) => {
                let raw_base = self.read_place(base)?;
                let base_val = self.ensure_nanboxed(raw_base);
                let boxed_val = self.ensure_nanboxed(val);
                if let Some(byte_off) = self.try_resolve_field_byte_offset(field_idx) {
                    let offset = self.builder.ins().iconst(types::I64, byte_off as i64);
                    self.builder.ins().call(self.ffi.typed_object_set_field, &[base_val, offset, boxed_val]);
                } else if let Some(boxed_key) = self.field_idx_to_boxed_key(field_idx) {
                    let key = self.builder.ins().iconst(types::I64, boxed_key as i64);
                    self.builder.ins().call(self.ffi.set_prop, &[base_val, key, boxed_val]);
                } else {
                    let field = self.builder.ins().iconst(types::I64, field_idx.0 as i64);
                    self.builder.ins().call(self.ffi.set_prop, &[base_val, field, boxed_val]);
                }
                Ok(())
            }
            Place::Index(base, operand) => {
                let raw_base = self.read_place(base)?;
                let base_val = self.ensure_nanboxed(raw_base);
                let raw_idx = self.compile_operand_raw(operand)?;
                let index_val = self.ensure_nanboxed(raw_idx);
                let boxed_val = self.ensure_nanboxed(val);
                self.inline_array_set(base_val, index_val, boxed_val);
                Ok(())
            }
            Place::Deref(inner) => {
                let ref_addr = self.read_place(inner)?;
                let boxed_val = self.ensure_nanboxed(val);
                self.builder.ins().store(MemFlags::new(), boxed_val, ref_addr, 0);
                Ok(())
            }
        }
    }

    /// Write zero/null to a Place's root local.
    /// Used after Move to prevent double-drop.
    /// Uses type-appropriate zero for native slots (0.0 for F64, 0 for I32, etc.)
    pub(crate) fn null_place(&mut self, place: &Place) -> Result<(), String> {
        let slot = place.root_local();
        // Only null the root local for simple locals.
        // Field/Index moves don't null the entire container.
        if matches!(place, Place::Local(_)) {
            let var = self.locals.get(&slot).ok_or_else(|| {
                format!("MirToIR: unknown local slot {}", slot)
            })?;
            let kind = self.slot_kind_of(slot);
            let null = match kind {
                shape_vm::type_tracking::SlotKind::Float64 => {
                    self.builder.ins().f64const(0.0)
                }
                shape_vm::type_tracking::SlotKind::Int32
                | shape_vm::type_tracking::SlotKind::UInt32 => {
                    self.builder.ins().iconst(types::I32, 0)
                }
                shape_vm::type_tracking::SlotKind::Bool
                | shape_vm::type_tracking::SlotKind::Int8
                | shape_vm::type_tracking::SlotKind::UInt8 => {
                    self.builder.ins().iconst(types::I8, 0)
                }
                shape_vm::type_tracking::SlotKind::Int16
                | shape_vm::type_tracking::SlotKind::UInt16 => {
                    self.builder.ins().iconst(types::I16, 0)
                }
                _ => self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64),
            };
            self.builder.def_var(*var, null);
        }
        Ok(())
    }
}
