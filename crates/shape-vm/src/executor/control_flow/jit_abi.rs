//! JIT Boundary ABI: raw-u64 + parallel `NativeKind` marshaling for VM ↔ JIT
//! transitions.
//!
//! Post-strict-typing (ADR-006 §2.7.7), VM stack slots and JIT context-buffer
//! slots share an 8-byte raw-u64 representation: typed integers, floats, bools,
//! and heap-pointer bits all live as raw native u64 with no NaN-boxing. Per
//! ADR-006 §2.7.5 the JIT FFI boundary is a cross-crate ABI surface that stays
//! on raw bits + parallel `NativeKind`; conversion to/from runtime carriers
//! (`KindedSlot`) happens on the runtime side, not at this boundary.
//!
//! Concretely this means `marshal_arg_to_jit` / `unmarshal_jit_result` are
//! identity-by-bits in every concrete `NativeKind` arm. Per ADR-006
//! §2.7.5.1 / §2.7.7 #5 the kind reaching this site is always concrete
//! (sourced from `FrameDescriptor.slots` / `OsrEntryPoint.local_kinds`); the
//! `NativeKind::Dynamic` / `NativeKind::Unknown` placeholder variants were
//! deleted alongside the strict-typing bulldozer (see
//! `crates/shape-value/src/native_kind.rs` §2.7.7 #6 for the deletion note).

#[cfg(any(test, feature = "jit"))]
use crate::type_tracking::NativeKind;

/// Marshal a single VM argument's raw u64 slot bits into JIT-compatible u64
/// bits, guided by the callee's `NativeKind` for that parameter slot.
///
/// Per ADR-006 §2.7.7 the VM stack stores raw native bits per slot (typed
/// integers as `i64 as u64` / `u64`, floats as `f64::to_bits()`, bools as
/// `0`/`1`, heap pointers as raw `Arc::into_raw` bits). Per ADR-006 §2.7.5
/// the JIT context buffer uses the same raw-u64 + parallel `NativeKind`
/// shape across the cross-crate boundary, so this function is identity by
/// bits in every `NativeKind` arm.
///
/// The `kind` parameter is retained for symmetry with the unmarshal direction
/// and to permit future `debug_assert!`-style kind-vs-payload sanity checks.
#[cfg(any(test, feature = "jit"))]
#[inline]
pub fn marshal_arg_to_jit(bits: u64, kind: NativeKind) -> u64 {
    match kind {
        NativeKind::Float64
        | NativeKind::NullableFloat64
        | NativeKind::Int8
        | NativeKind::NullableInt8
        | NativeKind::UInt8
        | NativeKind::NullableUInt8
        | NativeKind::Int16
        | NativeKind::NullableInt16
        | NativeKind::UInt16
        | NativeKind::NullableUInt16
        | NativeKind::Int32
        | NativeKind::NullableInt32
        | NativeKind::UInt32
        | NativeKind::NullableUInt32
        | NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize
        | NativeKind::Bool
        | NativeKind::String
        | NativeKind::Ptr(_) => bits,
    }
}

