//! v2 String FFI Symbol Registration
//!
//! Registers the v2 string operation symbols with the JIT builder and declares
//! their Cranelift function signatures using native types (I64 for pointers,
//! I32 for lengths, I8 for boolean returns).

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2_string_ffi::{
    jit_v2_string_alloc, jit_v2_string_concat, jit_v2_string_data, jit_v2_string_eq,
    jit_v2_string_len, jit_v2_string_print, jit_v2_string_release, jit_v2_string_retain,
};

/// Register v2 string FFI symbols with the JIT builder.
pub fn register_v2_string_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_v2_string_alloc", jit_v2_string_alloc as *const u8);
    builder.symbol("jit_v2_string_len", jit_v2_string_len as *const u8);
    builder.symbol("jit_v2_string_data", jit_v2_string_data as *const u8);
    builder.symbol("jit_v2_string_concat", jit_v2_string_concat as *const u8);
    builder.symbol("jit_v2_string_eq", jit_v2_string_eq as *const u8);
    builder.symbol("jit_v2_string_print", jit_v2_string_print as *const u8);
    builder.symbol(
        "jit_v2_string_retain",
        jit_v2_string_retain as *const u8,
    );
    builder.symbol(
        "jit_v2_string_release",
        jit_v2_string_release as *const u8,
    );
}

/// Declare v2 string FFI function signatures in the Cranelift module.
pub fn declare_v2_string_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    // jit_v2_string_alloc(data: *const u8, len: u32) -> *mut u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // data pointer
        sig.params.push(AbiParam::new(types::I32)); // len
        sig.returns.push(AbiParam::new(types::I64)); // StringObj pointer
        let func_id = module
            .declare_function("jit_v2_string_alloc", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_alloc");
        ffi_funcs.insert("jit_v2_string_alloc".to_string(), func_id);
    }

    // jit_v2_string_len(str_ptr: *mut u8) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // str_ptr
        sig.returns.push(AbiParam::new(types::I64)); // length
        let func_id = module
            .declare_function("jit_v2_string_len", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_len");
        ffi_funcs.insert("jit_v2_string_len".to_string(), func_id);
    }

    // jit_v2_string_data(str_ptr: *mut u8) -> *const u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // str_ptr
        sig.returns.push(AbiParam::new(types::I64)); // data pointer
        let func_id = module
            .declare_function("jit_v2_string_data", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_data");
        ffi_funcs.insert("jit_v2_string_data".to_string(), func_id);
    }

    // jit_v2_string_concat(a: *mut u8, b: *mut u8) -> *mut u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // a
        sig.params.push(AbiParam::new(types::I64)); // b
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_v2_string_concat", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_concat");
        ffi_funcs.insert("jit_v2_string_concat".to_string(), func_id);
    }

    // jit_v2_string_eq(a: *mut u8, b: *mut u8) -> u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // a
        sig.params.push(AbiParam::new(types::I64)); // b
        sig.returns.push(AbiParam::new(types::I8)); // 1 or 0
        let func_id = module
            .declare_function("jit_v2_string_eq", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_eq");
        ffi_funcs.insert("jit_v2_string_eq".to_string(), func_id);
    }

    // jit_v2_string_print(str_ptr: *mut u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // str_ptr
        let func_id = module
            .declare_function("jit_v2_string_print", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_print");
        ffi_funcs.insert("jit_v2_string_print".to_string(), func_id);
    }

    // jit_v2_string_retain(str_ptr: *mut u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // str_ptr
        let func_id = module
            .declare_function("jit_v2_string_retain", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_retain");
        ffi_funcs.insert("jit_v2_string_retain".to_string(), func_id);
    }

    // jit_v2_string_release(str_ptr: *mut u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // str_ptr
        let func_id = module
            .declare_function("jit_v2_string_release", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_string_release");
        ffi_funcs.insert("jit_v2_string_release".to_string(), func_id);
    }
}
