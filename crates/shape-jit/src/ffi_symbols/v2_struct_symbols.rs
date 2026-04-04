//! v2 Typed Struct FFI Symbol Registration
//!
//! Registers v2 struct allocation, field access, and refcounting symbols with
//! the JIT builder and declares their Cranelift signatures.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2_struct::{
    jit_v2_struct_alloc, jit_v2_struct_get_bool, jit_v2_struct_get_f64, jit_v2_struct_get_i32,
    jit_v2_struct_get_i64, jit_v2_struct_get_ptr, jit_v2_struct_refcount, jit_v2_struct_release,
    jit_v2_struct_retain, jit_v2_struct_set_bool, jit_v2_struct_set_f64, jit_v2_struct_set_i32,
    jit_v2_struct_set_i64, jit_v2_struct_set_ptr,
};

/// Register v2 struct FFI symbols with the JIT builder.
pub fn register_v2_struct_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_v2_struct_alloc", jit_v2_struct_alloc as *const u8);

    builder.symbol("jit_v2_struct_get_f64", jit_v2_struct_get_f64 as *const u8);
    builder.symbol("jit_v2_struct_set_f64", jit_v2_struct_set_f64 as *const u8);

    builder.symbol("jit_v2_struct_get_i64", jit_v2_struct_get_i64 as *const u8);
    builder.symbol("jit_v2_struct_set_i64", jit_v2_struct_set_i64 as *const u8);

    builder.symbol("jit_v2_struct_get_i32", jit_v2_struct_get_i32 as *const u8);
    builder.symbol("jit_v2_struct_set_i32", jit_v2_struct_set_i32 as *const u8);

    builder.symbol(
        "jit_v2_struct_get_bool",
        jit_v2_struct_get_bool as *const u8,
    );
    builder.symbol(
        "jit_v2_struct_set_bool",
        jit_v2_struct_set_bool as *const u8,
    );

    builder.symbol("jit_v2_struct_get_ptr", jit_v2_struct_get_ptr as *const u8);
    builder.symbol("jit_v2_struct_set_ptr", jit_v2_struct_set_ptr as *const u8);

    builder.symbol("jit_v2_struct_retain", jit_v2_struct_retain as *const u8);
    builder.symbol("jit_v2_struct_release", jit_v2_struct_release as *const u8);
    builder.symbol(
        "jit_v2_struct_refcount",
        jit_v2_struct_refcount as *const u8,
    );
}

/// Helper: declare a function and insert into the map.
macro_rules! decl {
    ($module:expr, $ffi_funcs:expr, $name:expr, $sig:expr) => {{
        let func_id = $module
            .declare_function($name, Linkage::Import, &$sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", $name));
        $ffi_funcs.insert($name.to_string(), func_id);
    }};
}

/// Declare v2 struct FFI function signatures in the Cranelift module.
pub fn declare_v2_struct_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    // jit_v2_struct_alloc(total_size: u32) -> *mut u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // total_size
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        decl!(module, ffi_funcs, "jit_v2_struct_alloc", sig);
    }

    // --- f64 get/set ---
    // jit_v2_struct_get_f64(ptr: *const u8, offset: u32) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::F64)); // value
        decl!(module, ffi_funcs, "jit_v2_struct_get_f64", sig);
    }
    // jit_v2_struct_set_f64(ptr: *mut u8, offset: u32, val: f64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.params.push(AbiParam::new(types::F64)); // val
        decl!(module, ffi_funcs, "jit_v2_struct_set_f64", sig);
    }

    // --- i64 get/set ---
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::I64)); // value
        decl!(module, ffi_funcs, "jit_v2_struct_get_i64", sig);
    }
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.params.push(AbiParam::new(types::I64)); // val
        decl!(module, ffi_funcs, "jit_v2_struct_set_i64", sig);
    }

    // --- i32 get/set ---
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::I32)); // value
        decl!(module, ffi_funcs, "jit_v2_struct_get_i32", sig);
    }
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.params.push(AbiParam::new(types::I32)); // val
        decl!(module, ffi_funcs, "jit_v2_struct_set_i32", sig);
    }

    // --- bool (u8) get/set ---
    // Note: Cranelift I8 for the value, but the ABI will pass it as an integer register.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::I8)); // value
        decl!(module, ffi_funcs, "jit_v2_struct_get_bool", sig);
    }
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.params.push(AbiParam::new(types::I8)); // val
        decl!(module, ffi_funcs, "jit_v2_struct_set_bool", sig);
    }

    // --- ptr get/set ---
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::I64)); // value (pointer as I64)
        decl!(module, ffi_funcs, "jit_v2_struct_get_ptr", sig);
    }
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.params.push(AbiParam::new(types::I64)); // val (pointer as I64)
        decl!(module, ffi_funcs, "jit_v2_struct_set_ptr", sig);
    }

    // --- refcounting ---
    // jit_v2_struct_retain(ptr: *mut u8)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        decl!(module, ffi_funcs, "jit_v2_struct_retain", sig);
    }
    // jit_v2_struct_release(ptr: *mut u8, total_size: u32)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // total_size
        decl!(module, ffi_funcs, "jit_v2_struct_release", sig);
    }
    // jit_v2_struct_refcount(ptr: *const u8) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.returns.push(AbiParam::new(types::I32)); // refcount
        decl!(module, ffi_funcs, "jit_v2_struct_refcount", sig);
    }
}
