//! SIMD FFI Symbol Registration
//!
//! This module handles registration and declaration of SIMD operation FFI symbols
//! for the JIT compiler, including both raw pointer SIMD operations and boxed vector intrinsics.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::simd::{
    jit_simd_add, jit_simd_add_scalar, jit_simd_div, jit_simd_div_scalar, jit_simd_eq,
    jit_simd_free, jit_simd_gt, jit_simd_gte, jit_simd_lt, jit_simd_lte, jit_simd_max,
    jit_simd_min, jit_simd_mul, jit_simd_mul_scalar, jit_simd_neq, jit_simd_sub,
    jit_simd_sub_scalar,
};
use super::vector::{
    jit_intrinsic_matmul_mat, jit_intrinsic_matmul_vec, jit_intrinsic_vec_abs,
    jit_intrinsic_vec_add, jit_intrinsic_vec_div, jit_intrinsic_vec_exp, jit_intrinsic_vec_ln,
    jit_intrinsic_vec_max, jit_intrinsic_vec_min, jit_intrinsic_vec_mul, jit_intrinsic_vec_sqrt,
    jit_intrinsic_vec_sub,
};

/// Register SIMD FFI symbols with the JIT builder
pub fn register_simd_symbols(builder: &mut JITBuilder) {
    // Vector intrinsics (boxed interface - legacy)
    builder.symbol("jit_intrinsic_vec_abs", jit_intrinsic_vec_abs as *const u8);
    builder.symbol(
        "jit_intrinsic_vec_sqrt",
        jit_intrinsic_vec_sqrt as *const u8,
    );
    builder.symbol("jit_intrinsic_vec_ln", jit_intrinsic_vec_ln as *const u8);
    builder.symbol("jit_intrinsic_vec_exp", jit_intrinsic_vec_exp as *const u8);
    builder.symbol("jit_intrinsic_vec_add", jit_intrinsic_vec_add as *const u8);
    builder.symbol("jit_intrinsic_vec_sub", jit_intrinsic_vec_sub as *const u8);
    builder.symbol("jit_intrinsic_vec_mul", jit_intrinsic_vec_mul as *const u8);
    builder.symbol("jit_intrinsic_vec_div", jit_intrinsic_vec_div as *const u8);
    builder.symbol("jit_intrinsic_vec_max", jit_intrinsic_vec_max as *const u8);
    builder.symbol("jit_intrinsic_vec_min", jit_intrinsic_vec_min as *const u8);
    builder.symbol(
        "jit_intrinsic_matmul_vec",
        jit_intrinsic_matmul_vec as *const u8,
    );
    builder.symbol(
        "jit_intrinsic_matmul_mat",
        jit_intrinsic_matmul_mat as *const u8,
    );

    // Raw pointer SIMD operations (zero-copy, high performance)
    // Binary: simd_op(ptr_a: *const f64, ptr_b: *const f64, len: u64) -> *mut f64
    builder.symbol("jit_simd_add", jit_simd_add as *const u8);
    builder.symbol("jit_simd_sub", jit_simd_sub as *const u8);
    builder.symbol("jit_simd_mul", jit_simd_mul as *const u8);
    builder.symbol("jit_simd_div", jit_simd_div as *const u8);
    builder.symbol("jit_simd_max", jit_simd_max as *const u8);
    builder.symbol("jit_simd_min", jit_simd_min as *const u8);

    // Scalar broadcast: simd_op_scalar(ptr: *const f64, scalar: f64, len: u64) -> *mut f64
    builder.symbol("jit_simd_add_scalar", jit_simd_add_scalar as *const u8);
    builder.symbol("jit_simd_sub_scalar", jit_simd_sub_scalar as *const u8);
    builder.symbol("jit_simd_mul_scalar", jit_simd_mul_scalar as *const u8);
    builder.symbol("jit_simd_div_scalar", jit_simd_div_scalar as *const u8);

    // Comparison: simd_cmp(ptr_a, ptr_b, len) -> *mut f64 (1.0/0.0 mask)
    builder.symbol("jit_simd_gt", jit_simd_gt as *const u8);
    builder.symbol("jit_simd_lt", jit_simd_lt as *const u8);
    builder.symbol("jit_simd_gte", jit_simd_gte as *const u8);
    builder.symbol("jit_simd_lte", jit_simd_lte as *const u8);
    builder.symbol("jit_simd_eq", jit_simd_eq as *const u8);
    builder.symbol("jit_simd_neq", jit_simd_neq as *const u8);
    builder.symbol("jit_simd_free", jit_simd_free as *const u8);
}

/// Declare SIMD FFI function signatures in the module
pub fn declare_simd_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // SIMD Binary Operations (ptr, ptr, len) -> ptr
    for name in [
        "jit_simd_add",
        "jit_simd_sub",
        "jit_simd_mul",
        "jit_simd_div",
        "jit_simd_max",
        "jit_simd_min",
        "jit_simd_gt",
        "jit_simd_lt",
        "jit_simd_gte",
        "jit_simd_lte",
        "jit_simd_eq",
        "jit_simd_neq",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // a_ptr
        sig.params.push(AbiParam::new(types::I64)); // b_ptr
        sig.params.push(AbiParam::new(types::I64)); // len
        sig.returns.push(AbiParam::new(types::I64)); // result_ptr
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // SIMD Scalar Operations (ptr, scalar: F64, len) -> ptr
    for name in [
        "jit_simd_add_scalar",
        "jit_simd_sub_scalar",
        "jit_simd_mul_scalar",
        "jit_simd_div_scalar",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::F64)); // scalar (NOTE: F64!)
        sig.params.push(AbiParam::new(types::I64)); // len
        sig.returns.push(AbiParam::new(types::I64)); // result_ptr
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_simd_free(ptr, len) - no return
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I64)); // len
        // No return value
        let func_id = module
            .declare_function("jit_simd_free", Linkage::Import, &sig)
            .expect("Failed to declare jit_simd_free");
        ffi_funcs.insert("jit_simd_free".to_string(), func_id);
    }

    // jit_intrinsic_vec_* (ctx, arg) -> u64
    for name in [
        "jit_intrinsic_vec_abs",
        "jit_intrinsic_vec_sqrt",
        "jit_intrinsic_vec_ln",
        "jit_intrinsic_vec_exp",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // arg
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_intrinsic_vec_* (ctx, a, b) -> u64
    for name in [
        "jit_intrinsic_vec_add",
        "jit_intrinsic_vec_sub",
        "jit_intrinsic_vec_mul",
        "jit_intrinsic_vec_div",
        "jit_intrinsic_vec_max",
        "jit_intrinsic_vec_min",
        "jit_intrinsic_matmul_vec",
        "jit_intrinsic_matmul_mat",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // a
        sig.params.push(AbiParam::new(types::I64)); // b
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }
}
