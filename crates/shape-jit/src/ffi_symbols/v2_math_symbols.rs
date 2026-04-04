//! v2 Typed Math FFI Symbol Registration
//!
//! Registers monomorphized math functions that operate on native types (f64, i64, i32)
//! directly, without NaN-boxing. Cranelift signatures use the actual native types
//! (F64, I64, I32, I8) instead of I64-as-u64 for everything.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2_math::*;

/// Register all v2 typed math FFI symbols with the JIT builder.
pub fn register_v2_math_symbols(builder: &mut JITBuilder) {
    // f64 arithmetic
    builder.symbol("jit_add_f64", jit_add_f64 as *const u8);
    builder.symbol("jit_sub_f64", jit_sub_f64 as *const u8);
    builder.symbol("jit_mul_f64", jit_mul_f64 as *const u8);
    builder.symbol("jit_div_f64", jit_div_f64 as *const u8);
    builder.symbol("jit_mod_f64", jit_mod_f64 as *const u8);
    builder.symbol("jit_neg_f64", jit_neg_f64 as *const u8);

    // f64 comparisons
    builder.symbol("jit_cmp_lt_f64", jit_cmp_lt_f64 as *const u8);
    builder.symbol("jit_cmp_le_f64", jit_cmp_le_f64 as *const u8);
    builder.symbol("jit_cmp_gt_f64", jit_cmp_gt_f64 as *const u8);
    builder.symbol("jit_cmp_ge_f64", jit_cmp_ge_f64 as *const u8);
    builder.symbol("jit_cmp_eq_f64", jit_cmp_eq_f64 as *const u8);
    builder.symbol("jit_cmp_ne_f64", jit_cmp_ne_f64 as *const u8);

    // i64 arithmetic
    builder.symbol("jit_add_i64", jit_add_i64 as *const u8);
    builder.symbol("jit_sub_i64", jit_sub_i64 as *const u8);
    builder.symbol("jit_mul_i64", jit_mul_i64 as *const u8);
    builder.symbol("jit_div_i64", jit_div_i64 as *const u8);
    builder.symbol("jit_mod_i64", jit_mod_i64 as *const u8);
    builder.symbol("jit_neg_i64", jit_neg_i64 as *const u8);

    // i64 comparisons
    builder.symbol("jit_cmp_lt_i64", jit_cmp_lt_i64 as *const u8);
    builder.symbol("jit_cmp_le_i64", jit_cmp_le_i64 as *const u8);
    builder.symbol("jit_cmp_gt_i64", jit_cmp_gt_i64 as *const u8);
    builder.symbol("jit_cmp_ge_i64", jit_cmp_ge_i64 as *const u8);
    builder.symbol("jit_cmp_eq_i64", jit_cmp_eq_i64 as *const u8);
    builder.symbol("jit_cmp_ne_i64", jit_cmp_ne_i64 as *const u8);

    // i32 arithmetic
    builder.symbol("jit_add_i32", jit_add_i32 as *const u8);
    builder.symbol("jit_sub_i32", jit_sub_i32 as *const u8);
    builder.symbol("jit_mul_i32", jit_mul_i32 as *const u8);
    builder.symbol("jit_div_i32", jit_div_i32 as *const u8);
    builder.symbol("jit_mod_i32", jit_mod_i32 as *const u8);
    builder.symbol("jit_neg_i32", jit_neg_i32 as *const u8);

    // i32 comparisons
    builder.symbol("jit_cmp_lt_i32", jit_cmp_lt_i32 as *const u8);
    builder.symbol("jit_cmp_le_i32", jit_cmp_le_i32 as *const u8);
    builder.symbol("jit_cmp_gt_i32", jit_cmp_gt_i32 as *const u8);
    builder.symbol("jit_cmp_ge_i32", jit_cmp_ge_i32 as *const u8);
    builder.symbol("jit_cmp_eq_i32", jit_cmp_eq_i32 as *const u8);
    builder.symbol("jit_cmp_ne_i32", jit_cmp_ne_i32 as *const u8);

    // f64 math functions
    builder.symbol("jit_sqrt_f64", jit_sqrt_f64 as *const u8);
    builder.symbol("jit_abs_f64", jit_abs_f64 as *const u8);
    builder.symbol("jit_floor_f64", jit_floor_f64 as *const u8);
    builder.symbol("jit_ceil_f64", jit_ceil_f64 as *const u8);
    builder.symbol("jit_round_f64", jit_round_f64 as *const u8);
    builder.symbol("jit_sin_f64", jit_sin_f64 as *const u8);
    builder.symbol("jit_cos_f64", jit_cos_f64 as *const u8);
    builder.symbol("jit_tan_f64", jit_tan_f64 as *const u8);
    builder.symbol("jit_asin_f64", jit_asin_f64 as *const u8);
    builder.symbol("jit_acos_f64", jit_acos_f64 as *const u8);
    builder.symbol("jit_atan_f64", jit_atan_f64 as *const u8);
    builder.symbol("jit_exp_f64", jit_exp_f64 as *const u8);
    builder.symbol("jit_ln_f64", jit_ln_f64 as *const u8);
    builder.symbol("jit_log_f64", jit_log_f64 as *const u8);
    builder.symbol("jit_pow_f64", jit_pow_f64 as *const u8);

    // i64 math functions
    builder.symbol("jit_abs_i64", jit_abs_i64 as *const u8);

    // Type conversions
    builder.symbol("jit_i64_to_f64", jit_i64_to_f64 as *const u8);
    builder.symbol("jit_f64_to_i64", jit_f64_to_i64 as *const u8);
    builder.symbol("jit_i32_to_f64", jit_i32_to_f64 as *const u8);
    builder.symbol("jit_f64_to_i32", jit_f64_to_i32 as *const u8);
    builder.symbol("jit_i32_to_i64", jit_i32_to_i64 as *const u8);
    builder.symbol("jit_i64_to_i32", jit_i64_to_i32 as *const u8);
}