/// Unmarshal a JIT return value back into raw u64 slot bits, guided by the
/// callee's return `NativeKind`.
///
/// Inverse of `marshal_arg_to_jit`: identity by bits in every concrete
/// `NativeKind` arm, since VM and JIT share the raw-u64 slot ABI per ADR-006
/// §2.7.5 / §2.7.7.
#[cfg(any(test, feature = "jit"))]
#[inline]
pub fn unmarshal_jit_result(bits: u64, kind: NativeKind) -> u64 {
    match kind {
        NativeKind::Float64
        | NativeKind::NullableFloat64
        | NativeKind::Int8
        | NativeKind::NullableInt8
        | NativeKind::UInt8
        | NativeKind::NullableUInt8
        | NativeKind::Int16
        | NativeKind::NullableInt16
        | NativeKind::UInt16
        | NativeKind::NullableUInt16
        | NativeKind::Int32
        | NativeKind::NullableInt32
        | NativeKind::UInt32
        | NativeKind::NullableUInt32
        | NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize
        | NativeKind::Bool
        | NativeKind::String
        | NativeKind::Ptr(_) => bits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // marshal_arg_to_jit tests — raw-u64 identity across concrete NativeKind
    // ========================================================================

    #[test]
    fn test_marshal_int64_identity() {
        let bits = 42u64;
        assert_eq!(marshal_arg_to_jit(bits, NativeKind::Int64), bits);
    }

    #[test]
    fn test_marshal_int64_negative_identity() {
        let bits = (-17i64) as u64;
        assert_eq!(marshal_arg_to_jit(bits, NativeKind::Int64), bits);
    }

    #[test]
    fn test_marshal_uint64_high_bits() {
        // u64 above i64::MAX must round-trip without sign-extension corruption
        let bits = (i64::MAX as u64) + 1;
        assert_eq!(marshal_arg_to_jit(bits, NativeKind::UInt64), bits);
    }

    #[test]
    fn test_marshal_float64_identity() {
        let bits = 3.14f64.to_bits();
        assert_eq!(marshal_arg_to_jit(bits, NativeKind::Float64), bits);
    }

    #[test]
    fn test_marshal_bool_true() {
        assert_eq!(marshal_arg_to_jit(1, NativeKind::Bool), 1);
    }

    #[test]
    fn test_marshal_bool_false() {
        assert_eq!(marshal_arg_to_jit(0, NativeKind::Bool), 0);
    }

    #[test]
    fn test_marshal_string_identity() {
        // String slot bits are the raw `Arc::into_raw` pointer bits — opaque,
        // identity round-trip.
        let bits = 0xDEAD_BEEF_CAFE_F00Du64;
        assert_eq!(marshal_arg_to_jit(bits, NativeKind::String), bits);
    }

    // ========================================================================
    // unmarshal_jit_result tests
    // ========================================================================

    #[test]
    fn test_unmarshal_int64_result() {
        assert_eq!(unmarshal_jit_result(42u64, NativeKind::Int64), 42u64);
    }

    #[test]
    fn test_unmarshal_int64_negative_result() {
        let bits = (-17i64) as u64;
        assert_eq!(unmarshal_jit_result(bits, NativeKind::Int64), bits);
    }

    #[test]
    fn test_unmarshal_float64_result() {
        let bits = 2.718f64.to_bits();
        assert_eq!(unmarshal_jit_result(bits, NativeKind::Float64), bits);
    }

    #[test]
    fn test_unmarshal_bool_true_result() {
        assert_eq!(unmarshal_jit_result(1, NativeKind::Bool), 1);
    }

    #[test]
    fn test_unmarshal_bool_false_result() {
        assert_eq!(unmarshal_jit_result(0, NativeKind::Bool), 0);
    }

    // ========================================================================
    // Round-trip tests
    // ========================================================================

    #[test]
    fn test_roundtrip_int64() {
        let bits = 42u64;
        let marshaled = marshal_arg_to_jit(bits, NativeKind::Int64);
        let unmarshaled = unmarshal_jit_result(marshaled, NativeKind::Int64);
        assert_eq!(unmarshaled, bits);
    }

    #[test]
    fn test_roundtrip_float64() {
        let bits = 3.14159f64.to_bits();
        let marshaled = marshal_arg_to_jit(bits, NativeKind::Float64);
        let unmarshaled = unmarshal_jit_result(marshaled, NativeKind::Float64);
        assert_eq!(unmarshaled, bits);
    }

    #[test]
    fn test_roundtrip_bool_true() {
        let marshaled = marshal_arg_to_jit(1, NativeKind::Bool);
        let unmarshaled = unmarshal_jit_result(marshaled, NativeKind::Bool);
        assert_eq!(unmarshaled, 1);
    }

    #[test]
    fn test_roundtrip_bool_false() {
        let marshaled = marshal_arg_to_jit(0, NativeKind::Bool);
        let unmarshaled = unmarshal_jit_result(marshaled, NativeKind::Bool);
        assert_eq!(unmarshaled, 0);
    }

    // ========================================================================
    // Mixed-type argument list simulation
    // ========================================================================

    #[test]
    fn test_mixed_type_args() {
        // Simulate a function with signature: fn(int, float, bool)
        let kinds = [NativeKind::Int64, NativeKind::Float64, NativeKind::Bool];
        let arg_bits = [
            10u64,             // int arg
            2.5f64.to_bits(),  // float arg
            1u64,              // bool arg (true)
        ];

        let marshaled: Vec<u64> = arg_bits
            .iter()
            .zip(kinds.iter())
            .map(|(bits, kind)| marshal_arg_to_jit(*bits, *kind))
            .collect();

        assert_eq!(marshaled[0], 10);
        assert_eq!(marshaled[1], 2.5f64.to_bits());
        assert_eq!(marshaled[2], 1);
    }
}
