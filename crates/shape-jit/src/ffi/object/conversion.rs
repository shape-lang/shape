//! JIT-FFI bits ↔ runtime-tier carrier conversions (ADR-006 §2.7.5).
//!
//! Pre-strict-typing this module bridged the JIT's NaN-boxed `u64`
//! representation and the VM's `ValueWord` (the v1 dynamic tagged word).
//! The Phase-2 bulldozer deleted both the `ValueWord` type and the
//! `tag_bits::*` discriminator family.
//!
//! ## Carrier shape (§2.7.5 stamp-at-compile-time)
//!
//! Per ADR-006 §2.7.5 the JIT-FFI surface is `(u64, NativeKind)` — raw
//! bits plus a parallel kind companion. The kind is stamped at JIT
//! compile time from the call signature; it is **never decoded from the
//! bits**. Consumers that need a runtime-tier carrier wrap the pair as
//! `KindedSlot::new(ValueSlot::from_raw(bits), kind)` per §2.7.6/Q7 when
//! crossing into runtime-tier dispatch.
//!
//! ## What this module does (W11-jit-carrier-conversion close)
//!
//! These functions are the bookkeeping layer between the JIT call
//! signature's separate `bits: u64` and `kind: NativeKind` parameters
//! (the §2.7.5 stable-FFI raw-pair shape) and the in-Rust `JitFfiCarrier`
//! tuple alias `(u64, NativeKind)`. The body is intentionally trivial —
//! per §2.7.5 the conversion is *identity*: raw bits stay raw bits, and
//! the kind companion is whatever the caller's static stamp passed in.
//! No decode, no probe, no `is_heap()` classification — the kind is
//! authoritative because the caller proved it at JIT compile time.
//!
//! ## What is forbidden
//!
//! - **Decoding kind from `bits`** (CLAUDE.md "Forbidden Patterns" #4,
//!   ADR-006 §2.7.7 #4 — the deleted `tag_bits` dispatch).
//! - **`is_heap()` / `is_tagged()` probe** to classify (§2.7.7 #7).
//! - **Bool-default fallback** when the caller didn't supply a kind
//!   (forbidden #9 — surface-and-stop instead).
//! - **`ValueWord` resurrection under any name** — the body's
//!   pre-bulldozer shape decoded `Arc<HeapValue>` from raw bits via
//!   `ValueWord::clone_from_bits` and re-encoded per-arm via
//!   `ValueWord::as_heap_ref` / `tag_bits::TAG_HEAP`; all deleted.
//!
//! ## Historical context (pre-bulldozer body, what's gone)
//!
//! The deleted pipeline ran:
//! 1. `nanboxed_to_jit_bits(&ValueWord)` decoded `ValueWord` per arm
//!    (each arm a `tag_bits::TAG_*` discriminator), produced JIT bits.
//! 2. `jit_bits_to_nanboxed(bits)` re-decoded raw bits via the same
//!    `tag_bits` machinery, constructed a `ValueWord` via
//!    `ValueWord::from_*` constructors, returned it.
//! 3. The `UNIFIED_HEAP_REFS` thread-local accumulator captured retained
//!    `Arc<HeapValue>` shares for `drain_unified_heap_refs()` to release
//!    at the end of each JIT call.
//!
//! All three steps are gone. The retain/release path now flows through
//! `clone_with_kind` / `drop_with_kind` at the producing call site
//! (ADR-006 §2.7.7), driven by the parallel-kind track — no global
//! drain accumulator.

use crate::ffi::jit_kinds::JitFfiCarrier;
use shape_value::NativeKind;

/// Per-JIT-call ref-drain hook. Retained as a named export for the JIT
/// executor's per-call epilogue (`executor.rs`). The pre-bulldozer body
/// drained the deleted `UNIFIED_HEAP_REFS` thread-local that the deleted
/// `nanboxed_to_jit_bits` pushed `Arc<HeapValue>` retain-shares into.
///
/// Per ADR-006 §2.7.5 the post-call lifecycle is dispatched through
/// `clone_with_kind` / `drop_with_kind` at the producing call site (the
/// parallel-kind track in `crates/shape-vm/src/executor/vm_impl/stack.rs`),
/// not via a global accumulator. The hook is a no-op — there is nothing
/// to drain.
#[inline]
pub fn drain_unified_heap_refs() {
    // ADR-006 §2.7.5 / §2.7.7: post-call retain/release is dispatched
    // per-slot through the parallel-kind track at the producing call
    // site. No global accumulator exists; nothing to drain.
}

// ============================================================================
// JIT-FFI bits → runtime-tier carrier (`(u64, NativeKind)` pair)
// ============================================================================

/// Pack raw JIT-FFI bits + their static `NativeKind` stamp into a
/// `JitFfiCarrier` per ADR-006 §2.7.5.
///
/// The kind comes from the JIT call signature's stamp — the caller's
/// emitter proved it at JIT compile time. This function does **not**
/// decode the kind from `bits`: per §2.7.7 #4 / #7, runtime kind
/// discrimination from a raw bit pattern is the deleted ValueWord
/// `tag_bits` shape and is forbidden.
///
/// Consumers assemble a `KindedSlot` from the returned pair via
/// `KindedSlot::new(ValueSlot::from_raw(bits), kind)` per §2.7.6/Q7
/// when they need a runtime-tier carrier.
#[inline]
pub fn jit_bits_to_nanboxed(bits: u64, kind: NativeKind) -> JitFfiCarrier {
    // §2.7.5: identity pack. Bits stay raw; kind is the caller's stamp.
    (bits, kind)
}

