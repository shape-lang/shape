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
use shape_vm::type_tracking::NativeKind;

use super::MirToIR;

/// The key/value element kinds of a typed `HashMap<K, V>` receiver, resolved
/// via the per-slot `ConcreteType` side-table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TypedMapKinds {
    /// The concrete value type stored in the map (e.g. `I64`, `F64`).
    pub value: NativeKind,
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
            ConcreteType::I64 => NativeKind::Int64,
            ConcreteType::F64 => NativeKind::Float64,
            _ => return None,
        };
        Some(TypedMapKinds { value: value_kind })
    }

    /// Try to emit an inline v2 typed-HashMap method call. Returns `Some(())`
    /// when the method was handled; `None` means the caller should fall back
    /// to the generic method-dispatch trampoline.
    ///
    /// ## Route A surface-and-stop (ADR-006 §2.7.14 Q15 / W11-jit-carrier-conversion)
    ///
    /// The kind-blind `jit_v2_map_*` FFI symbols (deleted ValueWord-shape map
    /// FFI: `jit_v2_map_get_str_i64` / `get_str_f64` / `has_str` /
    /// `set_str_i64` / `len`) are gated on the kinded `Arc<HashMapData>` +
    /// `KindedSlot` rebuild. The deleted bodies treated the map handle and
    /// key as `u64` bit-patterns whose kind was recovered by tag-bit decode
    /// — the §2.7.7 #4 / #7 forbidden pattern. Route A's resolution is the
    /// W11-jit-carrier-conversion sub-cluster.
    ///
    /// Until that lands, every typed-map method call surfaces-and-stops at
    /// JIT compile time. The receiver-side trampoline path in the VM is
    /// the runtime fallback when the JIT bails.
    pub(crate) fn try_emit_v2_typed_map_method(
        &mut self,
        method_name: &str,
        _receiver: &Place,
        _rest_args: &[Operand],
        _destination: &Place,
        _kinds: TypedMapKinds,
    ) -> Result<Option<()>, String> {
        Err(format!(
            "Route A surface-and-stop: SURFACE — typed-HashMap method `{}` \
             depends on the kinded `Arc<HashMapData>` + `KindedSlot` map FFI \
             rebuild. The kind-blind `jit_v2_map_*` symbols (deleted \
             ValueWord-shape ABI) are gated on W11-jit-carrier-conversion \
             per ADR-006 §2.7.14 Q15. ADR-006 §2.7.14 / §2.7.5.",
            method_name
        ))
    }
}

// Silence unused-import warnings — these are still re-exported by the
// module but the surface-and-stop body has dropped its uses.
#[allow(dead_code)]
const _: fn() = || {
    let _ = NativeKind::Int64;
    let _ = types::I64;
};
