//! Phase-2c surface module: JIT-FFI bits ↔ runtime carrier conversions.
//!
//! Pre-strict-typing this module bridged the JIT's NaN-boxed `u64`
//! representation and the VM's `ValueWord` (the v1 dynamic tagged word).
//! The Phase-2 bulldozer deleted both the `ValueWord` type and the
//! `tag_bits::*` discriminator family that this module's body decoded.
//! Per ADR-006 §2.7.5 the JIT-FFI surface is now `(u64, NativeKind)` —
//! raw bits plus a parallel kind companion stamped at JIT compile time
//! from the call signature; consumers wrap the pair as
//! `KindedSlot::new(ValueSlot::from_raw(bits), kind)` per §2.7.6/Q7 when
//! crossing into runtime-tier dispatch.
//!
//! This file's pre-bulldozer body — `jit_bits_to_nanboxed`,
//! `nanboxed_to_jit_bits`, `drain_unified_heap_refs`,
//! `jit_to_typed_array` / `typed_array_to_jit` helpers, the
//! `JitTaskGroup` heap-shape, and the per-`HeapValue::*` arm
//! re-encoding — are all gone with the deleted machinery they depended
//! on (`ValueWord::clone_from_bits`, `ValueWord::as_heap_ref`,
//! `ValueBits::is_unified_heap`, `ValueBits::unified_heap_ptr`,
//! `tag_bits::TAG_HEAP` / `TAG_INT` discriminators, `unified_array` /
//! `unified_matrix` / `unified_wrapper` / `unified_string` heap kinds,
//! `vmarray_from_vec` constructor, `value_word_drop::vw_clone` retain
//! helper, `ArgVec` carrier).
//!
//! Re-fill is gated on the kinded JIT-emission path (`op_call_value` /
//! `op_get_prop` / `op_length` / `op_make_closure` JIT lowerings)
//! threading the receiver's `NativeKind` companion through the call
//! signature per ADR-006 §2.7.5 / §2.7.10. Until then, the public
//! function symbols exist as `todo!("phase-2c: …")` surfaces so
//! downstream FFI consumers (`async_ops`, `control/mod.rs`,
//! `generic_builtin`, `data_access/mod.rs`, `executor.rs`,
//! `property_access::jit_hashmap_*`) report a single named blocker
//! when their own waves hit the kinded-translation site.
//!
//! See `docs/cluster-audits/wave-10-jit-playbook.md` §5
//! (surface-and-stop triggers).

use crate::ffi::jit_kinds::JitFfiCarrier;

/// Per-JIT-call ref-drain hook. Pre-strict-typing this drained the
/// thread-local `UNIFIED_HEAP_REFS` accumulator that `nanboxed_to_jit_bits`
/// pushed into when retaining `Arc<HeapValue>` shares for JIT consumption.
/// Per ADR-006 §2.7.5 the JIT-FFI carrier is `(u64, NativeKind)` and
/// retain/release is dispatched through `clone_with_kind` /
/// `drop_with_kind` at the producing call site, not via a global drain
/// hook — the post-call lifecycle moves into the kinded handler ABI per
/// §2.7.10/Q11. The named export is preserved so the JIT executor's
/// per-call epilogue compiles; the body is a Phase-2c surface.
pub fn drain_unified_heap_refs() {
    // Phase-2c §2.7.5 / §2.7.10/Q11: post-call ref-balance moves into the
    // kinded handler ABI dispatched at the producing call signature.
}

// ============================================================================
// JIT NaN-boxed bits → runtime-tier carrier
// ============================================================================

/// Convert raw JIT-FFI bits to a runtime-tier `JitFfiCarrier` per ADR-006
/// §2.7.5. Pre-strict-typing this returned `shape_value::ValueWord`; the
/// new shape is the `(u64, NativeKind)` pair the JIT call signature
/// stamps statically — runtime kind discrimination from the bits
/// themselves is forbidden (§2.7.7 #4 / #7).
///
/// # Phase-2c surface
///
/// The kind companion must flow through the JIT call signature; until
/// the relevant lowering site (`op_call_value` / `op_get_prop` / etc.)
/// threads `NativeKind` through, the body surfaces. See
/// `docs/cluster-audits/wave-10-jit-playbook.md` §5.
pub fn jit_bits_to_nanboxed(_bits: u64) -> JitFfiCarrier {
    todo!(
        "phase-2c §2.7.5: kinded JIT-FFI carrier conversion. \
         The JIT lowering must thread NativeKind through the call \
         signature; consumers assemble KindedSlot from the (u64, \
         NativeKind) pair per §2.7.6/Q7."
    )
}

/// Convert raw JIT-FFI bits to a runtime-tier `JitFfiCarrier` with
/// JITContext access for function-name lookup. Same Phase-2c surface as
/// `jit_bits_to_nanboxed`; the additional context pointer is preserved so
/// downstream call sites continue to type-check.
pub fn jit_bits_to_nanboxed_with_ctx(
    _bits: u64,
    _ctx: *const super::super::super::context::JITContext,
) -> JitFfiCarrier {
    todo!(
        "phase-2c §2.7.5: kinded JIT-FFI carrier conversion (with ctx). \
         The JIT lowering must thread NativeKind through the call \
         signature; function-name resolution flows through the kinded \
         carrier per §2.7.6/Q7."
    )
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

/// Convert a runtime-tier `JitFfiCarrier` (per ADR-006 §2.7.5) to raw
/// JIT-FFI bits. Pre-strict-typing this consumed `&shape_value::ValueWord`
/// and dispatched on `as_heap_ref()` / `tag_bits::TAG_HEAP` discriminators
/// — both deleted in the Phase-2 bulldozer. The new shape consumes the
/// canonical `(u64, NativeKind)` pair and dispatches on the explicit
/// `NativeKind` per §2.7.6/Q8 (no runtime kind discrimination from the
/// bits, no `is_heap()` probe).
///
/// # Phase-2c surface
///
/// Per-arm re-encoding (heap kinds → JitArray / unified_box, typed-array
/// arms → JitArray width-specific buffers) is gated on the `HeapKind`
/// carrier surfaces being available at this layer; deleted helpers
/// (`vmarray_from_vec`, `unified_array::*`, `unified_matrix::*`,
/// `unified_wrapper::*`, `unified_string::*`, `value_word_drop::vw_clone`)
/// were the encoding bridge and have no replacement until the kinded
/// JIT-FFI consumer waves (W11+) define the typed-Arc → JIT-bits
/// translations per arm.
pub fn nanboxed_to_jit_bits(_carrier: &JitFfiCarrier) -> u64 {
    todo!(
        "phase-2c §2.7.5: per-NativeKind re-encoding for the runtime-tier \
         carrier → JIT-bits direction. Each arm dispatches on the kind \
         companion per §2.7.6/Q8 (typed-Arc heap kinds, scalar kinds, \
         FunctionRef / ModuleFn / Ref labels). See \
         docs/cluster-audits/wave-10-jit-playbook.md §5."
    )
}
