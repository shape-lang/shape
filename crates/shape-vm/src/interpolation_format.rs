//! Canonical pure formatting for primitive interpolation values.
//!
//! The bytecode VM and native JIT both call this module. Carrier adoption,
//! display-method dispatch, and allocation remain backend concerns; the text
//! policy for primitive values has one authority here.

use shape_value::{KindedSlot, NativeKind};

/// Borrowed, carrier-independent primitive value accepted by interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InterpolationValue<'a> {
    Int(i64),
    UInt(u64),
    Number(f64),
    Bool(bool),
    String(&'a str),
}

/// Formatting policy supported by the primitive interpolation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpolationFormatPolicy {
    Default,
    Fixed { precision: usize },
}

/// Format a primitive interpolation value under the requested policy.
pub fn format_interpolation(
    value: InterpolationValue<'_>,
    policy: InterpolationFormatPolicy,
) -> String {
    match policy {
        InterpolationFormatPolicy::Default => format_default(value),
        InterpolationFormatPolicy::Fixed { precision } => match value {
            InterpolationValue::Int(value) => format!("{:.*}", precision, value as f64),
            InterpolationValue::UInt(value) => format!("{:.*}", precision, value as f64),
            InterpolationValue::Number(value) => format!("{:.*}", precision, value),
            InterpolationValue::Bool(_) | InterpolationValue::String(_) => format_default(value),
        },
    }
}

fn format_default(value: InterpolationValue<'_>) -> String {
    match value {
        InterpolationValue::Int(value) => value.to_string(),
        InterpolationValue::UInt(value) => value.to_string(),
        InterpolationValue::Number(value) if value.is_nan() => "NaN".to_string(),
        InterpolationValue::Number(value) if value.is_infinite() => {
            if value.is_sign_positive() {
                "Infinity".to_string()
            } else {
                "-Infinity".to_string()
            }
        }
        InterpolationValue::Number(value) if value.fract() == 0.0 && value.abs() < 1e15 => {
            format!("{}.0", value as i64)
        }
        InterpolationValue::Number(value) => value.to_string(),
        InterpolationValue::Bool(value) => value.to_string(),
        InterpolationValue::String(value) => value.to_string(),
    }
}

/// Borrow a VM slot as a primitive interpolation value when its carrier kind
/// has a canonical representation in this module.
pub(crate) fn primitive_interpolation_value(slot: &KindedSlot) -> Option<InterpolationValue<'_>> {
    match slot.kind {
        NativeKind::Int8
        | NativeKind::NullableInt8
        | NativeKind::Int16
        | NativeKind::NullableInt16
        | NativeKind::Int32
        | NativeKind::NullableInt32
        | NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize => Some(InterpolationValue::Int(slot.slot.as_i64())),
        NativeKind::UInt8
        | NativeKind::NullableUInt8
        | NativeKind::UInt16
        | NativeKind::NullableUInt16
        | NativeKind::UInt32
        | NativeKind::NullableUInt32
        | NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => Some(InterpolationValue::UInt(slot.slot.as_u64())),
        NativeKind::Float64 | NativeKind::NullableFloat64 => {
            Some(InterpolationValue::Number(slot.slot.as_f64()))
        }
        NativeKind::Float32 => Some(InterpolationValue::Number(f64::from(f32::from_bits(
            slot.slot.raw() as u32,
        )))),
        NativeKind::Bool => Some(InterpolationValue::Bool(slot.slot.as_bool())),
        NativeKind::String | NativeKind::StringV2 => slot.as_str().map(InterpolationValue::String),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_preserves_shape_primitive_text() {
        let cases = [
            (InterpolationValue::Int(-7), "-7"),
            (InterpolationValue::UInt(7), "7"),
            (InterpolationValue::Number(1.5), "1.5"),
            (InterpolationValue::Number(1.0), "1.0"),
            (InterpolationValue::Number(f64::NAN), "NaN"),
            (InterpolationValue::Number(f64::INFINITY), "Infinity"),
            (InterpolationValue::Number(f64::NEG_INFINITY), "-Infinity"),
            (InterpolationValue::Bool(true), "true"),
            (InterpolationValue::String("shape"), "shape"),
        ];
        for (value, expected) in cases {
            assert_eq!(
                format_interpolation(value, InterpolationFormatPolicy::Default),
                expected
            );
        }
    }

    #[test]
    fn fixed_policy_matches_vm_numeric_coercion_and_non_numeric_noop() {
        let fixed = InterpolationFormatPolicy::Fixed { precision: 2 };
        assert_eq!(
            format_interpolation(InterpolationValue::Int(7), fixed),
            "7.00"
        );
        assert_eq!(
            format_interpolation(InterpolationValue::UInt(7), fixed),
            "7.00"
        );
        assert_eq!(
            format_interpolation(InterpolationValue::Number(1.5), fixed),
            "1.50"
        );
        assert_eq!(
            format_interpolation(InterpolationValue::Bool(true), fixed),
            "true"
        );
        assert_eq!(
            format_interpolation(InterpolationValue::String("shape"), fixed),
            "shape"
        );
    }
}
