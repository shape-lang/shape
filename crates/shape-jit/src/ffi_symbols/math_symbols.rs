//! Math FFI Symbol Registration
//!
//! This module handles registration and declaration of math-related FFI symbols
//! for the JIT compiler, including trigonometric functions, generic arithmetic,
//! series comparisons, and intrinsic statistics.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::math::{
    jit_acos, jit_asin, jit_atan, jit_cos, jit_exp, jit_generic_add, jit_generic_div,
    jit_generic_mul, jit_generic_sub, jit_ln, jit_log, jit_pow, jit_sin, jit_tan,
};
use super::intrinsics::{
    jit_intrinsic_correlation, jit_intrinsic_covariance, jit_intrinsic_max, jit_intrinsic_mean,
    jit_intrinsic_median, jit_intrinsic_min, jit_intrinsic_percentile, jit_intrinsic_std,
    jit_intrinsic_sum, jit_intrinsic_variance, jit_series_broadcast, jit_series_clip,
    jit_series_cumprod, jit_series_diff, jit_series_ema, jit_series_highest_index,
    jit_series_lowest_index, jit_series_pct_change, jit_series_rolling_max, jit_series_rolling_min,
};

/// Register math FFI symbols with the JIT builder
pub fn register_math_symbols(builder: &mut JITBuilder) {
    // Trigonometric functions
    builder.symbol("jit_sin", jit_sin as *const u8);
    builder.symbol("jit_cos", jit_cos as *const u8);
    builder.symbol("jit_tan", jit_tan as *const u8);
    builder.symbol("jit_asin", jit_asin as *const u8);
    builder.symbol("jit_acos", jit_acos as *const u8);
    builder.symbol("jit_atan", jit_atan as *const u8);

    // Exponential and logarithmic functions
    builder.symbol("jit_exp", jit_exp as *const u8);
    builder.symbol("jit_ln", jit_ln as *const u8);
    builder.symbol("jit_log", jit_log as *const u8);
    builder.symbol("jit_pow", jit_pow as *const u8);

    // Generic arithmetic operations
    builder.symbol("jit_generic_add", jit_generic_add as *const u8);
    builder.symbol("jit_generic_sub", jit_generic_sub as *const u8);
    builder.symbol("jit_generic_mul", jit_generic_mul as *const u8);
    builder.symbol("jit_generic_div", jit_generic_div as *const u8);

    // Series comparison functions
    builder.symbol(
        "jit_series_gt",
        super::super::ffi::math::jit_series_gt as *const u8,
    );
    builder.symbol(
        "jit_series_lt",
        super::super::ffi::math::jit_series_lt as *const u8,
    );
    builder.symbol(
        "jit_series_gte",
        super::super::ffi::math::jit_series_gte as *const u8,
    );
    builder.symbol(
        "jit_series_lte",
        super::super::ffi::math::jit_series_lte as *const u8,
    );

    // Intrinsic aggregation functions
    builder.symbol("jit_intrinsic_sum", jit_intrinsic_sum as *const u8);
    builder.symbol("jit_intrinsic_mean", jit_intrinsic_mean as *const u8);
    builder.symbol("jit_intrinsic_min", jit_intrinsic_min as *const u8);
    builder.symbol("jit_intrinsic_max", jit_intrinsic_max as *const u8);
    builder.symbol("jit_intrinsic_std", jit_intrinsic_std as *const u8);
    builder.symbol(
        "jit_intrinsic_variance",
        jit_intrinsic_variance as *const u8,
    );
    builder.symbol("jit_intrinsic_median", jit_intrinsic_median as *const u8);
    builder.symbol(
        "jit_intrinsic_percentile",
        jit_intrinsic_percentile as *const u8,
    );
    builder.symbol(
        "jit_intrinsic_correlation",
        jit_intrinsic_correlation as *const u8,
    );
    builder.symbol(
        "jit_intrinsic_covariance",
        jit_intrinsic_covariance as *const u8,
    );

    // Series operations
    builder.symbol(
        "jit_series_rolling_min",
        jit_series_rolling_min as *const u8,
    );
    builder.symbol(
        "jit_series_rolling_max",
        jit_series_rolling_max as *const u8,
    );
    builder.symbol("jit_series_ema", jit_series_ema as *const u8);
    builder.symbol("jit_series_diff", jit_series_diff as *const u8);
    builder.symbol("jit_series_pct_change", jit_series_pct_change as *const u8);
    builder.symbol("jit_series_cumprod", jit_series_cumprod as *const u8);
    builder.symbol("jit_series_clip", jit_series_clip as *const u8);
    builder.symbol("jit_series_broadcast", jit_series_broadcast as *const u8);
    builder.symbol(
        "jit_series_highest_index",
        jit_series_highest_index as *const u8,
    );
    builder.symbol(
        "jit_series_lowest_index",
        jit_series_lowest_index as *const u8,
    );
}

