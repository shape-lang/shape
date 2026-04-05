//! Type conversion helpers: box/unbox between native and NaN-boxed values.
//!
//! v2-boundary: This entire module exists because FFI boundaries, function
//! calls, and returns still use NaN-boxed I64 format. Once all FFI functions
//! accept native types and the callee ABI is typed, these conversions can be
//! deleted. Inside native-typed locals, values already use their natural
//! Cranelift representation (F64, I32, I8, etc.).

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::type_tracking::SlotKind;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Convert a native-typed value to NaN-boxed I64 for FFI boundaries.
    ///
    /// - F64 → I64: bitcast (raw f64 bits are valid NaN-boxed floats)
    /// - I32 → I64: sign-extend + apply INT tag
    /// Convert native value to raw I64 bits (v2: NO NaN-boxing tags).
    /// - F64 → bitcast to I64 (raw IEEE 754 bits)
    /// - I32 → sign-extend to I64
    /// - I8 (bool) → zero-extend to I64 (0 or 1)
    /// - I16 → sign-extend to I64
    /// - I64 → no-op
    pub(crate) fn box_to_nanboxed(&mut self, val: Value, kind: SlotKind) -> Value {
        match kind {
            SlotKind::Float64 => {
                self.builder
                    .ins()
                    .bitcast(types::I64, MemFlags::new(), val)
            }
            SlotKind::Int32 | SlotKind::UInt32 => {
                self.builder.ins().sextend(types::I64, val)
            }
            SlotKind::Bool | SlotKind::Int8 | SlotKind::UInt8 => {
                self.builder.ins().uextend(types::I64, val)
            }
            SlotKind::Int16 | SlotKind::UInt16 => {
                self.builder.ins().sextend(types::I64, val)
            }
            // Already I64
            _ => val,
        }
    }

    /// Convert a NaN-boxed I64 to a native type for storing into a typed local.
    ///
    /// Convert raw I64 bits to native type (v2: NO NaN-boxing tag extraction).
    /// - I64 → F64: bitcast (raw bits → f64)
    /// - I64 → I32: ireduce (truncate to 32 bits)
    /// - I64 → I8 (bool): ireduce (truncate to 8 bits)
    /// - I64 → I64: no-op
    pub(crate) fn unbox_from_nanboxed(&mut self, val: Value, kind: SlotKind) -> Value {
        match kind {
            SlotKind::Float64 => {
                self.builder
                    .ins()
                    .bitcast(types::F64, MemFlags::new(), val)
            }
            SlotKind::Int32 | SlotKind::UInt32 => {
                self.builder.ins().ireduce(types::I32, val)
            }
            SlotKind::Bool | SlotKind::Int8 | SlotKind::UInt8 => {
                self.builder.ins().ireduce(types::I8, val)
            }
            SlotKind::Int16 | SlotKind::UInt16 => {
                self.builder.ins().ireduce(types::I16, val)
            }
            _ => val,
        }
    }

    /// Get the SlotKind for a given SlotId.
    pub(crate) fn slot_kind_of(&self, slot: shape_vm::mir::types::SlotId) -> SlotKind {
        super::types::slot_kind_for_local(&self.slot_kinds, slot.0)
    }

    /// Check if a slot is native-typed (non-NaN-boxed).
    pub(crate) fn is_native_slot(&self, slot: shape_vm::mir::types::SlotId) -> bool {
        super::types::is_native_slot(self.slot_kind_of(slot))
    }

    /// Infer a SlotKind from a Cranelift value's type.
    /// Used when we have a value but not its source SlotKind.
    pub(crate) fn infer_kind_from_value(&self, val: Value) -> SlotKind {
        let cl_type = self.builder.func.dfg.value_type(val);
        if cl_type == types::F64 {
            SlotKind::Float64
        } else if cl_type == types::I32 {
            SlotKind::Int32
        } else if cl_type == types::I8 {
            SlotKind::Bool
        } else if cl_type == types::I16 {
            SlotKind::Int16
        } else {
            SlotKind::Unknown // I64 = NaN-boxed
        }
    }

    /// Convert any native value to raw I64 bits WITHOUT NaN-boxing tags (v2).
    /// - F64 → bitcast to I64 (raw IEEE 754 bits)
    /// - I32 → sign-extend to I64
    /// - I8 (bool) → zero-extend to I64 (0 or 1)
    /// - I64 → no-op
    pub(crate) fn ensure_nanboxed(&mut self, val: Value) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::I64 {
            val
        } else if val_type == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
        } else if val_type == types::I32 {
            self.builder.ins().sextend(types::I64, val)
        } else if val_type == types::I8 {
            self.builder.ins().uextend(types::I64, val)
        } else if val_type == types::I16 {
            self.builder.ins().sextend(types::I64, val)
        } else {
            val
        }
    }

    /// Ensure a value matches a target SlotKind's Cranelift type.
    /// Handles: NaN-boxed → native (unbox), native → NaN-boxed (box), or same type (no-op).
    pub(crate) fn ensure_kind(&mut self, val: Value, target_kind: SlotKind) -> Value {
        let target_cl_type = super::types::cranelift_type_for_slot(target_kind);
        let val_type = self.builder.func.dfg.value_type(val);

        if val_type == target_cl_type {
            return val; // Already correct type
        }

        if val_type == types::I64 {
            // NaN-boxed → native
            self.unbox_from_nanboxed(val, target_kind)
        } else if target_cl_type == types::I64 {
            // Native → NaN-boxed
            let source_kind = self.infer_kind_from_value(val);
            self.box_to_nanboxed(val, source_kind)
        } else {
            // Different native types — route through NaN-boxed
            let source_kind = self.infer_kind_from_value(val);
            self.convert_between(val, source_kind, target_kind)
        }
    }

    /// Convert a value from one SlotKind representation to another.
    /// Used when assigning from a source with one kind to a target with another.
    pub(crate) fn convert_between(
        &mut self,
        val: Value,
        from: SlotKind,
        to: SlotKind,
    ) -> Value {
        if from == to {
            return val;
        }
        // Route through NaN-boxed as intermediate
        let nanboxed = self.box_to_nanboxed(val, from);
        self.unbox_from_nanboxed(nanboxed, to)
    }
}
