//! Slot type coercion helpers for native-typed v2 locals.
//!
//! After R4.3 these helpers are the last residue of the JIT's old
//! box/unbox layer. The three NaN-box conversion helpers that used to
//! live here (one native→I64 widener, one I64→native narrower, one
//! identity-to-I64 wrapper) were deleted once every external caller
//! moved to native FFI signatures, inline width extensions at VM-stack
//! push sites, or typed StackSlot storage for borrow/deref.
//!
//! What remains is a single Cranelift widen/narrow/bitcast path
//! (`ensure_kind`) and a slot-kind lookup (`slot_kind_of`). The path
//! from I64 ↔ native is a plain width conversion: there is no tag
//! manipulation, no NaN-box bit pattern, no ValueWord construction.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::type_tracking::SlotKind;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Get the SlotKind for a given SlotId.
    pub(crate) fn slot_kind_of(&self, slot: shape_vm::mir::types::SlotId) -> SlotKind {
        super::types::slot_kind_for_local(&self.slot_kinds, slot.0)
    }

    /// Widen a Cranelift value to an I64 bit pattern (ValueWord-shaped) for
    /// FFI boundaries that expect an `i64`-typed argument.
    ///
    /// F64 bitcasts in place (IEEE 754 bits = NaN-box encoding for numbers),
    /// I32/I16 sign-extend, I8 zero-extends (bool is 0/1). Already-I64 values
    /// pass through unchanged. Other Cranelift types are returned as-is.
    ///
    /// Use this when reading a raw native operand that must be handed to an
    /// FFI helper (e.g. `jit_array_push_elem`, `jit_typed_object_set_field`)
    /// whose signature is declared `(i64, …) -> i64` at the Cranelift layer.
    pub(crate) fn widen_to_i64(&mut self, val: Value) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::I64 {
            return val;
        }
        if val_type == types::F64 {
            return self.builder.ins().bitcast(types::I64, MemFlags::new(), val);
        }
        if val_type == types::I32 || val_type == types::I16 {
            return self.builder.ins().sextend(types::I64, val);
        }
        if val_type == types::I8 {
            return self.builder.ins().uextend(types::I64, val);
        }
        val
    }

    /// Coerce `val` so its Cranelift type matches `target_kind`'s declared
    /// Cranelift type.
    ///
    /// Used on `Place::Local` writes where the incoming `val` may have been
    /// produced at a different width (e.g. a generic-FFI I64 result stored
    /// into an F64 local, or a native F64 flowing into a dynamic I64 slot).
    ///
    /// The conversion table is a direct Cranelift widen/narrow/bitcast —
    /// no ValueWord tags or NaN-box bit patterns are involved. F64 ↔ I64
    /// goes through `bitcast` (raw IEEE 754 bits live in the I64 slot),
    /// integer widths use `sextend`/`uextend`/`ireduce`.
    pub(crate) fn ensure_kind(&mut self, val: Value, target_kind: SlotKind) -> Value {
        let target_cl_type = super::types::cranelift_type_for_slot(target_kind);
        let val_type = self.builder.func.dfg.value_type(val);

        if val_type == target_cl_type {
            return val;
        }

        // I64 → native (unbox-equivalent width reduction).
        if val_type == types::I64 {
            return match target_kind {
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
            };
        }

        // Native → I64 (box-equivalent width extension). F64 bitcasts; signed
        // integer widths sign-extend; bool/u8 zero-extend.
        if target_cl_type == types::I64 {
            return if val_type == types::F64 {
                self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
            } else if val_type == types::I32 || val_type == types::I16 {
                self.builder.ins().sextend(types::I64, val)
            } else if val_type == types::I8 {
                self.builder.ins().uextend(types::I64, val)
            } else {
                val
            };
        }

        // Native → native across differing widths: route through I64 as the
        // common intermediate. In practice this path is cold — MIR typing
        // keeps source and target aligned — but the general form keeps the
        // helper total.
        let widened = if val_type == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
        } else if val_type == types::I32 || val_type == types::I16 {
            self.builder.ins().sextend(types::I64, val)
        } else if val_type == types::I8 {
            self.builder.ins().uextend(types::I64, val)
        } else {
            val
        };

        match target_kind {
            SlotKind::Float64 => {
                self.builder
                    .ins()
                    .bitcast(types::F64, MemFlags::new(), widened)
            }
            SlotKind::Int32 | SlotKind::UInt32 => {
                self.builder.ins().ireduce(types::I32, widened)
            }
            SlotKind::Bool | SlotKind::Int8 | SlotKind::UInt8 => {
                self.builder.ins().ireduce(types::I8, widened)
            }
            SlotKind::Int16 | SlotKind::UInt16 => {
                self.builder.ins().ireduce(types::I16, widened)
            }
            _ => widened,
        }
    }
}
