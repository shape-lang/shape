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
use shape_vm::type_tracking::NativeKind;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Get the NativeKind for a given SlotId.
    ///
    /// Returns `Int64` when the inference pass left the slot
    /// undetermined — same Cranelift width as the legacy `_ => I64`
    /// catch-all in `cranelift_type_for_slot`. Codegen sites that
    /// specifically need a "kind was proven by inference" answer
    /// should call `slot_kind_for_local` directly and surface-and-stop
    /// on `None` per ADR-006 §2.7.7.
    pub(crate) fn slot_kind_of(&self, slot: shape_vm::mir::types::SlotId) -> NativeKind {
        super::types::slot_kind_for_local(&self.slot_kinds, slot.0)
            .unwrap_or(NativeKind::Int64)
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

    /// Widen a native Cranelift value into the I64 ABI slot used by JIT-FFI
    /// helpers. Per ADR-006 §2.7.5 the JIT-FFI carrier is `(u64, NativeKind)`
    /// — the slot's kind flows on the parallel companion stamped at JIT
    /// compile time, not packed into the bits.
    ///
    /// - `I64` (already in the ABI width): pass through unchanged.
    /// - `F64`: plain IEEE 754 `bitcast` — the consumer reads the raw bits
    ///   as `f64` because its parallel `NativeKind::Float64` says so.
    /// - `I8` with `hint == Some(Bool)`: zero-extend (bool is 0/1).
    /// - `I8`/`I16`/`I32` signed ints: sign-extend to I64 — the high bits
    ///   are the natural signed-int extension; consumers narrow with
    ///   `ireduce` per the kind companion.
    /// - Other types: fall back to plain `widen_to_i64` (raw bit-pattern).
    ///
    /// Replaces the deleted W-series `nan_box_for_value_word` NaN-box
    /// encoding (`tag_bits` payload mask + `TAG_INT` / `TAG_BOOL_*` tag
    /// dispatch). The `_hint` is retained on the signature for the bool
    /// zero-extend split; downstream callers should flow `NativeKind`
    /// through the JitFfiCarrier instead of relying on bit-level tags.
    pub(crate) fn nan_box_for_value_word(
        &mut self,
        val: Value,
        hint: Option<NativeKind>,
    ) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::I64 {
            return val;
        }
        if val_type == types::F64 {
            return self.builder.ins().bitcast(types::I64, MemFlags::new(), val);
        }
        // I8 with Bool kind companion: zero-extend to I64 (bool is 0/1).
        if val_type == types::I8 && matches!(hint, Some(NativeKind::Bool)) {
            return self.builder.ins().uextend(types::I64, val);
        }
        // Narrow signed int types (I8/I16/I32): sign-extend to the I64 ABI
        // width. The kind companion drives reinterpretation downstream.
        if val_type == types::I8 || val_type == types::I16 || val_type == types::I32 {
            return self.builder.ins().sextend(types::I64, val);
        }
        // Fallback: raw widen.
        self.widen_to_i64(val)
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
    pub(crate) fn ensure_kind(&mut self, val: Value, target_kind: NativeKind) -> Value {
        let target_cl_type = super::types::cranelift_type_for_slot(target_kind);
        let val_type = self.builder.func.dfg.value_type(val);

        if val_type == target_cl_type {
            return val;
        }

        // I64 → native (unbox-equivalent width reduction).
        if val_type == types::I64 {
            return match target_kind {
                NativeKind::Float64 => {
                    self.builder
                        .ins()
                        .bitcast(types::F64, MemFlags::new(), val)
                }
                NativeKind::Int32 | NativeKind::UInt32 => {
                    self.builder.ins().ireduce(types::I32, val)
                }
                NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => {
                    self.builder.ins().ireduce(types::I8, val)
                }
                NativeKind::Int16 | NativeKind::UInt16 => {
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
            NativeKind::Float64 => {
                self.builder
                    .ins()
                    .bitcast(types::F64, MemFlags::new(), widened)
            }
            NativeKind::Int32 | NativeKind::UInt32 => {
                self.builder.ins().ireduce(types::I32, widened)
            }
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => {
                self.builder.ins().ireduce(types::I8, widened)
            }
            NativeKind::Int16 | NativeKind::UInt16 => {
                self.builder.ins().ireduce(types::I16, widened)
            }
            _ => widened,
        }
    }

    /// Emit the lockstep kind-byte write into `JITContext.stack_kinds[slot_idx]`
    /// for the §2.7.7 / Q9 JIT-side parallel-kind track. Mirrors the data-side
    /// `store(stack[slot_idx], bits)` that every push site already emits.
    ///
    /// `slot_idx` is a Cranelift I64 slot index (the same value used to compute
    /// the data slot's byte offset). `kind` is the stamped `NativeKind` from
    /// the producing call signature (`operand_slot_kind` / a documented
    /// callee-classification kind for FFI sentinels). The byte is encoded via
    /// `stack_kind_code::encode` — codes 0..=23 are scalar arms, 128..=156
    /// are `Ptr(HeapKind)` arms (see
    /// `crates/shape-jit/src/ffi/stack_kind_code.rs`).
    ///
    /// **Forbidden alternatives** (refuse on sight):
    /// - Bool-default fallback for unknown kind (§2.7.7 #9) — callers must
    ///   surface-and-stop when the kind isn't known at the push site.
    /// - Skipping the kind write because "it's the same as the previous
    ///   slot" — the lockstep invariant requires a write at every push.
    /// - Decoding the kind from the data slot's bit pattern (§2.7.7 #4 / #7)
    ///   — kind comes from the call signature, not the bits.
    pub(crate) fn emit_kind_track_write(
        &mut self,
        slot_idx: Value,
        kind: shape_value::NativeKind,
    ) {
        let kinds_base = crate::context::STACK_KINDS_OFFSET as i64;
        // `stack_kinds: [u8; 512]` — slot index doubles as byte offset
        // within the kind track (no `<< 3` shift like the data side).
        let abs_off = self.builder.ins().iadd_imm(slot_idx, kinds_base);
        let addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
        let code = crate::ffi::stack_kind_code::encode(kind) as i64;
        let code_val = self.builder.ins().iconst(types::I8, code);
        self.builder.ins().store(MemFlags::new(), code_val, addr, 0);
    }
}
