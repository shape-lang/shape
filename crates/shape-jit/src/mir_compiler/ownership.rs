//! Ownership-aware codegen: Move, Copy, Drop.
//!
//! This is the core of what makes MirToIR correct where BytecodeToIR isn't:
//! - Move: read value, null source slot (prevents double-drop)
//! - Copy: read value, arc_retain if heap type (Arc::clone)
//! - Drop: arc_release for heap types, no-op for primitives

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile an Operand, respecting Move/Copy ownership semantics.
    pub(crate) fn compile_operand(&mut self, operand: &Operand) -> Result<Value, String> {
        match operand {
            Operand::Move(place) | Operand::MoveExplicit(place) => {
                // Move: read the value, then null the source to prevent double-drop.
                let val = self.read_place(place)?;
                self.null_place(place)?;
                Ok(val)
            }
            Operand::Copy(place) => {
                // Copy: read the value. For heap types, increment the refcount.
                let val = self.read_place(place)?;
                {
                    let slot = place.root_local();
                    let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
                    // Native primitive types (Float64, Int32, Bool) never need refcounting.
                    if !super::types::is_native_slot(slot_kind) {
                        let type_info = self
                            .local_types
                            .get(slot.0 as usize)
                            .cloned()
                            .unwrap_or(LocalTypeInfo::Unknown);
                        if super::types::is_heap_type(&type_info) {
                            self.builder.ins().call(self.ffi.arc_retain, &[val]);
                        } else if matches!(type_info, LocalTypeInfo::Unknown) {
                            self.builder.ins().call(self.ffi.arc_retain, &[val]);
                        }
                    }
                }
                Ok(val)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
        }
    }

    /// Compile an operand without ownership tracking (raw value access).
    /// Used for index operands in Place::Index where we just need the value.
    pub(crate) fn compile_operand_raw(&mut self, operand: &Operand) -> Result<Value, String> {
        match operand {
            Operand::Move(place) | Operand::MoveExplicit(place) | Operand::Copy(place) => {
                self.read_place(place)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
        }
    }

    /// Compile a MIR constant to a Cranelift value.
    ///
    /// Returns native types when possible (F64 for floats, I64 for ints, I8 for bools).
    /// Consumers that need NaN-boxed I64 (FFI calls) use `ensure_nanboxed()`.
    /// v2-boundary: Int, None, StringId, Str, Function, Method, ClosurePlaceholder
    /// all produce NaN-boxed I64 because the VM stack and FFI boundaries expect it.
    pub(crate) fn compile_constant(&mut self, constant: &MirConstant) -> Result<Value, String> {
        match constant {
            MirConstant::Int(n) => {
                // NaN-box the integer (I64 can't distinguish native from NaN-boxed).
                let boxed = shape_value::ValueWord::from_i64(*n).raw_bits();
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::Float(bits) => {
                // Native F64 — direct float constant. ~100x faster than FFI path.
                Ok(self.builder.ins().f64const(f64::from_bits(*bits)))
            }
            MirConstant::Bool(b) => {
                // Native I8 bool — 0 or 1.
                Ok(self.builder.ins().iconst(types::I8, *b as i64))
            }
            MirConstant::None => {
                Ok(self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64))
            }
            MirConstant::StringId(id) => {
                // Look up the string from the string table and NaN-box it at compile time.
                let idx = *id as usize;
                if idx < self.strings.len() {
                    let s = self.strings[idx].clone();
                    let boxed = crate::nan_boxing::box_string(s);
                    Ok(self.builder.ins().iconst(types::I64, boxed as i64))
                } else {
                    Ok(self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64))
                }
            }
            MirConstant::Str(s) => {
                // String literal carried in MIR — NaN-box at compile time.
                let boxed = crate::nan_boxing::box_string(s.clone());
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::Function(name) => {
                // Resolve function name to index, NaN-box as function ref
                if let Some(&idx) = self.function_indices.get(name.as_str()) {
                    let boxed = shape_value::ValueWord::from_function(idx).raw_bits();
                    Ok(self.builder.ins().iconst(types::I64, boxed as i64))
                } else {
                    Ok(self.builder.ins().iconst(types::I64, crate::nan_boxing::TAG_NULL as i64))
                }
            }
            MirConstant::Method(name) => {
                // Method name for dispatch — NaN-box the string at compile time.
                let boxed = crate::nan_boxing::box_string(name.clone());
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::ClosurePlaceholder => {
                // Should have been patched to Function(name) during bytecode compilation.
                // If we reach here, it means the patching didn't happen — return TAG_NULL.
                Ok(self.builder.ins().iconst(types::I64, crate::nan_boxing::TAG_NULL as i64))
            }
        }
    }

    /// Emit Drop for a local: release refcount if it's a heap type.
    pub(crate) fn emit_drop(&mut self, place: &Place) -> Result<(), String> {
        let slot = place.root_local();
        let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);

        // Native primitive types never need refcounting.
        if !super::types::is_native_slot(slot_kind) {
            let val = self.read_place(place)?;
            let type_info = self
                .local_types
                .get(slot.0 as usize)
                .cloned()
                .unwrap_or(LocalTypeInfo::Unknown);

            if super::types::is_heap_type(&type_info) {
                self.builder.ins().call(self.ffi.arc_release, &[val]);
            } else if matches!(type_info, LocalTypeInfo::Unknown) {
                self.builder.ins().call(self.ffi.arc_release, &[val]);
            }
        }

        // Null the slot to prevent use-after-drop.
        self.null_place(place)?;
        Ok(())
    }

    /// Release the old value of a local before overwriting it.
    /// This prevents Arc leaks when a heap local is reassigned.
    pub(crate) fn release_old_value_if_heap(
        &mut self,
        place: &Place,
    ) -> Result<(), String> {
        let slot = place.root_local();
        if matches!(place, Place::Local(_)) {
            let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
            // Native primitive types never need refcounting.
            if super::types::is_native_slot(slot_kind) {
                return Ok(());
            }

            let type_info = self
                .local_types
                .get(slot.0 as usize)
                .cloned()
                .unwrap_or(LocalTypeInfo::Unknown);

            if super::types::is_heap_type(&type_info)
                || matches!(type_info, LocalTypeInfo::Unknown)
            {
                let old_val = self.read_place(place)?;
                self.builder.ins().call(self.ffi.arc_release, &[old_val]);
            }
        }
        Ok(())
    }
}
