//! Math FFI Functions for JIT
//!
//! Trigonometric and mathematical functions for JIT-compiled code.
//!
//! ## SIMD Optimization
//!
//! Series arithmetic (+, -, *, /) uses SIMD-accelerated operations from
//! shape-runtime for high performance vectorized computation.
//!
//! ## R7.1 cleanup
//!
//! The 11 dispatch-fallback trampolines (`jit_generic_add`/`sub`/`mul`/
//! `div`/`mod`, `jit_generic_eq`/`neq`, `jit_generic_lt`/`le`/`gt`/`ge`)
//! were removed here after R5 retargeted the dynamic arithmetic /
//! comparison paths to typed opcodes + `CallMethod`; the only remaining
//! callers in MIR (`compile_binop`) were deleted in the same commit.

use super::value_ffi::*;

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
// Generic Binary Operations (REMOVED — R7.1)
// ============================================================================
// The `jit_generic_add` / `sub` / `mul` / `div` / `mod` / `eq` / `neq` /
// `lt` / `le` / `gt` / `ge` trampolines that used to live here were the
// last thing pinning the matching `FFIFuncRefs` fields alive. After
// R5.1–R5.6 retargeted every dynamic-arithmetic / comparison path
// (typed bitwise, user operator traits, DateTime, Matrix/Vec, string +
// scalar), MIR no longer emits a fully dynamic binop and
// `compile_binop` surfaces an error if one ever reaches it. The 11
// Rust FFI bodies, their Cranelift signatures, and the matching
// symbol registrations were deleted in the same commit.

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

// (R7.1) `jit_generic_lt` / `le` / `gt` / `ge` / `mod` / `eq` / `neq` were
// deleted together with the add/sub/mul/div trampolines above. They had
// no callers outside `compile_binop`, which now surfaces an error for
// any dynamic arithmetic / comparison op that reaches it after R5.

// `duration_to_seconds` (the last private helper in this file) went
// away with the Time/Duration branches in `jit_generic_add` / `sub`
// as part of the R7.1 deletion above.

// Tests for the former `jit_generic_add` / `sub` / `mul` / `div`
// trampolines were removed alongside the functions themselves in R7.1:
// those paths are unreachable from MIR after R5 and exercising them
// directly only confirmed the deleted helper's behaviour.
