//! Result/Option FFI Symbol Registration
//!
//! This module handles registration and declaration of Result and Option type FFI symbols
//! for the JIT compiler.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::result::{
    jit_is_err, jit_is_none, jit_is_ok, jit_is_result, jit_is_some, jit_make_err, jit_make_ok,
    jit_make_some, jit_result_inner, jit_unwrap_err, jit_unwrap_ok, jit_unwrap_or, jit_unwrap_some,
};

/// Register Result/Option FFI symbols with the JIT builder
pub fn register_result_option_symbols(builder: &mut JITBuilder) {
    // Result type FFI functions (Ok/Err)
    builder.symbol("jit_make_ok", jit_make_ok as *const u8);
    builder.symbol("jit_make_err", jit_make_err as *const u8);
    builder.symbol("jit_is_ok", jit_is_ok as *const u8);
    builder.symbol("jit_is_err", jit_is_err as *const u8);
    builder.symbol("jit_is_result", jit_is_result as *const u8);
    builder.symbol("jit_unwrap_ok", jit_unwrap_ok as *const u8);
    builder.symbol("jit_unwrap_err", jit_unwrap_err as *const u8);
    builder.symbol("jit_unwrap_or", jit_unwrap_or as *const u8);
    builder.symbol("jit_result_inner", jit_result_inner as *const u8);

    // Option type FFI functions (Some/None)
    builder.symbol("jit_make_some", jit_make_some as *const u8);
    builder.symbol("jit_is_some", jit_is_some as *const u8);
    builder.symbol("jit_is_none", jit_is_none as *const u8);
    builder.symbol("jit_unwrap_some", jit_unwrap_some as *const u8);
}

/// Declare Result/Option FFI function signatures in the module
pub fn declare_result_option_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    // Unary result/option operations: (bits) -> u64
    for name in [
        "jit_make_ok",
        "jit_make_err",
        "jit_is_ok",
        "jit_is_err",
        "jit_is_result",
        "jit_unwrap_ok",
        "jit_unwrap_err",
        "jit_result_inner",
        "jit_make_some",
        "jit_is_some",
        "jit_is_none",
        "jit_unwrap_some",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_unwrap_or(bits, default_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // bits
        sig.params.push(AbiParam::new(types::I64)); // default_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_unwrap_or", Linkage::Import, &sig)
            .expect("Failed to declare jit_unwrap_or");
        ffi_funcs.insert("jit_unwrap_or".to_string(), func_id);
    }
}
