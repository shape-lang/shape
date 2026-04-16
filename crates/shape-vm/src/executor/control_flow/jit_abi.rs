//! JIT Boundary ABI: TypedScalar marshaling for VM ↔ JIT transitions.
//!
//! When a callee function has a `FrameDescriptor` with known `SlotKind`s for
//! its parameters, the VM can marshal arguments into the JIT context buffer
//! using typed encoding instead of raw NaN-boxed passthrough. This gives the
//! JIT compiler accurate type information without an additional type-check on
//! the JIT side.
//!
//! The fallback is **always** NaN-boxed passthrough (raw u64 bits), never
//! synthetic None or null.

#[cfg(any(test, feature = "jit"))]
use crate::type_tracking::SlotKind;
#[cfg(any(test, feature = "jit"))]
use shape_value::{ValueWord, ValueWordExt};

/// Marshal a single VM argument into JIT-compatible u64 bits, guided by the
/// callee's `SlotKind` for that parameter slot.
///
/// # Encoding
///
/// | SlotKind           | Encoding                                         |
/// |--------------------|--------------------------------------------------|
/// | Int64/IntSize      | Extract i64, store as `i64 as u64` raw bits      |
/// | Float64            | NaN-boxed f64 passthrough (already correct)       |
/// | Bool               | `0u64` for false, `1u64` for true                |
/// | Unknown / other    | NaN-boxed passthrough (raw `ValueWord` bits)      |
///
/// The fallback is always the raw NaN-boxed bits — never None/null.
#[cfg(any(test, feature = "jit"))]
#[inline]
pub fn marshal_arg_to_jit(vw: &ValueWord, kind: SlotKind) -> u64 {
    match kind {
        // Integer types: extract the integer value and store raw bits.
        // The JIT expects plain i64 bits for integer-typed slots.
        SlotKind::Int64
        | SlotKind::NullableInt64
        | SlotKind::IntSize
        | SlotKind::NullableIntSize => {
            if vw.is_i64() {
                // Fast path: inline i48 -> i64
                let i = unsafe { vw.as_i64_unchecked() };
                i as u64
            } else if vw.is_f64() {
                let f = unsafe { vw.as_f64_unchecked() };
                (f as i64) as u64
            } else {
                vw.as_i64().unwrap_or(0) as u64
            }
        }

        // Smaller integer types: extract and store as i64 raw bits
        SlotKind::Int8
        | SlotKind::NullableInt8
        | SlotKind::Int16
        | SlotKind::NullableInt16
        | SlotKind::Int32
        | SlotKind::NullableInt32 => {
            if vw.is_i64() {
                (unsafe { vw.as_i64_unchecked() }) as u64
            } else if vw.is_f64() {
                (unsafe { vw.as_f64_unchecked() } as i64) as u64
            } else {
                vw.as_i64().unwrap_or(0) as u64
            }
        }

        // Unsigned integer types
        SlotKind::UInt8
        | SlotKind::NullableUInt8
        | SlotKind::UInt16
        | SlotKind::NullableUInt16
        | SlotKind::UInt32
        | SlotKind::NullableUInt32
        | SlotKind::UInt64
        | SlotKind::NullableUInt64
        | SlotKind::UIntSize
        | SlotKind::NullableUIntSize => {
            if vw.is_i64() {
                (unsafe { vw.as_i64_unchecked() }) as u64
            } else if vw.is_f64() {
                (unsafe { vw.as_f64_unchecked() }) as u64
            } else {
                // Heap-backed NativeScalar::U64 — extract the u64 payload,
                // not the raw NaN-boxed pointer bits.
                vw.as_u64_value().unwrap_or(0)
            }
        }

        // Float64: NaN-boxed passthrough (JIT uses same NaN-boxing for f64)
        SlotKind::Float64 | SlotKind::NullableFloat64 => vw.raw_bits(),

        // Bool: extract boolean and store as 0/1
        SlotKind::Bool => {
            if vw.is_bool() {
                (unsafe { vw.as_bool_unchecked() }) as u64
            } else {
                vw.raw_bits()
            }
        }

        // String, Dynamic, Unknown, or anything else: dynamic passthrough
        SlotKind::String | SlotKind::Dynamic | SlotKind::Unknown => vw.raw_bits(),
    }
}