/// Variant of `jit_bits_to_nanboxed` with `JITContext` access, retained
/// for downstream call sites (`async_ops`, `control`, `generic_builtin`,
/// `data_access`, etc.) that need function-name resolution from the
/// JIT context. The ctx pointer is plumbing for caller-side lookups
/// (function table, function names) — the carrier itself is the same
/// `(bits, kind)` pair.
#[inline]
pub fn jit_bits_to_nanboxed_with_ctx(
    bits: u64,
    kind: NativeKind,
    _ctx: *const super::super::super::context::JITContext,
) -> JitFfiCarrier {
    // §2.7.5: same identity pack as `jit_bits_to_nanboxed`. The `_ctx`
    // parameter is preserved for downstream signature compatibility but
    // unused here — function-name resolution happens at the caller
    // before passing `(bits, kind)` to this function.
    (bits, kind)
}

// ============================================================================
// TypedScalar ↔ JIT bits Conversion (still kind-flat — no carrier change)
// ============================================================================

/// Convert JIT NaN-boxed bits to a `TypedScalar` with an optional type hint.
///
/// `TypedScalar` is a kind-tagged scalar carrier (`ScalarKind` field) that
/// the producing site populates from the `NativeKind` hint per ADR-006
/// §2.7.5; this is the carrier shape for scalar-FFI returns and does not
/// participate in the deleted `ValueWord` dispatch. The hint comes from
/// the JIT-emitted `FrameDescriptor`'s slot-kind track per §2.7.7/Q9.
pub fn jit_bits_to_typed_scalar(
    bits: u64,
    hint: Option<shape_vm::NativeKind>,
) -> shape_value::TypedScalar {
    use crate::ffi::value_ffi::{
        TAG_BOOL_FALSE, TAG_BOOL_TRUE, TAG_NONE, TAG_NULL, TAG_UNIT, is_number, unbox_number,
    };
    use shape_value::TypedScalar;
    use shape_vm::NativeKind;

    if is_number(bits) {
        let f = unbox_number(bits);
        if let Some(h) = hint {
            match h {
                NativeKind::Int8 | NativeKind::NullableInt8 => return TypedScalar::i8(f as i8),
                NativeKind::UInt8 | NativeKind::NullableUInt8 => return TypedScalar::u8(f as u8),
                NativeKind::Int16 | NativeKind::NullableInt16 => return TypedScalar::i16(f as i16),
                NativeKind::UInt16 | NativeKind::NullableUInt16 => {
                    return TypedScalar::u16(f as u16);
                }
                NativeKind::Int32 | NativeKind::NullableInt32 => return TypedScalar::i32(f as i32),
                NativeKind::UInt32 | NativeKind::NullableUInt32 => {
                    return TypedScalar::u32(f as u32);
                }
                NativeKind::Int64 | NativeKind::NullableInt64 => return TypedScalar::i64(f as i64),
                NativeKind::UInt64 | NativeKind::NullableUInt64 => {
                    return TypedScalar::u64(f as u64);
                }
                NativeKind::Float64 | NativeKind::NullableFloat64 => {
                    return TypedScalar::f64_from_bits(bits);
                }
                _ => {
                    // Bool / String / Boxed / etc. fall through to the
                    // generic-number branch (the bits already encode an
                    // f64).
                }
            }
        }
        return TypedScalar::f64_from_bits(bits);
    }

    if bits == TAG_BOOL_TRUE {
        return TypedScalar::bool(true);
    }
    if bits == TAG_BOOL_FALSE {
        return TypedScalar::bool(false);
    }
    if bits == TAG_NULL || bits == TAG_NONE {
        return TypedScalar::none();
    }
    if bits == TAG_UNIT {
        return TypedScalar::unit();
    }

    // Non-scalar (heap pointer, function, etc.) — return None sentinel. The
    // kinded entry-point per ADR-006 §2.7.5/§2.7.10 takes the receiver's
    // NativeKind from the call signature for heap-shaped slots.
    TypedScalar::none()
}

