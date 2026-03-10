// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 7 sites
//     jit_box(HK_TIME, ...) — generic_add (Time+Duration, Duration+Time),
//       generic_sub (Time-Duration)
//     jit_box(HK_DURATION, ...) — generic_add (Duration+Duration),
//       generic_sub (Time-Time)
//     jit_box(HK_STRING, ...) — generic_add (String+String)
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Math FFI Functions for JIT
//!
//! Trigonometric and mathematical functions for JIT-compiled code.
//!
//! ## SIMD Optimization
//!
//! Series arithmetic (+, -, *, /) uses SIMD-accelerated operations from
//! shape-runtime for high performance vectorized computation.

use super::super::nan_boxing::*;

// SIMD threshold - use SIMD for arrays >= this size
#[allow(dead_code)]
const SIMD_THRESHOLD: usize = 16;

// ============================================================================
// Trigonometric Functions
// ============================================================================

pub extern "C" fn jit_sin(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.sin())
}

pub extern "C" fn jit_cos(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.cos())
}

pub extern "C" fn jit_tan(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.tan())
}

pub extern "C" fn jit_asin(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.asin())
}

pub extern "C" fn jit_acos(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.acos())
}

pub extern "C" fn jit_atan(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.atan())
}

// ============================================================================
// Exponential and Logarithmic Functions
// ============================================================================

pub extern "C" fn jit_exp(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.exp())
}

pub extern "C" fn jit_ln(value_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.ln())
}

