//! v2 Typed Array FFI Symbol Registration
//!
//! Registers `jit_v2_array_*` function pointers with the JIT builder and
//! declares their Cranelift signatures using **native types** (F64, I64, I32)
//! instead of the universal I64-as-NaN-boxed convention of the v1 array FFI.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2_array::{
    jit_v2_array_alloc_f64, jit_v2_array_alloc_i32, jit_v2_array_alloc_i64,
    jit_v2_array_get_f64, jit_v2_array_get_i32, jit_v2_array_get_i64,
    jit_v2_array_len, jit_v2_array_push_f64, jit_v2_array_push_i32, jit_v2_array_push_i64,
    jit_v2_array_release, jit_v2_array_retain, jit_v2_array_set_f64, jit_v2_array_set_i32,
    jit_v2_array_set_i64,
};

/// Register v2 typed array FFI symbols with the JIT builder.
pub fn register_v2_array_symbols(builder: &mut JITBuilder) {
    // f64
    builder.symbol("jit_v2_array_alloc_f64", jit_v2_array_alloc_f64 as *const u8);
    builder.symbol("jit_v2_array_get_f64", jit_v2_array_get_f64 as *const u8);
    builder.symbol("jit_v2_array_set_f64", jit_v2_array_set_f64 as *const u8);
    builder.symbol("jit_v2_array_push_f64", jit_v2_array_push_f64 as *const u8);

    // i64
    builder.symbol("jit_v2_array_alloc_i64", jit_v2_array_alloc_i64 as *const u8);
    builder.symbol("jit_v2_array_get_i64", jit_v2_array_get_i64 as *const u8);
    builder.symbol("jit_v2_array_set_i64", jit_v2_array_set_i64 as *const u8);
    builder.symbol("jit_v2_array_push_i64", jit_v2_array_push_i64 as *const u8);

    // i32
    builder.symbol("jit_v2_array_alloc_i32", jit_v2_array_alloc_i32 as *const u8);
    builder.symbol("jit_v2_array_get_i32", jit_v2_array_get_i32 as *const u8);
    builder.symbol("jit_v2_array_set_i32", jit_v2_array_set_i32 as *const u8);
    builder.symbol("jit_v2_array_push_i32", jit_v2_array_push_i32 as *const u8);

    // type-agnostic
    builder.symbol("jit_v2_array_len", jit_v2_array_len as *const u8);
    builder.symbol("jit_v2_array_retain", jit_v2_array_retain as *const u8);
    builder.symbol("jit_v2_array_release", jit_v2_array_release as *const u8);
}