/// Convert a `TypedScalar` to JIT NaN-boxed bits. Integer kinds box as
/// `box_number(value as f64)` since the JIT's Cranelift IR uses f64 for
/// all numeric operations internally — this is JIT-internal scalar
/// encoding, not `tag_bits` dispatch.
pub fn typed_scalar_to_jit_bits(ts: &shape_value::TypedScalar) -> u64 {
    use crate::ffi::value_ffi::{TAG_BOOL_FALSE, TAG_BOOL_TRUE, TAG_NULL, TAG_UNIT, box_number};
    use shape_value::ScalarKind;

    match ts.kind {
        ScalarKind::I8 | ScalarKind::I16 | ScalarKind::I32 | ScalarKind::I64 => {
            box_number(ts.payload_lo as i64 as f64)
        }
        ScalarKind::U8 | ScalarKind::U16 | ScalarKind::U32 | ScalarKind::U64 => {
            box_number(ts.payload_lo as f64)
        }
        ScalarKind::I128 | ScalarKind::U128 => box_number(ts.payload_lo as i64 as f64),
        ScalarKind::F64 | ScalarKind::F32 => ts.payload_lo, // already f64 bits
        ScalarKind::Bool => {
            if ts.payload_lo != 0 {
                TAG_BOOL_TRUE
            } else {
                TAG_BOOL_FALSE
            }
        }
        ScalarKind::None => TAG_NULL,
        ScalarKind::Unit => TAG_UNIT,
    }
}

// ============================================================================
// Runtime-tier carrier → JIT NaN-boxed bits
// ============================================================================

/// Unpack a `JitFfiCarrier` back to raw JIT-FFI bits per ADR-006 §2.7.5.
///
/// The carrier is `(bits, kind)` — the bits are already in the JIT's
/// raw 8-byte slot form per §2.7.5 stable-FFI rule. Unpacking is a
/// projection, not a re-encoding: the producing caller (the one that
/// constructed the carrier via `jit_bits_to_nanboxed`) already supplied
/// the canonical JIT-side bit pattern. The kind companion is consumed
/// by the caller's dispatch shell at the JIT-internal call site
/// (`op_get_prop` / `op_call_method` / etc. emitter), not re-decoded
/// here.
///
/// **Forbidden alternatives**:
/// - Per-arm re-encoding via `tag_bits::TAG_HEAP` / `make_tagged` on the
///   `NativeKind::Ptr(HeapKind::*)` arms — that is the deleted ValueWord
///   pipeline (CLAUDE.md "Forbidden Patterns" #4).
/// - `is_heap()` probe on `bits` to classify before re-encoding —
///   §2.7.7 #7 forbidden.
/// - Stamping kind from a side-table keyed by `bits` — same forbidden
///   shape; kind is on the carrier struct, not derived from the bits.
#[inline]
pub fn nanboxed_to_jit_bits(carrier: &JitFfiCarrier) -> u64 {
    // §2.7.5: identity unpack. The bits ARE the JIT-side representation;
    // no re-encoding step exists under strict typing.
    //
    // Kind on the carrier is consumed by the caller's dispatch shell
    // for stack-slot retain/release accounting at the JIT-internal
    // call site (the producing emitter knows the kind statically and
    // emits the matching `clone_with_kind` / `drop_with_kind` epilogue
    // in JIT IR). We do NOT touch refcounts here — that would alias-
    // share the slot's ownership in a way the dispatch shell would
    // either leak (if we cloned) or double-free (if we dropped).
    let _kind = carrier.1;
    carrier.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::{HeapKind, NativeKind};

    #[test]
    fn jit_bits_to_nanboxed_packs_bits_and_kind_identity() {
        // §2.7.5: pack is a trivial tuple constructor.
        let bits = 0xDEAD_BEEF_CAFE_BABEu64;
        let carrier = jit_bits_to_nanboxed(bits, NativeKind::Int64);
        assert_eq!(carrier.0, bits);
        assert_eq!(carrier.1, NativeKind::Int64);
    }

    #[test]
    fn jit_bits_to_nanboxed_preserves_heap_kind() {
        let bits = 0x1234_5678_9ABCu64; // arbitrary
        let carrier = jit_bits_to_nanboxed(bits, NativeKind::Ptr(HeapKind::TypedObject));
        assert_eq!(carrier.0, bits);
        assert_eq!(carrier.1, NativeKind::Ptr(HeapKind::TypedObject));
    }

    #[test]
    fn nanboxed_to_jit_bits_unpacks_identity() {
        // §2.7.5: unpack returns the bits unchanged; kind is consumed at
        // the caller's dispatch shell (not here).
        let bits = 0xABCD_1234_5678_DEFFu64;
        let carrier: JitFfiCarrier = (bits, NativeKind::Float64);
        assert_eq!(nanboxed_to_jit_bits(&carrier), bits);
    }

    #[test]
    fn roundtrip_bits_through_carrier() {
        // pack → unpack is identity on bits per §2.7.5.
        for kind in [
            NativeKind::Int64,
            NativeKind::Float64,
            NativeKind::Bool,
            NativeKind::String,
            NativeKind::Ptr(HeapKind::TypedObject),
            NativeKind::Ptr(HeapKind::TypedArray),
            NativeKind::Ptr(HeapKind::HashMap),
        ] {
            let bits = 0x1122_3344_5566_7788u64;
            let carrier = jit_bits_to_nanboxed(bits, kind);
            assert_eq!(carrier.1, kind);
            assert_eq!(nanboxed_to_jit_bits(&carrier), bits);
        }
    }

    #[test]
    fn drain_unified_heap_refs_is_noop() {
        // §2.7.5: no global accumulator to drain — refcounts are
        // dispatched per-slot through the parallel-kind track at the
        // producing call site.
        drain_unified_heap_refs();
        drain_unified_heap_refs();
    }
}