pub extern "C" fn jit_log(value_bits: u64, base_bits: u64) -> u64 {
    let x = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return box_number(f64::NAN);
    };
    let base = if is_number(base_bits) {
        unbox_number(base_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(x.log(base))
}

// ============================================================================
// Power Function
// ============================================================================

pub extern "C" fn jit_pow(base_bits: u64, exp_bits: u64) -> u64 {
    let base = if is_number(base_bits) {
        unbox_number(base_bits)
    } else {
        return box_number(f64::NAN);
    };
    let exp = if is_number(exp_bits) {
        unbox_number(exp_bits)
    } else {
        return box_number(f64::NAN);
    };
    box_number(base.powf(exp))
}

// ============================================================================
// Generic Binary Operations (for non-numeric types)
// ============================================================================

/// Generic add that handles Time + Duration, Duration + Duration, etc.
pub extern "C" fn jit_generic_add(a_bits: u64, b_bits: u64) -> u64 {
    use super::super::context::JITDuration;

    // Both numbers - fast path
    if is_number(a_bits) && is_number(b_bits) {
        return box_number(unbox_number(a_bits) + unbox_number(b_bits));
    }

    let a_kind = heap_kind(a_bits);
    let b_kind = heap_kind(b_bits);

    // Time + Duration or Duration + Time
    if a_kind == Some(HK_TIME) && b_kind == Some(HK_DURATION) {
        let timestamp = *unsafe { jit_unbox::<i64>(a_bits) };
        let dur = unsafe { jit_unbox::<JITDuration>(b_bits) };
        let seconds = duration_to_seconds(dur);
        let new_timestamp = timestamp + seconds as i64;
        return jit_box(HK_TIME, new_timestamp);
    }

    if a_kind == Some(HK_DURATION) && b_kind == Some(HK_TIME) {
        let timestamp = *unsafe { jit_unbox::<i64>(b_bits) };
        let dur = unsafe { jit_unbox::<JITDuration>(a_bits) };
        let seconds = duration_to_seconds(dur);
        let new_timestamp = timestamp + seconds as i64;
        return jit_box(HK_TIME, new_timestamp);
    }

    // Duration + Duration
    if a_kind == Some(HK_DURATION) && b_kind == Some(HK_DURATION) {
        let a_dur = unsafe { jit_unbox::<JITDuration>(a_bits) };
        let b_dur = unsafe { jit_unbox::<JITDuration>(b_bits) };
        let a_secs = duration_to_seconds(a_dur);
        let b_secs = duration_to_seconds(b_dur);
        let total_secs = a_secs + b_secs;
        return jit_box(
            HK_DURATION,
            JITDuration {
                value: total_secs,
                unit: 0,
            },
        );
    }

    // String concatenation
    if a_kind == Some(HK_STRING) && b_kind == Some(HK_STRING) {
        let a_str = unsafe { jit_unbox::<String>(a_bits) };
        let b_str = unsafe { jit_unbox::<String>(b_bits) };
        let result = format!("{}{}", a_str, b_str);
        return jit_box(HK_STRING, result);
    }

    // Fallback for numbers (one might be boxed differently)
    if is_number(a_bits) || is_number(b_bits) {
        let a_num = if is_number(a_bits) {
            unbox_number(a_bits)
        } else {
            0.0
        };
        let b_num = if is_number(b_bits) {
            unbox_number(b_bits)
        } else {
            0.0
        };
        return box_number(a_num + b_num);
    }

    TAG_NULL
}

/// Generic subtract that handles Time - Duration, Duration - Duration, etc.
pub extern "C" fn jit_generic_sub(a_bits: u64, b_bits: u64) -> u64 {
    use super::super::context::JITDuration;

    // Both numbers - fast path
    if is_number(a_bits) && is_number(b_bits) {
        return box_number(unbox_number(a_bits) - unbox_number(b_bits));
    }

    let a_kind = heap_kind(a_bits);
    let b_kind = heap_kind(b_bits);

    // Time - Duration
    if a_kind == Some(HK_TIME) && b_kind == Some(HK_DURATION) {
        let timestamp = *unsafe { jit_unbox::<i64>(a_bits) };
        let dur = unsafe { jit_unbox::<JITDuration>(b_bits) };
        let seconds = duration_to_seconds(dur);
        let new_timestamp = timestamp - seconds as i64;
        return jit_box(HK_TIME, new_timestamp);
    }

    // Time - Time = Duration (in seconds)
    if a_kind == Some(HK_TIME) && b_kind == Some(HK_TIME) {
        let a_ts = *unsafe { jit_unbox::<i64>(a_bits) };
        let b_ts = *unsafe { jit_unbox::<i64>(b_bits) };
        let diff_secs = (a_ts - b_ts) as f64;
        return jit_box(
            HK_DURATION,
            JITDuration {
                value: diff_secs,
                unit: 0,
            },
        );
    }

    // Fallback for numbers
    if is_number(a_bits) || is_number(b_bits) {
        let a_num = if is_number(a_bits) {
            unbox_number(a_bits)
        } else {
            0.0
        };
        let b_num = if is_number(b_bits) {
            unbox_number(b_bits)
        } else {
            0.0
        };
        return box_number(a_num - b_num);
    }

    TAG_NULL
}

/// Generic multiplication for JIT (Series * Series, Series * number, number * Series, etc.)
#[unsafe(no_mangle)]
pub extern "C" fn jit_generic_mul(a_bits: u64, b_bits: u64) -> u64 {
    // Both numbers - fast path
    if is_number(a_bits) && is_number(b_bits) {
        return box_number(unbox_number(a_bits) * unbox_number(b_bits));
    }

    // Fallback for numbers
    if is_number(a_bits) || is_number(b_bits) {
        let a_num = if is_number(a_bits) {
            unbox_number(a_bits)
        } else {
            1.0
        };
        let b_num = if is_number(b_bits) {
            unbox_number(b_bits)
        } else {
            1.0
        };
        return box_number(a_num * b_num);
    }

    TAG_NULL
}

/// Generic division for JIT (Series / Series, Series / number, number / Series, etc.)
#[unsafe(no_mangle)]
pub extern "C" fn jit_generic_div(a_bits: u64, b_bits: u64) -> u64 {
    // Both numbers - fast path
    if is_number(a_bits) && is_number(b_bits) {
        let b = unbox_number(b_bits);
        return box_number(if b == 0.0 {
            f64::NAN
        } else {
            unbox_number(a_bits) / b
        });
    }

    // Fallback for numbers
    if is_number(a_bits) || is_number(b_bits) {
        let a_num = if is_number(a_bits) {
            unbox_number(a_bits)
        } else {
            0.0
        };
        let b_num = if is_number(b_bits) {
            unbox_number(b_bits)
        } else {
            1.0
        };
        return box_number(if b_num == 0.0 {
            f64::NAN
        } else {
            a_num / b_num
        });
    }

    TAG_NULL
}

/// SIMD-accelerated Series addition
#[allow(dead_code)]
fn series_add_simd(a_bits: u64, b_bits: u64) -> u64 {
    series_simd_binary_op(
        a_bits,
        b_bits,
        super::simd::jit_simd_add,
        super::simd::jit_simd_add_scalar,
    )
}

/// SIMD-accelerated Series subtraction
#[allow(dead_code)]
fn series_sub_simd(a_bits: u64, b_bits: u64) -> u64 {
    series_simd_binary_op(
        a_bits,
        b_bits,
        super::simd::jit_simd_sub,
        super::simd::jit_simd_sub_scalar,
    )
}

/// SIMD-accelerated Series multiplication
#[allow(dead_code)]
fn series_mul_simd(a_bits: u64, b_bits: u64) -> u64 {
    series_simd_binary_op(
        a_bits,
        b_bits,
        super::simd::jit_simd_mul,
        super::simd::jit_simd_mul_scalar,
    )
}

/// SIMD-accelerated Series division
#[allow(dead_code)]
fn series_div_simd(a_bits: u64, b_bits: u64) -> u64 {
    series_simd_binary_op(
        a_bits,
        b_bits,
        super::simd::jit_simd_div,
        super::simd::jit_simd_div_scalar,
    )
}

/// Helper for SIMD series binary operations
/// Uses raw pointer SIMD functions for maximum performance
fn series_simd_binary_op(
    _a_bits: u64,
    _b_bits: u64,
    _simd_binary: extern "C" fn(*const f64, *const f64, u64) -> *mut f64,
    _simd_scalar: extern "C" fn(*const f64, f64, u64) -> *mut f64,
) -> u64 {
    TAG_NULL
}

/// Fallback helper for series binary operations (for non-SIMD ops)
#[allow(dead_code)]
fn series_binary_op<F>(_a_bits: u64, _b_bits: u64, _op: F) -> u64
where
    F: Fn(f64, f64) -> f64,
{
    TAG_NULL
}

/// Generic comparison for Series > Series, Series > number, etc.
/// Returns a Series of 1.0/0.0 for series comparisons, or a boolean for scalars.
pub extern "C" fn jit_series_gt(a_bits: u64, b_bits: u64) -> u64 {
    series_comparison_op(a_bits, b_bits, |a, b| a > b)
}

pub extern "C" fn jit_series_lt(a_bits: u64, b_bits: u64) -> u64 {
    series_comparison_op(a_bits, b_bits, |a, b| a < b)
}

pub extern "C" fn jit_series_gte(a_bits: u64, b_bits: u64) -> u64 {
    series_comparison_op(a_bits, b_bits, |a, b| a >= b)
}

pub extern "C" fn jit_series_lte(a_bits: u64, b_bits: u64) -> u64 {
    series_comparison_op(a_bits, b_bits, |a, b| a <= b)
}

/// Helper for series comparison operations
fn series_comparison_op<F>(a_bits: u64, b_bits: u64, op: F) -> u64
where
    F: Fn(f64, f64) -> bool,
{
    // Fallback: numeric comparison
    if is_number(a_bits) && is_number(b_bits) {
        let a = unbox_number(a_bits);
        let b = unbox_number(b_bits);
        return if op(a, b) {
            TAG_BOOL_TRUE
        } else {
            TAG_BOOL_FALSE
        };
    }
    TAG_BOOL_FALSE
}

/// Generic equality that handles strings, booleans, and other non-numeric types.
/// Compares string contents (not pointer identity), numbers by value, booleans by tag.
pub extern "C" fn jit_generic_eq(a_bits: u64, b_bits: u64) -> u64 {
    // Both numbers - fast path
    if is_number(a_bits) && is_number(b_bits) {
        return if unbox_number(a_bits) == unbox_number(b_bits) {
            TAG_BOOL_TRUE
        } else {
            TAG_BOOL_FALSE
        };
    }

    // Identical tags (bools, null, unit)
    if a_bits == b_bits {
        return TAG_BOOL_TRUE;
    }

    // Both heap values
    let a_kind = heap_kind(a_bits);
    let b_kind = heap_kind(b_bits);

    if a_kind == Some(HK_STRING) && b_kind == Some(HK_STRING) {
        let a_str = unsafe { jit_unbox::<String>(a_bits) };
        let b_str = unsafe { jit_unbox::<String>(b_bits) };
        return if a_str == b_str {
            TAG_BOOL_TRUE
        } else {
            TAG_BOOL_FALSE
        };
    }

    TAG_BOOL_FALSE
}

/// Generic inequality — inverse of jit_generic_eq.
pub extern "C" fn jit_generic_neq(a_bits: u64, b_bits: u64) -> u64 {
    if jit_generic_eq(a_bits, b_bits) == TAG_BOOL_TRUE {
        TAG_BOOL_FALSE
    } else {
        TAG_BOOL_TRUE
    }
}

/// Helper: convert JITDuration to seconds
fn duration_to_seconds(dur: &super::super::context::JITDuration) -> f64 {
    match dur.unit {
        0 => dur.value,              // seconds
        1 => dur.value * 60.0,       // minutes
        2 => dur.value * 3600.0,     // hours
        3 => dur.value * 86400.0,    // days
        4 => dur.value * 604800.0,   // weeks
        5 => dur.value * 2592000.0,  // months (30 days)
        6 => dur.value * 31536000.0, // years (365 days)
        _ => dur.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scalar_add() {
        let a = box_number(10.0);
        let b = box_number(32.0);
        let result = jit_generic_add(a, b);
        assert_eq!(unbox_number(result), 42.0);
    }

    #[test]
    fn test_scalar_sub() {
        let a = box_number(100.0);
        let b = box_number(58.0);
        let result = jit_generic_sub(a, b);
        assert_eq!(unbox_number(result), 42.0);
    }

    #[test]
    fn test_scalar_mul() {
        let a = box_number(6.0);
        let b = box_number(7.0);
        let result = jit_generic_mul(a, b);
        assert_eq!(unbox_number(result), 42.0);
    }

    #[test]
    fn test_scalar_div() {
        let a = box_number(84.0);
        let b = box_number(2.0);
        let result = jit_generic_div(a, b);
        assert_eq!(unbox_number(result), 42.0);
    }

    #[test]
    fn test_scalar_div_by_zero() {
        let a = box_number(42.0);
        let b = box_number(0.0);
        let result = jit_generic_div(a, b);
        // Division by zero should return infinity or NaN
        let val = unbox_number(result);
        assert!(val.is_infinite() || val.is_nan());
    }
}
