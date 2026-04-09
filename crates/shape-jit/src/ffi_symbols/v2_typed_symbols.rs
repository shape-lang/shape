//! v2 Typed FFI Symbol Registration (legacy transitional)
//!
//! Registers a small set of v2 FFI function symbols that haven't yet been
//! migrated to their proper v2 sub-modules. As the v2 migration completes,
//! entries here should move to v2_struct_symbols, v2_math_symbols, etc.
//!
//! **Struct symbols** (`jit_v2_struct_alloc`, `jit_v2_struct_get_f64`,
//! `jit_v2_struct_set_f64`) were removed from here — they are now registered
//! by `v2_struct_symbols` which provides proper raw-pointer implementations
//! without NaN-boxing overhead.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2_typed::{
    jit_v2_array_alloc_f64, jit_v2_pow_f64, jit_v2_print_typed,
};

/// Register v2 typed FFI symbols with the JIT builder.
///
/// Note: `jit_v2_array_get_f64`, `jit_v2_array_push_f64`, `jit_v2_retain`, and
/// `jit_v2_release` symbol names are owned by the typed v2 module
/// (`super::v2_symbols`) which uses native `*mut TypedArray<f64>` pointers and
/// raw f64 values instead of NaN-boxed u64. The legacy entries that used to
/// live here have been removed to avoid duplicate-declaration errors at
/// Cranelift module declaration time.
///
/// Struct symbols (`jit_v2_struct_alloc`, `jit_v2_struct_get_f64`,
/// `jit_v2_struct_set_f64`) are now registered by `v2_struct_symbols`.
pub fn register_v2_typed_symbols(builder: &mut JITBuilder) {
    // Array operations (legacy NaN-boxed allocator — `jit_v2_array_alloc_f64`
    // is a *different* symbol name from the typed-path `jit_v2_array_new_f64`)
    builder.symbol("jit_v2_array_alloc_f64", jit_v2_array_alloc_f64 as *const u8);

    // Math operations
    builder.symbol("jit_v2_pow_f64", jit_v2_pow_f64 as *const u8);

    // Print
    builder.symbol("jit_v2_print_typed", jit_v2_print_typed as *const u8);
}

/// Declare v2 typed FFI function signatures in the module.
///
/// Note: Struct signatures (`jit_v2_struct_alloc`, `jit_v2_struct_get_f64`,
/// `jit_v2_struct_set_f64`) are now declared by `v2_struct_symbols`.
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
