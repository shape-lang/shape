// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 5 sites
//     box_ok, box_err, box_some — jit_make_ok, jit_make_err, jit_make_some
//     (these use sub-tag encoding, not jit_box — allocation via Box::into_raw
//      in the Ok/Err/Some wrapper fns in nan_boxing.rs)
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Result Type FFI Functions for JIT
//!
//! Functions for creating and manipulating Result types (Ok/Err) in JIT-compiled code.

use super::super::nan_boxing::*;

// ============================================================================
// Result Type Creation
// ============================================================================

/// Create an Ok result wrapping the inner value
pub extern "C" fn jit_make_ok(inner_bits: u64) -> u64 {
    box_ok(inner_bits)
}

/// Create an Err result wrapping the inner value
pub extern "C" fn jit_make_err(inner_bits: u64) -> u64 {
    box_err(inner_bits)
}

// ============================================================================
// Result Type Checking
// ============================================================================

/// Check if a value is Ok (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_ok(bits: u64) -> u64 {
    if is_ok_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Check if a value is Err (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_err(bits: u64) -> u64 {
    if is_err_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Check if a value is any Result type (Ok or Err)
pub extern "C" fn jit_is_result(bits: u64) -> u64 {
    if is_result_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

// ============================================================================
// Result Type Unwrapping
// ============================================================================

/// Unwrap an Ok value, returning the inner value
/// If not Ok, returns TAG_NULL
pub extern "C" fn jit_unwrap_ok(bits: u64) -> u64 {
    if is_ok_tag(bits) {
        unsafe { unbox_result_inner(bits) }
    } else {
        TAG_NULL
    }
}

/// Unwrap an Err value, returning the inner value
/// If not Err, returns TAG_NULL
pub extern "C" fn jit_unwrap_err(bits: u64) -> u64 {
    if is_err_tag(bits) {
        unsafe { unbox_result_inner(bits) }
    } else {
        TAG_NULL
    }
}

/// Unwrap Ok or return default value
/// If Ok, returns the inner value; otherwise returns the default
pub extern "C" fn jit_unwrap_or(bits: u64, default_bits: u64) -> u64 {
    if is_ok_tag(bits) {
        unsafe { unbox_result_inner(bits) }
    } else {
        default_bits
    }
}

// ============================================================================
// Result Type Transformation
// ============================================================================

/// Map over Ok value - if Ok, applies function and returns new Ok
/// This is a simplified version that just returns the inner value for now
/// (full map support would require function call machinery)
pub extern "C" fn jit_result_inner(bits: u64) -> u64 {
    if is_ok_tag(bits) || is_err_tag(bits) {
        unsafe { unbox_result_inner(bits) }
    } else {
        bits
    }
}

// ============================================================================
// Option Type Functions
// ============================================================================

/// Create a Some value wrapping the inner value
pub extern "C" fn jit_make_some(inner_bits: u64) -> u64 {
    box_some(inner_bits)
}

/// Check if a value is Some (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_some(bits: u64) -> u64 {
    if is_some_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Check if a value is None (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_none(bits: u64) -> u64 {
    if is_none_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Unwrap a Some value, returning the inner value
/// If not Some, returns TAG_NULL
pub extern "C" fn jit_unwrap_some(bits: u64) -> u64 {
    if is_some_tag(bits) {
        unsafe { unbox_some_inner(bits) }
    } else {
        TAG_NULL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_ok_roundtrip() {
        // Test that Ok wrapping and unwrapping preserves the value
        // This test previously caused SIGSEGV due to 44-bit pointer truncation
        let inner = box_number(42.0);
        let ok_result = jit_make_ok(inner);

        assert_eq!(jit_is_ok(ok_result), TAG_BOOL_TRUE);
        assert_eq!(jit_is_err(ok_result), TAG_BOOL_FALSE);
        assert_eq!(jit_is_result(ok_result), TAG_BOOL_TRUE);

        let unwrapped = jit_unwrap_ok(ok_result);
        assert_eq!(unbox_number(unwrapped), 42.0);
    }

    #[test]
    fn test_result_err_roundtrip() {
        // Test that Err wrapping and unwrapping preserves the value
        let inner = box_number(-1.0);
        let err_result = jit_make_err(inner);

        assert_eq!(jit_is_ok(err_result), TAG_BOOL_FALSE);
        assert_eq!(jit_is_err(err_result), TAG_BOOL_TRUE);
        assert_eq!(jit_is_result(err_result), TAG_BOOL_TRUE);

        let unwrapped = jit_unwrap_err(err_result);
        assert_eq!(unbox_number(unwrapped), -1.0);
    }

    #[test]
    fn test_unwrap_or_with_ok() {
        let ok_result = jit_make_ok(box_number(100.0));
        let default = box_number(0.0);

        let result = jit_unwrap_or(ok_result, default);
        assert_eq!(unbox_number(result), 100.0);
    }

    #[test]
    fn test_unwrap_or_with_err() {
        let err_result = jit_make_err(box_number(-1.0));
        let default = box_number(999.0);

        let result = jit_unwrap_or(err_result, default);
        assert_eq!(unbox_number(result), 999.0);
    }

    #[test]
    fn test_option_some_roundtrip() {
        // This test previously caused SIGSEGV due to 44-bit pointer truncation
        let inner = box_number(3.14159);
        let some_opt = jit_make_some(inner);

        assert_eq!(jit_is_some(some_opt), TAG_BOOL_TRUE);
        assert_eq!(jit_is_none(some_opt), TAG_BOOL_FALSE);

        let unwrapped = jit_unwrap_some(some_opt);
        assert!((unbox_number(unwrapped) - 3.14159).abs() < 0.0001);
    }

    #[test]
    fn test_option_none() {
        // TAG_NULL represents None
        assert_eq!(jit_is_none(TAG_NULL), TAG_BOOL_TRUE);
        assert_eq!(jit_is_some(TAG_NULL), TAG_BOOL_FALSE);
    }

    #[test]
    fn test_non_result_values() {
        // Regular numbers should not be results
        let num = box_number(42.0);
        assert_eq!(jit_is_result(num), TAG_BOOL_FALSE);
        assert_eq!(jit_is_ok(num), TAG_BOOL_FALSE);
        assert_eq!(jit_is_err(num), TAG_BOOL_FALSE);
    }

    #[test]
    fn test_result_inner() {
        // Test jit_result_inner which extracts inner regardless of Ok/Err
        let ok_val = jit_make_ok(box_number(123.0));
        let err_val = jit_make_err(box_number(456.0));

        let ok_inner = jit_result_inner(ok_val);
        let err_inner = jit_result_inner(err_val);

        assert_eq!(unbox_number(ok_inner), 123.0);
        assert_eq!(unbox_number(err_inner), 456.0);
    }
}
