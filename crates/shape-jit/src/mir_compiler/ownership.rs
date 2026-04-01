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
                    let type_info = self
                        .local_types
                        .get(slot.0 as usize)
                        .cloned()
                        .unwrap_or(LocalTypeInfo::Unknown);
                    if super::types::is_heap_type(&type_info) {
                        // arc_retain increments the Arc refcount.
                        // For non-heap values, arc_retain is a no-op.
                        self.builder.ins().call(self.ffi.arc_retain, &[val]);
                    } else if matches!(type_info, LocalTypeInfo::Unknown) {
                        // Unknown type: conservatively retain (arc_retain handles
                        // non-heap values as no-ops via tag classification).
                        self.builder.ins().call(self.ffi.arc_retain, &[val]);
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
    pub(crate) fn compile_constant(&mut self, constant: &MirConstant) -> Result<Value, String> {
        match constant {
            MirConstant::Int(n) => {
                // NaN-box the integer: tag with INT_TAG.
                let boxed = shape_value::ValueWord::from_i64(*n).raw_bits();
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::Float(bits) => {
                // Float is stored as raw f64 bits — already NaN-boxed.
                Ok(self.builder.ins().iconst(types::I64, *bits as i64))
            }
            MirConstant::Bool(b) => {
                let boxed = if *b {
                    crate::nan_boxing::TAG_BOOL_TRUE
                } else {
                    crate::nan_boxing::TAG_BOOL_FALSE
                };
                Ok(self.builder.ins().iconst(types::I64, boxed as i64))
            }
            MirConstant::None => {
                Ok(self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64))
            }
            MirConstant::StringId(id) => {
                // String constants are heap-allocated. Use the FFI to materialize.
                // For now, create a tagged string pointer via the string table.
                // This requires runtime support — emit a call to jit_intern_string.
                let str_id = self.builder.ins().iconst(types::I64, *id as i64);
                // Use generic_builtin for string materialization
                // TODO: Direct string constant support
                Ok(str_id)
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
                // Method name for dispatch — encoded as a string ID.
                let _ = name;
                let zero = self.builder.ins().iconst(types::I64, 0);
                Ok(zero)
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
        let val = self.read_place(place)?;

        {
            let slot = place.root_local();
            let type_info = self
                .local_types
                .get(slot.0 as usize)
                .cloned()
                .unwrap_or(LocalTypeInfo::Unknown);

            if super::types::is_heap_type(&type_info) {
                // Known heap type: always release.
                self.builder.ins().call(self.ffi.arc_release, &[val]);
            } else if matches!(type_info, LocalTypeInfo::Unknown) {
                // Unknown type: arc_release handles non-heap values as no-ops.
                self.builder.ins().call(self.ffi.arc_release, &[val]);
            }
            // Copy types: no-op (no refcount to manage).
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
