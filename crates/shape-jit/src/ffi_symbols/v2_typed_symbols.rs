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
    jit_v2_array_alloc_f64, jit_v2_pow_f64, jit_v2_print_typed, jit_v2_struct_alloc,
    jit_v2_struct_get_f64, jit_v2_struct_set_f64,
};

/// Register v2 typed FFI symbols with the JIT builder.
///
/// Note: `jit_v2_array_get_f64`, `jit_v2_array_push_f64`, `jit_v2_retain`, and
/// `jit_v2_release` symbol names are owned by the typed v2 module
/// (`super::v2_symbols`) which uses native `*mut TypedArray<f64>` pointers and
/// raw f64 values instead of NaN-boxed u64. The legacy entries that used to
/// live here have been removed to avoid duplicate-declaration errors at
/// Cranelift module declaration time.
pub fn register_v2_typed_symbols(builder: &mut JITBuilder) {
    // Array operations (legacy NaN-boxed allocator — `jit_v2_array_alloc_f64`
    // is a *different* symbol name from the typed-path `jit_v2_array_new_f64`)
    builder.symbol("jit_v2_array_alloc_f64", jit_v2_array_alloc_f64 as *const u8);

    // Math operations
    builder.symbol("jit_v2_pow_f64", jit_v2_pow_f64 as *const u8);

    // Struct operations
    builder.symbol("jit_v2_struct_alloc", jit_v2_struct_alloc as *const u8);
    builder.symbol("jit_v2_struct_get_f64", jit_v2_struct_get_f64 as *const u8);
    builder.symbol("jit_v2_struct_set_f64", jit_v2_struct_set_f64 as *const u8);

    // Print
    builder.symbol("jit_v2_print_typed", jit_v2_print_typed as *const u8);
}

/// Declare v2 typed FFI function signatures in the module.
///
/// Note: `jit_v2_array_get_f64`, `jit_v2_array_push_f64`, `jit_v2_retain`, and
/// `jit_v2_release` are declared by `super::v2_symbols::declare_v2_functions`
/// instead — those names belong to the typed v2 module with native pointer
/// signatures, not the legacy NaN-boxed bodies that used to live here.
pub fn declare_v2_typed_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_v2_array_alloc_f64(capacity: I64) -> I64 (legacy NaN-boxed array — separate
    // symbol name from the typed-path `jit_v2_array_new_f64`)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // NaN-boxed array
        let func_id = module
            .declare_function("jit_v2_array_alloc_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_alloc_f64");
        ffi_funcs.insert("jit_v2_array_alloc_f64".to_string(), func_id);
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

    // jit_v2_retain and jit_v2_release are declared by `super::v2_symbols`
    // (typed v2 module path) and intentionally NOT re-declared here.

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
