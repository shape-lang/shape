//! Typed formatted-string value production for MIR JIT codegen.
//!
//! `jit_format_value` is the native implementation of
//! `Rvalue::FormatValue`. The call site stamps the source `NativeKind` and
//! only emits calls for the four canonical carriers supported here. This
//! function consumes the operand share and returns a fresh canonical
//! `Arc<String>` raw carrier; it never relabels the operand bits as String.

use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::{KindedSlot, NativeKind, ValueSlot};
use std::sync::{Arc, OnceLock};

/// Ordinary interpolation formatting (`f"{value}"`).
pub(crate) const FORMAT_DEFAULT: u8 = 0;
/// Fixed-point numeric formatting (`f"{value:fixed(N)}"`).
pub(crate) const FORMAT_FIXED: u8 = 1;

fn consume_default(bits: u64, kind: NativeKind) -> String {
    static EMPTY_REGISTRY: OnceLock<TypeSchemaRegistry> = OnceLock::new();
    let registry = EMPTY_REGISTRY.get_or_init(TypeSchemaRegistry::new);
    let formatter = shape_vm::executor::printing::ValueFormatter::new(registry);
    let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
    let rendered = formatter.format_kinded(&value);
    // `value` owns the moved operand share. Its kind-aware Drop consumes that
    // share after rendering; String therefore uses Arc::from_raw exactly once.
    rendered
}

fn consume_fixed(bits: u64, kind: NativeKind, precision: u8) -> String {
    match kind {
        // Match the VM's `FormatValueWithSpec` conversion exactly: integer
        // values first coerce to f64, then use precision-controlled rendering.
        NativeKind::Int64 => format!("{:.*}", precision as usize, bits as i64 as f64),
        NativeKind::Float64 => {
            format!("{:.*}", precision as usize, f64::from_bits(bits))
        }
        // The VM treats fixed formatting as a no-op for non-numeric values.
        NativeKind::Bool | NativeKind::String => consume_default(bits, kind),
        _ => unreachable!("JIT codegen emitted unsupported FormatValue source kind"),
    }
}

/// Convert one typed interpolation operand to a fresh `Arc<String>` carrier.
///
/// The MIR compiler only emits this call after proving the source kind is one
/// of `Int64`, `Bool`, `Float64`, or `String`, and the spec is Default or
/// Fixed. Table and ContentStyle are rejected during whole-program preflight;
/// unknown source kinds are rejected during codegen. The invariant arms below
/// are therefore unreachable from user-authored native code.
#[unsafe(no_mangle)]
pub extern "C" fn jit_format_value(bits: u64, kind_code: u8, spec_code: u8, precision: u8) -> u64 {
    let kind = match super::stack_kind_code::decode(kind_code) {
        Some(
            kind
            @ (NativeKind::Int64 | NativeKind::Bool | NativeKind::Float64 | NativeKind::String),
        ) => kind,
        _ => unreachable!("JIT codegen emitted unsupported FormatValue kind code"),
    };

    let rendered = match spec_code {
        FORMAT_DEFAULT => consume_default(bits, kind),
        FORMAT_FIXED => consume_fixed(bits, kind, precision),
        _ => unreachable!("JIT codegen emitted unsupported FormatValue spec code"),
    };

    Arc::into_raw(Arc::new(rendered)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take_rendered(bits: u64) -> String {
        let value = unsafe { Arc::<String>::from_raw(bits as *const String) };
        (*value).clone()
    }

    #[test]
    fn default_scalar_formatting_returns_canonical_string_carriers() {
        let cases = [
            (42_u64, NativeKind::Int64, "42"),
            (1_u64, NativeKind::Bool, "true"),
            (1.5_f64.to_bits(), NativeKind::Float64, "1.5"),
            (1.0_f64.to_bits(), NativeKind::Float64, "1.0"),
            (f64::NAN.to_bits(), NativeKind::Float64, "NaN"),
            (f64::INFINITY.to_bits(), NativeKind::Float64, "Infinity"),
            (
                f64::NEG_INFINITY.to_bits(),
                NativeKind::Float64,
                "-Infinity",
            ),
        ];

        for (bits, kind, expected) in cases {
            let rendered = jit_format_value(
                bits,
                crate::ffi::stack_kind_code::encode(kind),
                FORMAT_DEFAULT,
                0,
            );
            assert_eq!(take_rendered(rendered), expected);
        }
    }

    #[test]
    fn formatting_consumes_string_input_and_returns_a_fresh_string_carrier() {
        let input = Arc::into_raw(Arc::new("hello".to_string())) as u64;
        let rendered = jit_format_value(
            input,
            crate::ffi::stack_kind_code::encode(NativeKind::String),
            FORMAT_DEFAULT,
            0,
        );
        assert_eq!(take_rendered(rendered), "hello");
    }

    #[test]
    fn fixed_formatting_matches_vm_numeric_coercion() {
        let rendered = jit_format_value(
            7,
            crate::ffi::stack_kind_code::encode(NativeKind::Int64),
            FORMAT_FIXED,
            2,
        );
        assert_eq!(take_rendered(rendered), "7.00");

        let rendered = jit_format_value(
            1.5_f64.to_bits(),
            crate::ffi::stack_kind_code::encode(NativeKind::Float64),
            FORMAT_FIXED,
            3,
        );
        assert_eq!(take_rendered(rendered), "1.500");
    }
}
