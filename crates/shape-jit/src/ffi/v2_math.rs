//! Monomorphized Typed FFI Math Functions (v2 Runtime)
//!
//! These functions operate on native types (f64, i64, i32) directly,
//! eliminating NaN-boxing overhead. They will replace the generic
//! NaN-boxed math FFI functions as the v2 runtime migration progresses.

// ============================================================================
// f64 Arithmetic
// ============================================================================

#[inline]
pub extern "C" fn jit_add_f64(a: f64, b: f64) -> f64 {
    a + b
}

#[inline]
pub extern "C" fn jit_sub_f64(a: f64, b: f64) -> f64 {
    a - b
}

#[inline]
pub extern "C" fn jit_mul_f64(a: f64, b: f64) -> f64 {
    a * b
}

#[inline]
pub extern "C" fn jit_div_f64(a: f64, b: f64) -> f64 {
    a / b
}

#[inline]
pub extern "C" fn jit_mod_f64(a: f64, b: f64) -> f64 {
    a % b
}

#[inline]
pub extern "C" fn jit_neg_f64(a: f64) -> f64 {
    -a
}

// ============================================================================
// f64 Comparisons (return u8: 1 = true, 0 = false)
// ============================================================================

#[inline]
pub extern "C" fn jit_cmp_lt_f64(a: f64, b: f64) -> u8 {
    (a < b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_le_f64(a: f64, b: f64) -> u8 {
    (a <= b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_gt_f64(a: f64, b: f64) -> u8 {
    (a > b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_ge_f64(a: f64, b: f64) -> u8 {
    (a >= b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_eq_f64(a: f64, b: f64) -> u8 {
    (a == b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_ne_f64(a: f64, b: f64) -> u8 {
    (a != b) as u8
}

// ============================================================================
// i64 Arithmetic (wrapping to avoid UB on overflow)
// ============================================================================

#[inline]
pub extern "C" fn jit_add_i64(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

#[inline]
pub extern "C" fn jit_sub_i64(a: i64, b: i64) -> i64 {
    a.wrapping_sub(b)
}

#[inline]
pub extern "C" fn jit_mul_i64(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}

#[inline]
pub extern "C" fn jit_div_i64(a: i64, b: i64) -> i64 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

#[inline]
pub extern "C" fn jit_mod_i64(a: i64, b: i64) -> i64 {
    if b == 0 {
        0
    } else {
        a.wrapping_rem(b)
    }
}

#[inline]
pub extern "C" fn jit_neg_i64(a: i64) -> i64 {
    a.wrapping_neg()
}

// ============================================================================
// i64 Comparisons
// ============================================================================

#[inline]
pub extern "C" fn jit_cmp_lt_i64(a: i64, b: i64) -> u8 {
    (a < b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_le_i64(a: i64, b: i64) -> u8 {
    (a <= b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_gt_i64(a: i64, b: i64) -> u8 {
    (a > b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_ge_i64(a: i64, b: i64) -> u8 {
    (a >= b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_eq_i64(a: i64, b: i64) -> u8 {
    (a == b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_ne_i64(a: i64, b: i64) -> u8 {
    (a != b) as u8
}

// ============================================================================
// i32 Arithmetic (wrapping to avoid UB on overflow)
// ============================================================================

#[inline]
pub extern "C" fn jit_add_i32(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

#[inline]
pub extern "C" fn jit_sub_i32(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}

#[inline]
pub extern "C" fn jit_mul_i32(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}

#[inline]
pub extern "C" fn jit_div_i32(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

#[inline]
pub extern "C" fn jit_mod_i32(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_rem(b)
    }
}

#[inline]
pub extern "C" fn jit_neg_i32(a: i32) -> i32 {
    a.wrapping_neg()
}

// ============================================================================
// i32 Comparisons
// ============================================================================

#[inline]
pub extern "C" fn jit_cmp_lt_i32(a: i32, b: i32) -> u8 {
    (a < b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_le_i32(a: i32, b: i32) -> u8 {
    (a <= b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_gt_i32(a: i32, b: i32) -> u8 {
    (a > b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_ge_i32(a: i32, b: i32) -> u8 {
    (a >= b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_eq_i32(a: i32, b: i32) -> u8 {
    (a == b) as u8
}

#[inline]
pub extern "C" fn jit_cmp_ne_i32(a: i32, b: i32) -> u8 {
    (a != b) as u8
}

// ============================================================================
// f64 Math Functions
// ============================================================================

#[inline]
pub extern "C" fn jit_sqrt_f64(a: f64) -> f64 {
    a.sqrt()
}

#[inline]
pub extern "C" fn jit_abs_f64(a: f64) -> f64 {
    a.abs()
}

#[inline]
pub extern "C" fn jit_floor_f64(a: f64) -> f64 {
    a.floor()
}

#[inline]
pub extern "C" fn jit_ceil_f64(a: f64) -> f64 {
    a.ceil()
}

#[inline]
pub extern "C" fn jit_round_f64(a: f64) -> f64 {
    a.round()
}

#[inline]
pub extern "C" fn jit_sin_f64(a: f64) -> f64 {
    a.sin()
}

#[inline]
pub extern "C" fn jit_cos_f64(a: f64) -> f64 {
    a.cos()
}

#[inline]
pub extern "C" fn jit_tan_f64(a: f64) -> f64 {
    a.tan()
}

#[inline]
pub extern "C" fn jit_asin_f64(a: f64) -> f64 {
    a.asin()
}

#[inline]
pub extern "C" fn jit_acos_f64(a: f64) -> f64 {
    a.acos()
}

#[inline]
pub extern "C" fn jit_atan_f64(a: f64) -> f64 {
    a.atan()
}

#[inline]
pub extern "C" fn jit_exp_f64(a: f64) -> f64 {
    a.exp()
}

#[inline]
pub extern "C" fn jit_ln_f64(a: f64) -> f64 {
    a.ln()
}

#[inline]
pub extern "C" fn jit_log_f64(a: f64, base: f64) -> f64 {
    a.log(base)
}

#[inline]
pub extern "C" fn jit_pow_f64(a: f64, b: f64) -> f64 {
    a.powf(b)
}

// ============================================================================
// i64 Math Functions
// ============================================================================

#[inline]
pub extern "C" fn jit_abs_i64(a: i64) -> i64 {
    a.wrapping_abs()
}

// ============================================================================
// Type Conversions
// ============================================================================

#[inline]
pub extern "C" fn jit_i64_to_f64(a: i64) -> f64 {
    a as f64
}

#[inline]
pub extern "C" fn jit_f64_to_i64(a: f64) -> i64 {
    a as i64
}

#[inline]
pub extern "C" fn jit_i32_to_f64(a: i32) -> f64 {
    a as f64
}

#[inline]
pub extern "C" fn jit_f64_to_i32(a: f64) -> i32 {
    a as i32
}

#[inline]
pub extern "C" fn jit_i32_to_i64(a: i32) -> i64 {
    a as i64
}

#[inline]
pub extern "C" fn jit_i64_to_i32(a: i64) -> i32 {
    a as i32
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- f64 arithmetic ---

    #[test]
    fn test_v2_math_add_f64() {
        assert_eq!(jit_add_f64(10.0, 32.0), 42.0);
        assert_eq!(jit_add_f64(-1.5, 2.5), 1.0);
        assert_eq!(jit_add_f64(0.0, 0.0), 0.0);
    }

    #[test]
    fn test_v2_math_sub_f64() {
        assert_eq!(jit_sub_f64(100.0, 58.0), 42.0);
        assert_eq!(jit_sub_f64(0.0, 1.0), -1.0);
    }

    #[test]
    fn test_v2_math_mul_f64() {
        assert_eq!(jit_mul_f64(6.0, 7.0), 42.0);
        assert_eq!(jit_mul_f64(-3.0, 2.0), -6.0);
        assert_eq!(jit_mul_f64(0.0, 999.0), 0.0);
    }

    #[test]
    fn test_v2_math_div_f64() {
        assert_eq!(jit_div_f64(84.0, 2.0), 42.0);
        assert!(jit_div_f64(1.0, 0.0).is_infinite());
        assert!(jit_div_f64(0.0, 0.0).is_nan());
    }

    #[test]
    fn test_v2_math_mod_f64() {
        assert_eq!(jit_mod_f64(10.0, 3.0), 1.0);
        assert_eq!(jit_mod_f64(10.0, 5.0), 0.0);
    }

    #[test]
    fn test_v2_math_neg_f64() {
        assert_eq!(jit_neg_f64(42.0), -42.0);
        assert_eq!(jit_neg_f64(-1.0), 1.0);
        assert_eq!(jit_neg_f64(0.0), 0.0);
    }

    // --- f64 comparisons ---

    #[test]
    fn test_v2_math_cmp_f64() {
        assert_eq!(jit_cmp_lt_f64(1.0, 2.0), 1);
        assert_eq!(jit_cmp_lt_f64(2.0, 1.0), 0);
        assert_eq!(jit_cmp_lt_f64(1.0, 1.0), 0);

        assert_eq!(jit_cmp_le_f64(1.0, 2.0), 1);
        assert_eq!(jit_cmp_le_f64(1.0, 1.0), 1);
        assert_eq!(jit_cmp_le_f64(2.0, 1.0), 0);

        assert_eq!(jit_cmp_gt_f64(2.0, 1.0), 1);
        assert_eq!(jit_cmp_gt_f64(1.0, 2.0), 0);
        assert_eq!(jit_cmp_gt_f64(1.0, 1.0), 0);

        assert_eq!(jit_cmp_ge_f64(2.0, 1.0), 1);
        assert_eq!(jit_cmp_ge_f64(1.0, 1.0), 1);
        assert_eq!(jit_cmp_ge_f64(1.0, 2.0), 0);

        assert_eq!(jit_cmp_eq_f64(1.0, 1.0), 1);
        assert_eq!(jit_cmp_eq_f64(1.0, 2.0), 0);

        assert_eq!(jit_cmp_ne_f64(1.0, 2.0), 1);
        assert_eq!(jit_cmp_ne_f64(1.0, 1.0), 0);
    }

    #[test]
    fn test_v2_math_cmp_f64_nan() {
        // NaN comparisons: all should return false (0) except ne
        assert_eq!(jit_cmp_lt_f64(f64::NAN, 1.0), 0);
        assert_eq!(jit_cmp_le_f64(f64::NAN, 1.0), 0);
        assert_eq!(jit_cmp_gt_f64(f64::NAN, 1.0), 0);
        assert_eq!(jit_cmp_ge_f64(f64::NAN, 1.0), 0);
        assert_eq!(jit_cmp_eq_f64(f64::NAN, f64::NAN), 0);
        assert_eq!(jit_cmp_ne_f64(f64::NAN, f64::NAN), 1);
    }

    // --- i64 arithmetic ---

    #[test]
    fn test_v2_math_add_i64() {
        assert_eq!(jit_add_i64(10, 32), 42);
        assert_eq!(jit_add_i64(-1, 1), 0);
        // Wrapping overflow
        assert_eq!(jit_add_i64(i64::MAX, 1), i64::MIN);
    }

    #[test]
    fn test_v2_math_sub_i64() {
        assert_eq!(jit_sub_i64(100, 58), 42);
        assert_eq!(jit_sub_i64(0, 1), -1);
        // Wrapping underflow
        assert_eq!(jit_sub_i64(i64::MIN, 1), i64::MAX);
    }

    #[test]
    fn test_v2_math_mul_i64() {
        assert_eq!(jit_mul_i64(6, 7), 42);
        assert_eq!(jit_mul_i64(-3, 2), -6);
        assert_eq!(jit_mul_i64(0, i64::MAX), 0);
    }

    #[test]
    fn test_v2_math_div_i64() {
        assert_eq!(jit_div_i64(84, 2), 42);
        assert_eq!(jit_div_i64(7, 2), 3); // truncation
        assert_eq!(jit_div_i64(1, 0), 0); // div by zero => 0
        // MIN / -1 would overflow; wrapping_div handles it
        assert_eq!(jit_div_i64(i64::MIN, -1), i64::MIN);
    }

    #[test]
    fn test_v2_math_mod_i64() {
        assert_eq!(jit_mod_i64(10, 3), 1);
        assert_eq!(jit_mod_i64(10, 5), 0);
        assert_eq!(jit_mod_i64(1, 0), 0); // mod by zero => 0
    }

    #[test]
    fn test_v2_math_neg_i64() {
        assert_eq!(jit_neg_i64(42), -42);
        assert_eq!(jit_neg_i64(-1), 1);
        assert_eq!(jit_neg_i64(0), 0);
        // Wrapping: -MIN overflows
        assert_eq!(jit_neg_i64(i64::MIN), i64::MIN);
    }

    // --- i64 comparisons ---

    #[test]
    fn test_v2_math_cmp_i64() {
        assert_eq!(jit_cmp_lt_i64(1, 2), 1);
        assert_eq!(jit_cmp_lt_i64(2, 1), 0);
        assert_eq!(jit_cmp_le_i64(1, 1), 1);
        assert_eq!(jit_cmp_gt_i64(2, 1), 1);
        assert_eq!(jit_cmp_ge_i64(1, 1), 1);
        assert_eq!(jit_cmp_eq_i64(42, 42), 1);
        assert_eq!(jit_cmp_eq_i64(1, 2), 0);
        assert_eq!(jit_cmp_ne_i64(1, 2), 1);
        assert_eq!(jit_cmp_ne_i64(1, 1), 0);
    }

    // --- i32 arithmetic ---

    #[test]
    fn test_v2_math_add_i32() {
        assert_eq!(jit_add_i32(10, 32), 42);
        assert_eq!(jit_add_i32(i32::MAX, 1), i32::MIN);
    }

    #[test]
    fn test_v2_math_sub_i32() {
        assert_eq!(jit_sub_i32(100, 58), 42);
        assert_eq!(jit_sub_i32(i32::MIN, 1), i32::MAX);
    }

    #[test]
    fn test_v2_math_mul_i32() {
        assert_eq!(jit_mul_i32(6, 7), 42);
        assert_eq!(jit_mul_i32(0, i32::MAX), 0);
    }

    #[test]
    fn test_v2_math_div_i32() {
        assert_eq!(jit_div_i32(84, 2), 42);
        assert_eq!(jit_div_i32(1, 0), 0);
        assert_eq!(jit_div_i32(i32::MIN, -1), i32::MIN);
    }

    #[test]
    fn test_v2_math_mod_i32() {
        assert_eq!(jit_mod_i32(10, 3), 1);
        assert_eq!(jit_mod_i32(1, 0), 0);
    }

    #[test]
    fn test_v2_math_neg_i32() {
        assert_eq!(jit_neg_i32(42), -42);
        assert_eq!(jit_neg_i32(i32::MIN), i32::MIN);
    }

    // --- i32 comparisons ---

    #[test]
    fn test_v2_math_cmp_i32() {
        assert_eq!(jit_cmp_lt_i32(1, 2), 1);
        assert_eq!(jit_cmp_lt_i32(2, 1), 0);
        assert_eq!(jit_cmp_le_i32(1, 1), 1);
        assert_eq!(jit_cmp_gt_i32(2, 1), 1);
        assert_eq!(jit_cmp_ge_i32(1, 1), 1);
        assert_eq!(jit_cmp_eq_i32(42, 42), 1);
        assert_eq!(jit_cmp_ne_i32(1, 2), 1);
        assert_eq!(jit_cmp_ne_i32(1, 1), 0);
    }

    // --- f64 math functions ---

    #[test]
    fn test_v2_math_sqrt_f64() {
        assert_eq!(jit_sqrt_f64(4.0), 2.0);
        assert_eq!(jit_sqrt_f64(0.0), 0.0);
        assert!(jit_sqrt_f64(-1.0).is_nan());
    }

    #[test]
    fn test_v2_math_abs_f64() {
        assert_eq!(jit_abs_f64(-42.0), 42.0);
        assert_eq!(jit_abs_f64(42.0), 42.0);
        assert_eq!(jit_abs_f64(0.0), 0.0);
    }

    #[test]
    fn test_v2_math_floor_ceil_round_f64() {
        assert_eq!(jit_floor_f64(2.7), 2.0);
        assert_eq!(jit_floor_f64(-2.3), -3.0);
        assert_eq!(jit_ceil_f64(2.3), 3.0);
        assert_eq!(jit_ceil_f64(-2.7), -2.0);
        assert_eq!(jit_round_f64(2.5), 3.0);
        assert_eq!(jit_round_f64(2.4), 2.0);
    }

    #[test]
    fn test_v2_math_trig_f64() {
        let pi = std::f64::consts::PI;
        assert!((jit_sin_f64(0.0)).abs() < 1e-15);
        assert!((jit_cos_f64(0.0) - 1.0).abs() < 1e-15);
        assert!((jit_sin_f64(pi / 2.0) - 1.0).abs() < 1e-15);
        assert!((jit_tan_f64(0.0)).abs() < 1e-15);
        assert!((jit_asin_f64(1.0) - pi / 2.0).abs() < 1e-15);
        assert!((jit_acos_f64(1.0)).abs() < 1e-15);
        assert!((jit_atan_f64(0.0)).abs() < 1e-15);
    }

    #[test]
    fn test_v2_math_exp_ln_f64() {
        assert!((jit_exp_f64(0.0) - 1.0).abs() < 1e-15);
        assert!((jit_ln_f64(1.0)).abs() < 1e-15);
        assert!((jit_exp_f64(1.0) - std::f64::consts::E).abs() < 1e-14);
        assert!((jit_ln_f64(std::f64::consts::E) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn test_v2_math_log_f64() {
        assert!((jit_log_f64(8.0, 2.0) - 3.0).abs() < 1e-14);
        assert!((jit_log_f64(100.0, 10.0) - 2.0).abs() < 1e-14);
    }

    #[test]
    fn test_v2_math_pow_f64() {
        assert_eq!(jit_pow_f64(2.0, 10.0), 1024.0);
        assert_eq!(jit_pow_f64(3.0, 0.0), 1.0);
        assert_eq!(jit_pow_f64(0.0, 5.0), 0.0);
        assert!((jit_pow_f64(4.0, 0.5) - 2.0).abs() < 1e-15);
    }

    // --- i64 math functions ---

    #[test]
    fn test_v2_math_abs_i64() {
        assert_eq!(jit_abs_i64(-42), 42);
        assert_eq!(jit_abs_i64(42), 42);
        assert_eq!(jit_abs_i64(0), 0);
        // Wrapping: abs(MIN) overflows
        assert_eq!(jit_abs_i64(i64::MIN), i64::MIN);
    }

    // --- type conversions ---

    #[test]
    fn test_v2_math_i64_to_f64() {
        assert_eq!(jit_i64_to_f64(42), 42.0);
        assert_eq!(jit_i64_to_f64(-1), -1.0);
        assert_eq!(jit_i64_to_f64(0), 0.0);
    }

    #[test]
    fn test_v2_math_f64_to_i64() {
        assert_eq!(jit_f64_to_i64(42.9), 42);
        assert_eq!(jit_f64_to_i64(-1.1), -1);
        assert_eq!(jit_f64_to_i64(0.0), 0);
    }

    #[test]
    fn test_v2_math_i32_to_f64() {
        assert_eq!(jit_i32_to_f64(42), 42.0);
        assert_eq!(jit_i32_to_f64(-1), -1.0);
    }

    #[test]
    fn test_v2_math_f64_to_i32() {
        assert_eq!(jit_f64_to_i32(42.9), 42);
        assert_eq!(jit_f64_to_i32(-1.1), -1);
    }

    #[test]
    fn test_v2_math_i32_i64_conversions() {
        assert_eq!(jit_i32_to_i64(42), 42i64);
        assert_eq!(jit_i32_to_i64(-1), -1i64);
        assert_eq!(jit_i64_to_i32(42), 42i32);
        // Truncation
        assert_eq!(jit_i64_to_i32(i64::MAX), -1i32);
    }
}
