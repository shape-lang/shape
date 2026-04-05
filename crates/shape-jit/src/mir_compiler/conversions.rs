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
    /// - I8 (bool) → I64: select TAG_BOOL_TRUE/FALSE
    /// - I64 (already NaN-boxed) → I64: no-op
    pub(crate) fn box_to_nanboxed(&mut self, val: Value, kind: SlotKind) -> Value {
        match kind {
            SlotKind::Float64 => {
                // f64 bits are already valid NaN-boxed (values < TAG_BASE)
                self.builder
                    .ins()
                    .bitcast(types::I64, MemFlags::new(), val)
            }
            SlotKind::Int32 | SlotKind::UInt32 => {
                // i32 → sign-extend to i64, then apply INT NaN-box tag
                let extended = self.builder.ins().sextend(types::I64, val);
                let payload_mask = self
                    .builder
                    .ins()
                    .iconst(types::I64, shape_value::tags::PAYLOAD_MASK as i64);
                let payload = self.builder.ins().band(extended, payload_mask);
                let int_tag = self.builder.ins().iconst(
                    types::I64,
                    (shape_value::tags::TAG_BASE
                        | (shape_value::tags::TAG_INT << shape_value::tags::TAG_SHIFT))
                        as i64,
                );
                self.builder.ins().bor(int_tag, payload)
            }
            SlotKind::Bool | SlotKind::Int8 | SlotKind::UInt8 => {
                // i8 → select TAG_BOOL_TRUE or TAG_BOOL_FALSE
                let true_val = self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_BOOL_TRUE as i64);
                let false_val = self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_BOOL_FALSE as i64);
                self.builder.ins().select(val, true_val, false_val)
            }
            SlotKind::Int16 | SlotKind::UInt16 => {
                // i16 → sign-extend to i64, then INT NaN-box tag
                let extended = self.builder.ins().sextend(types::I64, val);
                let payload_mask = self
                    .builder
                    .ins()
                    .iconst(types::I64, shape_value::tags::PAYLOAD_MASK as i64);
                let payload = self.builder.ins().band(extended, payload_mask);
                let int_tag = self.builder.ins().iconst(
                    types::I64,
                    (shape_value::tags::TAG_BASE
                        | (shape_value::tags::TAG_INT << shape_value::tags::TAG_SHIFT))
                        as i64,
                );
                self.builder.ins().bor(int_tag, payload)
            }
            // Already I64 NaN-boxed
            _ => val,
        }
    }

    /// Convert a NaN-boxed I64 to a native type for storing into a typed local.
    ///
    /// - I64 → F64: bitcast (raw bits → f64)
    /// - I64 → I32: extract INT payload, ireduce
    /// - I64 → I8 (bool): compare against TAG_BOOL_TRUE
    /// - I64 → I64: no-op
    pub(crate) fn unbox_from_nanboxed(&mut self, val: Value, kind: SlotKind) -> Value {
        match kind {
            SlotKind::Float64 => {
                self.builder
                    .ins()
                    .bitcast(types::F64, MemFlags::new(), val)
            }
            SlotKind::Int32 | SlotKind::UInt32 => {
                // Extract 48-bit payload, sign-extend, ireduce to i32
                let shifted = self.builder.ins().ishl_imm(val, 16);
                let sign_ext = self.builder.ins().sshr_imm(shifted, 16);
                self.builder.ins().ireduce(types::I32, sign_ext)
            }
            SlotKind::Bool | SlotKind::Int8 | SlotKind::UInt8 => {
                let tag_true = self
                    .builder
                    .ins()
                    .iconst(types::I64, crate::nan_boxing::TAG_BOOL_TRUE as i64);
                // icmp returns I8 (0 or 1)
                self.builder.ins().icmp(IntCC::Equal, val, tag_true)
            }
            SlotKind::Int16 | SlotKind::UInt16 => {
                let shifted = self.builder.ins().ishl_imm(val, 16);
                let sign_ext = self.builder.ins().sshr_imm(shifted, 16);
                self.builder.ins().ireduce(types::I16, sign_ext)
            }
            // Already I64
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

    /// Ensure a value is NaN-boxed I64. If it's already I64, no-op.
    /// If it's a native type (F64, I32, I8), box it.
    pub(crate) fn ensure_nanboxed(&mut self, val: Value) -> Value {
        let kind = self.infer_kind_from_value(val);
        self.box_to_nanboxed(val, kind)
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