/// Declare v2 typed array FFI function signatures in the Cranelift module.
///
/// Key difference from v1: parameters and returns use native Cranelift types
/// (`F64`, `I32`) where the element type is known, rather than everything
/// being `I64`.
pub fn declare_v2_array_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    // -----------------------------------------------------------------------
    // f64 variants
    // -----------------------------------------------------------------------

    // jit_v2_array_alloc_f64(cap: I32) -> I64 (ptr)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // cap
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        let func_id = module
            .declare_function("jit_v2_array_alloc_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_alloc_f64");
        ffi_funcs.insert("jit_v2_array_alloc_f64".to_string(), func_id);
    }

    // jit_v2_array_get_f64(arr: I64, index: I64) -> F64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::F64)); // value
        let func_id = module
            .declare_function("jit_v2_array_get_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_get_f64");
        ffi_funcs.insert("jit_v2_array_get_f64".to_string(), func_id);
    }

    // jit_v2_array_set_f64(arr: I64, index: I64, val: F64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.params.push(AbiParam::new(types::F64)); // val
        // no return
        let func_id = module
            .declare_function("jit_v2_array_set_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_set_f64");
        ffi_funcs.insert("jit_v2_array_set_f64".to_string(), func_id);
    }

    // jit_v2_array_push_f64(arr: I64, val: F64) -> I64 (new ptr)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::F64)); // val
        sig.returns.push(AbiParam::new(types::I64)); // new ptr
        let func_id = module
            .declare_function("jit_v2_array_push_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_push_f64");
        ffi_funcs.insert("jit_v2_array_push_f64".to_string(), func_id);
    }

    // -----------------------------------------------------------------------
    // i64 variants
    // -----------------------------------------------------------------------

    // jit_v2_array_alloc_i64(cap: I32) -> I64 (ptr)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // cap
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        let func_id = module
            .declare_function("jit_v2_array_alloc_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_alloc_i64");
        ffi_funcs.insert("jit_v2_array_alloc_i64".to_string(), func_id);
    }

    // jit_v2_array_get_i64(arr: I64, index: I64) -> I64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::I64)); // value
        let func_id = module
            .declare_function("jit_v2_array_get_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_get_i64");
        ffi_funcs.insert("jit_v2_array_get_i64".to_string(), func_id);
    }

    // jit_v2_array_set_i64(arr: I64, index: I64, val: I64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.params.push(AbiParam::new(types::I64)); // val
        // no return
        let func_id = module
            .declare_function("jit_v2_array_set_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_set_i64");
        ffi_funcs.insert("jit_v2_array_set_i64".to_string(), func_id);
    }

    // jit_v2_array_push_i64(arr: I64, val: I64) -> I64 (new ptr)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // val
        sig.returns.push(AbiParam::new(types::I64)); // new ptr
        let func_id = module
            .declare_function("jit_v2_array_push_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_push_i64");
        ffi_funcs.insert("jit_v2_array_push_i64".to_string(), func_id);
    }

    // -----------------------------------------------------------------------
    // i32 variants
    // -----------------------------------------------------------------------

    // jit_v2_array_alloc_i32(cap: I32) -> I64 (ptr)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // cap
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        let func_id = module
            .declare_function("jit_v2_array_alloc_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_alloc_i32");
        ffi_funcs.insert("jit_v2_array_alloc_i32".to_string(), func_id);
    }

    // jit_v2_array_get_i32(arr: I64, index: I64) -> I32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::I32)); // value
        let func_id = module
            .declare_function("jit_v2_array_get_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_get_i32");
        ffi_funcs.insert("jit_v2_array_get_i32".to_string(), func_id);
    }

    // jit_v2_array_set_i32(arr: I64, index: I64, val: I32)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.params.push(AbiParam::new(types::I32)); // val
        // no return
        let func_id = module
            .declare_function("jit_v2_array_set_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_set_i32");
        ffi_funcs.insert("jit_v2_array_set_i32".to_string(), func_id);
    }

    // jit_v2_array_push_i32(arr: I64, val: I32) -> I64 (new ptr)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I32)); // val
        sig.returns.push(AbiParam::new(types::I64)); // new ptr
        let func_id = module
            .declare_function("jit_v2_array_push_i32", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_push_i32");
        ffi_funcs.insert("jit_v2_array_push_i32".to_string(), func_id);
    }

    // -----------------------------------------------------------------------
    // Type-agnostic operations
    // -----------------------------------------------------------------------

    // jit_v2_array_len(arr: I64) -> I64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.returns.push(AbiParam::new(types::I64)); // len
        let func_id = module
            .declare_function("jit_v2_array_len", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_len");
        ffi_funcs.insert("jit_v2_array_len".to_string(), func_id);
    }

    // jit_v2_array_retain(arr: I64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        // no return
        let func_id = module
            .declare_function("jit_v2_array_retain", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_retain");
        ffi_funcs.insert("jit_v2_array_retain".to_string(), func_id);
    }

    // jit_v2_array_release(arr: I64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        // no return
        let func_id = module
            .declare_function("jit_v2_array_release", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_array_release");
        ffi_funcs.insert("jit_v2_array_release".to_string(), func_id);
    }
}
