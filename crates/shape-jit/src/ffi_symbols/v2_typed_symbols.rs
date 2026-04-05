//! v2 Typed FFI Symbol Registration
//!
//! Registers the v2 native-typed FFI function symbols with the JIT builder
//! and declares their Cranelift signatures. These functions accept native
//! types (f64, i64, etc.) instead of NaN-boxed u64.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2_typed::{
    jit_v2_array_alloc_f64, jit_v2_array_get_f64, jit_v2_array_push_f64, jit_v2_pow_f64,
    jit_v2_print_typed, jit_v2_release, jit_v2_retain, jit_v2_struct_alloc,
    jit_v2_struct_get_f64, jit_v2_struct_set_f64,
};

/// Register v2 typed FFI symbols with the JIT builder
pub fn register_v2_typed_symbols(builder: &mut JITBuilder) {
    // Array operations
    builder.symbol("jit_v2_array_alloc_f64", jit_v2_array_alloc_f64 as *const u8);
    builder.symbol("jit_v2_array_push_f64", jit_v2_array_push_f64 as *const u8);
    builder.symbol("jit_v2_array_get_f64", jit_v2_array_get_f64 as *const u8);

    // Math operations
    builder.symbol("jit_v2_pow_f64", jit_v2_pow_f64 as *const u8);

    // Struct operations
    builder.symbol("jit_v2_struct_alloc", jit_v2_struct_alloc as *const u8);
    builder.symbol("jit_v2_struct_get_f64", jit_v2_struct_get_f64 as *const u8);
    builder.symbol("jit_v2_struct_set_f64", jit_v2_struct_set_f64 as *const u8);

    // Refcount operations
    builder.symbol("jit_v2_retain", jit_v2_retain as *const u8);
    builder.symbol("jit_v2_release", jit_v2_release as *const u8);

    // Print
    builder.symbol("jit_v2_print_typed", jit_v2_print_typed as *const u8);
}

/// Declare v2 typed FFI function signatures in the module
pub fn declare_v2_typed_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_v2_array_alloc_f64(capacity: I64) -> I64 (NaN-boxed array)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // NaN-boxed array
        let func_id = module
            .declare_function("jit_v2_array_alloc_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_alloc_f64");
        ffi_funcs.insert("jit_v2_array_alloc_f64".to_string(), func_id);
    }

    // jit_v2_array_push_f64(arr_bits: I64, val: F64) -> I64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::F64)); // val (raw f64)
        sig.returns.push(AbiParam::new(types::I64)); // arr_bits (unchanged)
        let func_id = module
            .declare_function("jit_v2_array_push_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_push_f64");
        ffi_funcs.insert("jit_v2_array_push_f64".to_string(), func_id);
    }

    // jit_v2_array_get_f64(arr_bits: I64, idx: I64) -> F64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::I64)); // idx
        sig.returns.push(AbiParam::new(types::F64)); // raw f64 value
        let func_id = module
            .declare_function("jit_v2_array_get_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_get_f64");
        ffi_funcs.insert("jit_v2_array_get_f64".to_string(), func_id);
    }

    // jit_v2_pow_f64(base: F64, exp: F64) -> F64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64)); // base
        sig.params.push(AbiParam::new(types::F64)); // exp
        sig.returns.push(AbiParam::new(types::F64)); // result
        let func_id = module
            .declare_function("jit_v2_pow_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_pow_f64");
        ffi_funcs.insert("jit_v2_pow_f64".to_string(), func_id);
    }

    // jit_v2_struct_alloc(schema_id: I32, total_size: I64) -> I64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // schema_id
        sig.params.push(AbiParam::new(types::I64)); // total_size
        sig.returns.push(AbiParam::new(types::I64)); // NaN-boxed typed object
        let func_id = module
            .declare_function("jit_v2_struct_alloc", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_struct_alloc");
        ffi_funcs.insert("jit_v2_struct_alloc".to_string(), func_id);
    }

    // jit_v2_struct_get_f64(ptr_bits: I64, offset: I32) -> F64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr_bits
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::F64)); // raw f64 value
        let func_id = module
            .declare_function("jit_v2_struct_get_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_struct_get_f64");
        ffi_funcs.insert("jit_v2_struct_get_f64".to_string(), func_id);
    }

    // jit_v2_struct_set_f64(ptr_bits: I64, offset: I32, val: F64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr_bits
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.params.push(AbiParam::new(types::F64)); // val (raw f64)
        let func_id = module
            .declare_function("jit_v2_struct_set_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_struct_set_f64");
        ffi_funcs.insert("jit_v2_struct_set_f64".to_string(), func_id);
    }

    // jit_v2_retain(ptr_bits: I64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr_bits
        let func_id = module
            .declare_function("jit_v2_retain", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_retain");
        ffi_funcs.insert("jit_v2_retain".to_string(), func_id);
    }

    // jit_v2_release(ptr_bits: I64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr_bits
        let func_id = module
            .declare_function("jit_v2_release", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_release");
        ffi_funcs.insert("jit_v2_release".to_string(), func_id);
    }

    // jit_v2_print_typed(value_bits: I64, type_tag: I8)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.params.push(AbiParam::new(types::I8));  // type_tag
        let func_id = module
            .declare_function("jit_v2_print_typed", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_print_typed");
        ffi_funcs.insert("jit_v2_print_typed".to_string(), func_id);
    }
}