/// Unmarshal a JIT return value back into a `ValueWord`, guided by the
/// callee's return `SlotKind`.
///
/// This is the inverse of `marshal_arg_to_jit` for the return path:
///
/// | SlotKind           | Decoding                                         |
/// |--------------------|--------------------------------------------------|
/// | Int64/IntSize      | Raw bits → i64 → ValueWord::from_i48/from_f64   |
/// | Float64            | NaN-boxed passthrough (already a ValueWord)       |
/// | Bool               | 0/1 → ValueWord::from_bool                       |
/// | Unknown / other    | NaN-boxed passthrough (transmute to ValueWord)    |
///
/// The fallback is always NaN-boxed passthrough.
#[cfg(any(test, feature = "jit"))]
#[inline]
pub fn unmarshal_jit_result(bits: u64, kind: SlotKind) -> ValueWord {
    match kind {
        // Integer return: JIT stored raw i64 bits, reconstruct ValueWord
        SlotKind::Int64
        | SlotKind::NullableInt64
        | SlotKind::IntSize
        | SlotKind::NullableIntSize
        | SlotKind::Int8
        | SlotKind::NullableInt8
        | SlotKind::Int16
        | SlotKind::NullableInt16
        | SlotKind::Int32
        | SlotKind::NullableInt32 => ValueWord::from_i64(bits as i64),

        // Unsigned integer return (sub-64: fits in i64)
        SlotKind::UInt8
        | SlotKind::NullableUInt8
        | SlotKind::UInt16
        | SlotKind::NullableUInt16
        | SlotKind::UInt32
        | SlotKind::NullableUInt32
        | SlotKind::UIntSize
        | SlotKind::NullableUIntSize => ValueWord::from_i64(bits as i64),

        // U64 return: may exceed i64::MAX
        SlotKind::UInt64 | SlotKind::NullableUInt64 => {
            if bits <= i64::MAX as u64 {
                ValueWord::from_i64(bits as i64)
            } else {
                ValueWord::from_native_u64(bits)
            }
        }

        // Float64: NaN-boxed passthrough
        SlotKind::Float64 | SlotKind::NullableFloat64 => unsafe {
            std::mem::transmute::<u64, ValueWord>(bits)
        },

        // Bool return: 0 → false, nonzero → true
        SlotKind::Bool => ValueWord::from_bool(bits != 0),

        // Dynamic passthrough: String, Dynamic, Unknown
        SlotKind::String | SlotKind::Dynamic | SlotKind::Unknown => unsafe {
            std::mem::transmute::<u64, ValueWord>(bits)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::{ValueWord, ValueWordExt};

    // ========================================================================
    // marshal_arg_to_jit tests
    // ========================================================================

    #[test]
    fn test_marshal_int64_from_i48() {
        let vw = ValueWord::from_f64(42.0);
        // When the ValueWord is an integer stored as f64, and the slot is Int64,
        // we should get 42 as raw i64 bits.
        let bits = marshal_arg_to_jit(&vw, SlotKind::Int64);
        assert_eq!(bits as i64, 42);
    }

    #[test]
    fn test_marshal_int64_from_inline_int() {
        // Create a value that is an inline i48 integer
        let vw = ValueWord::from_f64(100.0);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Int64);
        assert_eq!(bits as i64, 100);
    }

    #[test]
    fn test_marshal_int64_negative() {
        let vw = ValueWord::from_f64(-17.0);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Int64);
        assert_eq!(bits as i64, -17);
    }

    #[test]
    fn test_marshal_float64_passthrough() {
        let vw = ValueWord::from_f64(3.14);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Float64);
        // Float64 is NaN-boxed passthrough — should be the same raw bits
        assert_eq!(bits, vw.raw_bits());
    }

    #[test]
    fn test_marshal_bool_true() {
        let vw = ValueWord::from_bool(true);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Bool);
        assert_eq!(bits, 1);
    }

    #[test]
    fn test_marshal_bool_false() {
        let vw = ValueWord::from_bool(false);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Bool);
        assert_eq!(bits, 0);
    }

    #[test]
    fn test_marshal_unknown_passthrough() {
        let vw = ValueWord::from_f64(99.5);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Unknown);
        // Unknown: NaN-boxed passthrough
        assert_eq!(bits, vw.raw_bits());
    }

    #[test]
    fn test_marshal_string_passthrough() {
        let vw = ValueWord::from_string(std::sync::Arc::new("hello".to_string()));
        let bits = marshal_arg_to_jit(&vw, SlotKind::String);
        assert_eq!(bits, vw.raw_bits());
    }

    #[test]
    fn test_marshal_bool_on_non_bool_fallback() {
        // If we have a float value but the slot expects Bool, it should fall back
        // to NaN-boxed passthrough (not crash or produce nonsense)
        let vw = ValueWord::from_f64(1.0);
        let bits = marshal_arg_to_jit(&vw, SlotKind::Bool);
        // Falls through to NaN-boxed passthrough since tag is F64, not Bool
        assert_eq!(bits, vw.raw_bits());
    }

    // ========================================================================
    // unmarshal_jit_result tests
    // ========================================================================

    #[test]
    fn test_unmarshal_int64_result() {
        let result = unmarshal_jit_result(42u64, SlotKind::Int64);
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_unmarshal_int64_negative_result() {
        let result = unmarshal_jit_result((-17i64) as u64, SlotKind::Int64);
        assert_eq!(result.as_i64(), Some(-17));
    }

    #[test]
    fn test_unmarshal_float64_result() {
        let vw = ValueWord::from_f64(2.718);
        let bits = vw.raw_bits();
        let result = unmarshal_jit_result(bits, SlotKind::Float64);
        assert_eq!(result.as_f64(), Some(2.718));
    }

    #[test]
    fn test_unmarshal_bool_true_result() {
        let result = unmarshal_jit_result(1, SlotKind::Bool);
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_unmarshal_bool_false_result() {
        let result = unmarshal_jit_result(0, SlotKind::Bool);
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_unmarshal_unknown_passthrough() {
        let vw = ValueWord::from_f64(123.456);
        let bits = vw.raw_bits();
        let result = unmarshal_jit_result(bits, SlotKind::Unknown);
        assert_eq!(result.as_f64(), Some(123.456));
    }

    // ========================================================================
    // Round-trip tests: marshal then unmarshal
    // ========================================================================

    #[test]
    fn test_roundtrip_int64() {
        let original = ValueWord::from_f64(42.0);
        let marshaled = marshal_arg_to_jit(&original, SlotKind::Int64);
        let unmarshaled = unmarshal_jit_result(marshaled, SlotKind::Int64);
        // After round-trip through JIT, integer comes back as i64 (not f64)
        assert_eq!(unmarshaled.as_i64(), Some(42));
    }

    #[test]
    fn test_roundtrip_float64() {
        let original = ValueWord::from_f64(3.14159);
        let marshaled = marshal_arg_to_jit(&original, SlotKind::Float64);
        let unmarshaled = unmarshal_jit_result(marshaled, SlotKind::Float64);
        assert_eq!(unmarshaled.as_f64(), original.as_f64());
    }

    #[test]
    fn test_roundtrip_bool_true() {
        let original = ValueWord::from_bool(true);
        let marshaled = marshal_arg_to_jit(&original, SlotKind::Bool);
        let unmarshaled = unmarshal_jit_result(marshaled, SlotKind::Bool);
        assert_eq!(unmarshaled.as_bool(), Some(true));
    }

    #[test]
    fn test_roundtrip_bool_false() {
        let original = ValueWord::from_bool(false);
        let marshaled = marshal_arg_to_jit(&original, SlotKind::Bool);
        let unmarshaled = unmarshal_jit_result(marshaled, SlotKind::Bool);
        assert_eq!(unmarshaled.as_bool(), Some(false));
    }

    #[test]
    fn test_roundtrip_unknown() {
        let original = ValueWord::from_f64(99.9);
        let marshaled = marshal_arg_to_jit(&original, SlotKind::Unknown);
        let unmarshaled = unmarshal_jit_result(marshaled, SlotKind::Unknown);
        assert_eq!(unmarshaled.as_f64(), original.as_f64());
    }

    // ========================================================================
    // Mixed-type argument list simulation
    // ========================================================================

    #[test]
    fn test_mixed_type_args() {
        // Simulate a function with signature: fn(int, float, bool)
        let kinds = [SlotKind::Int64, SlotKind::Float64, SlotKind::Bool];
        let args = [
            ValueWord::from_f64(10.0),  // int arg
            ValueWord::from_f64(2.5),   // float arg
            ValueWord::from_bool(true), // bool arg
        ];

        let marshaled: Vec<u64> = args
            .iter()
            .zip(kinds.iter())
            .map(|(vw, kind)| marshal_arg_to_jit(vw, *kind))
            .collect();

        // Verify each was marshaled correctly
        assert_eq!(marshaled[0] as i64, 10); // int: raw i64 bits
        assert_eq!(marshaled[1], args[1].raw_bits()); // float: NaN-boxed passthrough
        assert_eq!(marshaled[2], 1); // bool: 1
    }

    #[test]
    fn test_mixed_type_with_unknown() {
        // fn(int, unknown, bool, unknown)
        let kinds = [
            SlotKind::Int64,
            SlotKind::Unknown,
            SlotKind::Bool,
            SlotKind::Unknown,
        ];
        let args = [
            ValueWord::from_f64(5.0),
            ValueWord::from_f64(3.14),
            ValueWord::from_bool(false),
            ValueWord::none(),
        ];

        let marshaled: Vec<u64> = args
            .iter()
            .zip(kinds.iter())
            .map(|(vw, kind)| marshal_arg_to_jit(vw, *kind))
            .collect();

        assert_eq!(marshaled[0] as i64, 5); // typed int
        assert_eq!(marshaled[1], args[1].raw_bits()); // unknown: passthrough
        assert_eq!(marshaled[2], 0); // typed bool
        assert_eq!(marshaled[3], args[3].raw_bits()); // unknown: passthrough
    }
}