/// Declare all v2 typed math FFI function signatures in the Cranelift module.
///
/// Unlike the v1 signatures which use I64 for everything (NaN-boxed u64),
/// these use the actual native types: F64, I64, I32, I8.
pub fn declare_v2_math_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // --- f64 binary arithmetic: (f64, f64) -> f64 ---
    for name in [
        "jit_add_f64",
        "jit_sub_f64",
        "jit_mul_f64",
        "jit_div_f64",
        "jit_mod_f64",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::F64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- f64 unary: (f64) -> f64 ---
    for name in [
        "jit_neg_f64",
        "jit_sqrt_f64",
        "jit_abs_f64",
        "jit_floor_f64",
        "jit_ceil_f64",
        "jit_round_f64",
        "jit_sin_f64",
        "jit_cos_f64",
        "jit_tan_f64",
        "jit_asin_f64",
        "jit_acos_f64",
        "jit_atan_f64",
        "jit_exp_f64",
        "jit_ln_f64",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::F64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- f64 binary math: (f64, f64) -> f64 ---
    for name in ["jit_log_f64", "jit_pow_f64"] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::F64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- f64 comparisons: (f64, f64) -> i8 ---
    for name in [
        "jit_cmp_lt_f64",
        "jit_cmp_le_f64",
        "jit_cmp_gt_f64",
        "jit_cmp_ge_f64",
        "jit_cmp_eq_f64",
        "jit_cmp_ne_f64",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I8));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- i64 binary arithmetic: (i64, i64) -> i64 ---
    for name in [
        "jit_add_i64",
        "jit_sub_i64",
        "jit_mul_i64",
        "jit_div_i64",
        "jit_mod_i64",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- i64 unary: (i64) -> i64 ---
    for name in ["jit_neg_i64", "jit_abs_i64"] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- i64 comparisons: (i64, i64) -> i8 ---
    for name in [
        "jit_cmp_lt_i64",
        "jit_cmp_le_i64",
        "jit_cmp_gt_i64",
        "jit_cmp_ge_i64",
        "jit_cmp_eq_i64",
        "jit_cmp_ne_i64",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I8));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- i32 binary arithmetic: (i32, i32) -> i32 ---
    for name in [
        "jit_add_i32",
        "jit_sub_i32",
        "jit_mul_i32",
        "jit_div_i32",
        "jit_mod_i32",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I32));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- i32 unary: (i32) -> i32 ---
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I32));
        let func_id = module
            .declare_function("jit_neg_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_neg_i32");
        ffi_funcs.insert("jit_neg_i32".to_string(), func_id);
    }

    // --- i32 comparisons: (i32, i32) -> i8 ---
    for name in [
        "jit_cmp_lt_i32",
        "jit_cmp_le_i32",
        "jit_cmp_gt_i32",
        "jit_cmp_ge_i32",
        "jit_cmp_eq_i32",
        "jit_cmp_ne_i32",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I8));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // --- Type conversions ---

    // jit_i64_to_f64: (i64) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        let func_id = module
            .declare_function("jit_i64_to_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_i64_to_f64");
        ffi_funcs.insert("jit_i64_to_f64".to_string(), func_id);
    }

    // jit_f64_to_i64: (f64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_f64_to_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_f64_to_i64");
        ffi_funcs.insert("jit_f64_to_i64".to_string(), func_id);
    }

    // jit_i32_to_f64: (i32) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::F64));
        let func_id = module
            .declare_function("jit_i32_to_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_i32_to_f64");
        ffi_funcs.insert("jit_i32_to_f64".to_string(), func_id);
    }

    // jit_f64_to_i32: (f64) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I32));
        let func_id = module
            .declare_function("jit_f64_to_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_f64_to_i32");
        ffi_funcs.insert("jit_f64_to_i32".to_string(), func_id);
    }

    // jit_i32_to_i64: (i32) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_i32_to_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_i32_to_i64");
        ffi_funcs.insert("jit_i32_to_i64".to_string(), func_id);
    }

    // jit_i64_to_i32: (i64) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func_id = module
            .declare_function("jit_i64_to_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_i64_to_i32");
        ffi_funcs.insert("jit_i64_to_i32".to_string(), func_id);
    }
}
