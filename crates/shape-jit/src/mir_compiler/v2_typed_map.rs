//! Inline typed `HashMap<string, ...>` codegen for the v2 runtime.
//!
//! Emits direct FFI calls to `jit_v2_map_get_str_i64` / `jit_v2_map_get_str_f64`
//! / `jit_v2_map_has_str` / `jit_v2_map_set_str_i64` / `jit_v2_map_len` when
//! the compiler has proven the receiver is a `HashMap<string, T>`. This
//! bypasses the generic `jit_call_method` trampoline (which otherwise takes
//! the "VM-format HashMap" path through `dispatch_method_via_trampoline`).
//!
//! ## Dispatch contract
//!
//! - `get(key)` / `set(key, val)` / `has(key)` — key must be a string. The
//!   FFI helpers treat non-string keys as "miss" (return `none` / `false`).
//! - The receiver's concrete type must be `HashMap<String, V>` where
//!   V is one of: `I64` (int), `F64` (number).
//! - `length` / `len` / `size` — no arg, returns `i64`.
//!
//! ## Why not inline the body?
//!
//! HashMap lookups involve a hash computation, bucket probing, and string
//! comparison. Inlining that in Cranelift IR would trade code-size for
//! ~zero win over a direct FFI call (one C call, no stack setup beyond
//! register-passed args). The FFI helpers also handle null / wrong-type
//! receivers safely, which keeps the JIT codegen straight-line.

use cranelift::prelude::*;
use shape_value::v2::ConcreteType;
use shape_vm::mir::types::{Operand, Place};
use shape_vm::type_tracking::SlotKind;

use super::MirToIR;

/// The key/value element kinds of a typed `HashMap<K, V>` receiver, resolved
/// via the per-slot `ConcreteType` side-table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TypedMapKinds {
    /// The concrete value type stored in the map (e.g. `I64`, `F64`).
    pub value: SlotKind,
}

impl<'a, 'b> MirToIR<'a, 'b> {
    /// If the place's root local is a `HashMap<String, V>` whose value type
    /// is a scalar primitive, return the corresponding kinds. Returns `None`
    /// for non-map slots, non-string-keyed maps, or unresolved types — caller
    /// falls back to the legacy trampoline path.
    pub(crate) fn v2_typed_str_map_kinds(&self, place: &Place) -> Option<TypedMapKinds> {
        let slot = match place {
            Place::Local(s) => *s,
            _ => return None,
        };
        let ct = self.concrete_types.get(slot.0 as usize)?;
        let (k, v) = match ct {
            ConcreteType::HashMap(k, v) => (k.as_ref(), v.as_ref()),
            _ => return None,
        };
        // Only string-keyed maps have dedicated FFI helpers today.
        if !matches!(k, ConcreteType::String) {
            return None;
        }
        let value_kind = match v {
            ConcreteType::I64 => SlotKind::Int64,
            ConcreteType::F64 => SlotKind::Float64,
            _ => return None,
        };
        Some(TypedMapKinds { value: value_kind })
    }

    /// Try to emit an inline v2 typed-HashMap method call. Returns `Some(())`
    /// when the method was handled; `None` means the caller should fall back
    /// to the generic method-dispatch trampoline.
    pub(crate) fn try_emit_v2_typed_map_method(
        &mut self,
        method_name: &str,
        receiver: &Place,
        rest_args: &[Operand],
        destination: &Place,
        kinds: TypedMapKinds,
    ) -> Result<Option<()>, String> {
        match method_name {
            // ── length / len / size ─────────────────────────────────────
            "length" | "len" | "size" => {
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                let map_bits = self.read_place(receiver)?;
                let map_bits = self.ensure_nanboxed(map_bits);
                let inst = self.builder.ins().call(self.ffi.v2_map_len, &[map_bits]);
                let len_i64 = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, len_i64)?;
                Ok(Some(()))
            }

            // ── has ────────────────────────────────────────────────────
            "has" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                let map_bits = self.read_place(receiver)?;
                let map_bits = self.ensure_nanboxed(map_bits);
                let key_bits = self.compile_operand_raw(&rest_args[0])?;
                let key_bits = self.ensure_nanboxed(key_bits);
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.v2_map_has_str, &[map_bits, key_bits]);
                let result_i64 = self.builder.inst_results(inst)[0];
                // destination is typically a Bool-kinded place; write the i64
                // and let the slot-kind plumbing narrow it as needed.
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result_i64)?;
                Ok(Some(()))
            }

            // ── get ────────────────────────────────────────────────────
            "get" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                let map_bits = self.read_place(receiver)?;
                let map_bits = self.ensure_nanboxed(map_bits);
                let key_bits = self.compile_operand_raw(&rest_args[0])?;
                let key_bits = self.ensure_nanboxed(key_bits);
                let result = match kinds.value {
                    SlotKind::Int64 | SlotKind::UInt64 => {
                        let inst = self.builder.ins().call(
                            self.ffi.v2_map_get_str_i64,
                            &[map_bits, key_bits],
                        );
                        self.builder.inst_results(inst)[0]
                    }
                    SlotKind::Float64 => {
                        let inst = self.builder.ins().call(
                            self.ffi.v2_map_get_str_f64,
                            &[map_bits, key_bits],
                        );
                        self.builder.inst_results(inst)[0]
                    }
                    _ => return Ok(None),
                };
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result)?;
                Ok(Some(()))
            }

            // ── set ────────────────────────────────────────────────────
            // set(key, value) — only the int-valued variant has a dedicated
            // helper today; other value types fall back to the generic
            // trampoline.
            "set" => {
                if rest_args.len() != 2 {
                    return Ok(None);
                }
                if !matches!(kinds.value, SlotKind::Int64 | SlotKind::UInt64) {
                    return Ok(None);
                }
                let map_bits = self.read_place(receiver)?;
                let map_bits = self.ensure_nanboxed(map_bits);
                let key_bits = self.compile_operand_raw(&rest_args[0])?;
                let key_bits = self.ensure_nanboxed(key_bits);
                let val_bits = self.compile_operand_raw(&rest_args[1])?;
                let val_bits = self.ensure_nanboxed(val_bits);
                let inst = self.builder.ins().call(
                    self.ffi.v2_map_set_str_i64,
                    &[map_bits, key_bits, val_bits],
                );
                let new_map_bits = self.builder.inst_results(inst)[0];
                // Write the (possibly CoW-cloned) map handle back to the
                // receiver slot so subsequent reads see the update.
                if let Place::Local(_) = receiver {
                    self.write_place(receiver, new_map_bits)?;
                }
                // The destination of a `.set()` call is conventionally unit
                // / none — write a zero sentinel so the caller's slot gets
                // a defined value.
                let none_val = self.builder.ins().iconst(types::I64, 0i64);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, none_val)?;
                Ok(Some(()))
            }

            _ => Ok(None),
        }
    }
}