/// Declare math FFI function signatures in the module
pub fn declare_math_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // Math functions (single param)
    for name in [
        "jit_sin", "jit_cos", "jit_tan", "jit_asin", "jit_acos", "jit_atan", "jit_exp", "jit_ln",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_log(value_bits, base_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // value
        sig.params.push(AbiParam::new(types::I64)); // base
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_log", Linkage::Import, &sig)
            .expect("Failed to declare jit_log");
        ffi_funcs.insert("jit_log".to_string(), func_id);
    }

    // jit_pow(base_bits, exp_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // base
        sig.params.push(AbiParam::new(types::I64)); // exp
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_pow", Linkage::Import, &sig)
            .expect("Failed to declare jit_pow");
        ffi_funcs.insert("jit_pow".to_string(), func_id);
    }

    // Generic binary ops for non-numeric types (Time + Duration, Series ops, etc.)
    // jit_generic_add/sub/mul/div(a_bits, b_bits) -> u64
    for name in [
        "jit_generic_add",
        "jit_generic_sub",
        "jit_generic_mul",
        "jit_generic_div",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // a
        sig.params.push(AbiParam::new(types::I64)); // b
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // Series comparison functions (a_bits: u64, b_bits: u64) -> u64
    for name in [
        "jit_series_gt",
        "jit_series_lt",
        "jit_series_gte",
        "jit_series_lte",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // a_bits
        sig.params.push(AbiParam::new(types::I64)); // b_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // Intrinsic aggregation functions (series_bits: u64) -> u64
    for name in [
        "jit_intrinsic_sum",
        "jit_intrinsic_mean",
        "jit_intrinsic_min",
        "jit_intrinsic_max",
        "jit_intrinsic_std",
        "jit_intrinsic_variance",
        "jit_intrinsic_median",
        "jit_series_cumprod",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // Intrinsic two-arg functions (series_bits: u64, arg: u64) -> u64
    for name in [
        "jit_intrinsic_percentile",
        "jit_series_rolling_min",
        "jit_series_rolling_max",
        "jit_series_ema",
        "jit_series_diff",
        "jit_series_pct_change",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.params.push(AbiParam::new(types::I64)); // second arg
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // Two-series functions (a_bits: u64, b_bits: u64) -> u64
    for name in ["jit_intrinsic_correlation", "jit_intrinsic_covariance"] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // a_bits
        sig.params.push(AbiParam::new(types::I64)); // b_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // Clip function (series_bits: u64, min: u64, max: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.params.push(AbiParam::new(types::I64)); // min
        sig.params.push(AbiParam::new(types::I64)); // max
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_series_clip", Linkage::Import, &sig)
            .expect("Failed to declare jit_series_clip");
        ffi_funcs.insert("jit_series_clip".to_string(), func_id);
    }

    // jit_series_broadcast(value_bits: u64, len_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.params.push(AbiParam::new(types::I64)); // len_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_series_broadcast", Linkage::Import, &sig)
            .expect("Failed to declare jit_series_broadcast");
        ffi_funcs.insert("jit_series_broadcast".to_string(), func_id);
    }

    // jit_series_highest_index(series_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_series_highest_index", Linkage::Import, &sig)
            .expect("Failed to declare jit_series_highest_index");
        ffi_funcs.insert("jit_series_highest_index".to_string(), func_id);
    }

    // jit_series_lowest_index(series_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_series_lowest_index", Linkage::Import, &sig)
            .expect("Failed to declare jit_series_lowest_index");
        ffi_funcs.insert("jit_series_lowest_index".to_string(), func_id);
    }
}
