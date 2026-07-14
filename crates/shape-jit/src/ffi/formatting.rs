//! Typed formatted-string carrier production for MIR JIT codegen.
//!
//! Source kind and policy are selected by the compiler through distinct FFI
//! imports. These functions contain no metadata decode and no formatting
//! policy: they call `shape_vm::interpolation_format`, then allocate the
//! canonical `Arc<String>` raw carrier returned to native code.

use shape_vm::interpolation_format::{
    InterpolationFormatPolicy, InterpolationValue, format_interpolation,
};
use std::sync::Arc;

/// Convert rendered text into one transferable raw `Arc<String>` share.
///
/// # Safety
///
/// The caller must install the returned share in exactly one String owner or
/// adopt and release it with `Arc::<String>::from_raw` exactly once.
unsafe fn return_carrier(rendered: String) -> u64 {
    Arc::into_raw(Arc::new(rendered)) as u64
}

/// Format a proven `int` value with the default interpolation policy.
///
/// # Safety
///
/// The returned bits own one `Arc<String>` strong share. The caller must
/// install them in exactly one `NativeKind::String` owner or adopt and release
/// that share with `Arc::<String>::from_raw` exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_default_i64(value: i64) -> u64 {
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe {
        return_carrier(format_interpolation(
            InterpolationValue::Int(value),
            InterpolationFormatPolicy::Default,
        ))
    }
}

/// Format a proven `bool` value with the default interpolation policy.
///
/// # Safety
///
/// `value` must be the canonical native bool byte (`0` or `1`). The returned
/// bits carry one transferable `Arc<String>` strong share as documented by
/// [`jit_format_default_i64`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_default_bool(value: u8) -> u64 {
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe {
        return_carrier(format_interpolation(
            InterpolationValue::Bool(value != 0),
            InterpolationFormatPolicy::Default,
        ))
    }
}

/// Format a proven `number` value with the default interpolation policy.
///
/// # Safety
///
/// The returned bits carry one transferable `Arc<String>` strong share as
/// documented by [`jit_format_default_i64`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_default_f64(value: f64) -> u64 {
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe {
        return_carrier(format_interpolation(
            InterpolationValue::Number(value),
            InterpolationFormatPolicy::Default,
        ))
    }
}

/// Consume one proven `Arc<String>` raw share and format it with the default
/// interpolation policy.
///
/// # Safety
///
/// `bits` must be non-zero and come from `Arc::into_raw(Arc<String>)`. It must
/// transfer exactly one live strong share to this call and must not be used as
/// an owner afterward. This function adopts and releases that input share
/// exactly once. The returned bits own one new transferable `Arc<String>`
/// share under the contract documented by [`jit_format_default_i64`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_default_string(bits: u64) -> u64 {
    // SAFETY: upheld by the exported function's transfer contract.
    let input = unsafe { Arc::<String>::from_raw(bits as *const String) };
    let rendered = format_interpolation(
        InterpolationValue::String(input.as_str()),
        InterpolationFormatPolicy::Default,
    );
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe { return_carrier(rendered) }
}

/// Format a proven `int` value with fixed precision.
///
/// # Safety
///
/// The returned bits carry one transferable `Arc<String>` strong share as
/// documented by [`jit_format_default_i64`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_fixed_i64(value: i64, precision: u8) -> u64 {
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe {
        return_carrier(format_interpolation(
            InterpolationValue::Int(value),
            InterpolationFormatPolicy::Fixed {
                precision: precision as usize,
            },
        ))
    }
}

/// Format a proven `bool` value with fixed policy (a textual no-op).
///
/// # Safety
///
/// `value` must be the canonical native bool byte (`0` or `1`). The returned
/// bits carry one transferable `Arc<String>` strong share as documented by
/// [`jit_format_default_i64`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_fixed_bool(value: u8, precision: u8) -> u64 {
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe {
        return_carrier(format_interpolation(
            InterpolationValue::Bool(value != 0),
            InterpolationFormatPolicy::Fixed {
                precision: precision as usize,
            },
        ))
    }
}

/// Format a proven `number` value with fixed precision.
///
/// # Safety
///
/// The returned bits carry one transferable `Arc<String>` strong share as
/// documented by [`jit_format_default_i64`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_fixed_f64(value: f64, precision: u8) -> u64 {
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe {
        return_carrier(format_interpolation(
            InterpolationValue::Number(value),
            InterpolationFormatPolicy::Fixed {
                precision: precision as usize,
            },
        ))
    }
}

/// Consume one proven `Arc<String>` raw share and apply fixed policy (a
/// textual no-op).
///
/// # Safety
///
/// `bits` follows the exact non-null one-share transfer contract documented by
/// [`jit_format_default_string`]. The returned bits own one new transferable
/// `Arc<String>` share.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_format_fixed_string(bits: u64, precision: u8) -> u64 {
    // SAFETY: upheld by the exported function's transfer contract.
    let input = unsafe { Arc::<String>::from_raw(bits as *const String) };
    let rendered = format_interpolation(
        InterpolationValue::String(input.as_str()),
        InterpolationFormatPolicy::Fixed {
            precision: precision as usize,
        },
    );
    // SAFETY: the exported return contract transfers exactly one raw share.
    unsafe { return_carrier(rendered) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adopt the one returned carrier share and clone its text for assertion.
    ///
    /// # Safety
    ///
    /// `bits` must be a fresh return from one formatting entry and must not be
    /// adopted elsewhere.
    unsafe fn take_rendered(bits: u64) -> String {
        // SAFETY: upheld by this helper's one-share input contract.
        let value = unsafe { Arc::<String>::from_raw(bits as *const String) };
        (*value).clone()
    }

    #[test]
    fn typed_default_entries_return_canonical_string_carriers() {
        // SAFETY: direct scalars satisfy each typed entry's input contract;
        // each returned carrier is adopted exactly once by `take_rendered`.
        unsafe {
            assert_eq!(take_rendered(jit_format_default_i64(42)), "42");
            assert_eq!(take_rendered(jit_format_default_bool(1)), "true");
            assert_eq!(take_rendered(jit_format_default_f64(1.5)), "1.5");
            assert_eq!(take_rendered(jit_format_default_f64(1.0)), "1.0");
            assert_eq!(take_rendered(jit_format_default_f64(f64::NAN)), "NaN");
            assert_eq!(
                take_rendered(jit_format_default_f64(f64::INFINITY)),
                "Infinity"
            );
        }
    }

    #[test]
    fn typed_string_entries_consume_one_share_and_return_one_fresh_share() {
        let input = Arc::into_raw(Arc::new("hello".to_string())) as u64;
        // SAFETY: input transfers its only raw Arc share to the entry; output
        // is adopted exactly once by `take_rendered`.
        unsafe {
            assert_eq!(take_rendered(jit_format_default_string(input)), "hello");
        }

        let input = Arc::into_raw(Arc::new("hello".to_string())) as u64;
        // SAFETY: same one-share transfer, with total fixed metadata.
        unsafe {
            assert_eq!(take_rendered(jit_format_fixed_string(input, 2)), "hello");
        }
    }

    #[test]
    fn typed_fixed_entries_use_shared_vm_policy() {
        // SAFETY: direct scalars satisfy each typed entry's input contract;
        // each returned carrier is adopted exactly once.
        unsafe {
            assert_eq!(take_rendered(jit_format_fixed_i64(7, 2)), "7.00");
            assert_eq!(take_rendered(jit_format_fixed_f64(1.5, 3)), "1.500");
            assert_eq!(take_rendered(jit_format_fixed_bool(1, 2)), "true");
        }
    }
}
