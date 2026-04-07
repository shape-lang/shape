//! v2 typed FFI symbol registration for JIT compiler.
//!
//! Registers native-typed v2 FFI functions with the JIT builder and declares
//! their Cranelift signatures. KEY DIFFERENCE from v1: return types use native
//! Cranelift types (F64, I32) instead of everything being I64.

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2;

/// Register all v2 FFI symbols with the JIT builder.
pub fn register_v2_symbols(builder: &mut JITBuilder) {
    // Array — f64
    builder.symbol("jit_v2_array_new_f64", v2::jit_v2_array_new_f64 as *const u8);
    builder.symbol("jit_v2_array_get_f64", v2::jit_v2_array_get_f64 as *const u8);
    builder.symbol("jit_v2_array_set_f64", v2::jit_v2_array_set_f64 as *const u8);
    builder.symbol("jit_v2_array_push_f64", v2::jit_v2_array_push_f64 as *const u8);
    builder.symbol("jit_v2_array_len_f64", v2::jit_v2_array_len_f64 as *const u8);

    // Array — i64
    builder.symbol("jit_v2_array_new_i64", v2::jit_v2_array_new_i64 as *const u8);
    builder.symbol("jit_v2_array_get_i64", v2::jit_v2_array_get_i64 as *const u8);
    builder.symbol("jit_v2_array_set_i64", v2::jit_v2_array_set_i64 as *const u8);
    builder.symbol("jit_v2_array_push_i64", v2::jit_v2_array_push_i64 as *const u8);
    builder.symbol("jit_v2_array_len_i64", v2::jit_v2_array_len_i64 as *const u8);

    // Array — i32
    builder.symbol("jit_v2_array_new_i32", v2::jit_v2_array_new_i32 as *const u8);
    builder.symbol("jit_v2_array_get_i32", v2::jit_v2_array_get_i32 as *const u8);
    builder.symbol("jit_v2_array_set_i32", v2::jit_v2_array_set_i32 as *const u8);
    builder.symbol("jit_v2_array_push_i32", v2::jit_v2_array_push_i32 as *const u8);
    builder.symbol("jit_v2_array_len_i32", v2::jit_v2_array_len_i32 as *const u8);

    // Array — bool (encoded as u8 internally)
    builder.symbol("jit_v2_array_new_bool", v2::jit_v2_array_new_bool as *const u8);
    builder.symbol("jit_v2_array_get_bool", v2::jit_v2_array_get_bool as *const u8);
    builder.symbol("jit_v2_array_set_bool", v2::jit_v2_array_set_bool as *const u8);
    builder.symbol("jit_v2_array_push_bool", v2::jit_v2_array_push_bool as *const u8);
    builder.symbol("jit_v2_array_len_bool", v2::jit_v2_array_len_bool as *const u8);

    // Struct field access
    builder.symbol("jit_v2_field_load_f64", v2::jit_v2_field_load_f64 as *const u8);
    builder.symbol("jit_v2_field_load_i64", v2::jit_v2_field_load_i64 as *const u8);
    builder.symbol("jit_v2_field_load_i32", v2::jit_v2_field_load_i32 as *const u8);
    builder.symbol("jit_v2_field_load_ptr", v2::jit_v2_field_load_ptr as *const u8);
    builder.symbol("jit_v2_field_store_f64", v2::jit_v2_field_store_f64 as *const u8);
    builder.symbol("jit_v2_field_store_i64", v2::jit_v2_field_store_i64 as *const u8);
    builder.symbol("jit_v2_field_store_i32", v2::jit_v2_field_store_i32 as *const u8);
    builder.symbol("jit_v2_field_store_ptr", v2::jit_v2_field_store_ptr as *const u8);

    // Refcount
    builder.symbol("jit_v2_retain", v2::jit_v2_retain as *const u8);
    builder.symbol("jit_v2_release", v2::jit_v2_release as *const u8);

    // Struct allocation
    builder.symbol("jit_v2_alloc_struct", v2::jit_v2_alloc_struct as *const u8);
}

/// Helper: declare a function and insert into the map.
fn declare(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
    name: &str,
    sig: &Signature,
) {
    if let Ok(func_id) = module.declare_function(name, Linkage::Import, sig) {
        ffi_funcs.insert(name.to_string(), func_id);
    }
}

/// Declare all v2 FFI function signatures in the Cranelift module.
pub fn declare_v2_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // ========================================================================
    // Array — f64
    // ========================================================================

    // jit_v2_array_new_f64(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        declare(module, ffi_funcs, "jit_v2_array_new_f64", &sig);
    }

    // jit_v2_array_get_f64(arr: ptr, index: i64) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::F64)); // NATIVE F64
        declare(module, ffi_funcs, "jit_v2_array_get_f64", &sig);
    }

    // jit_v2_array_set_f64(arr: ptr, index: i64, val: f64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.params.push(AbiParam::new(types::F64)); // val NATIVE F64
        declare(module, ffi_funcs, "jit_v2_array_set_f64", &sig);
    }

    // jit_v2_array_push_f64(arr: ptr, val: f64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::F64)); // val NATIVE F64
        declare(module, ffi_funcs, "jit_v2_array_push_f64", &sig);
    }

    // jit_v2_array_len_f64(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.returns.push(AbiParam::new(types::I32)); // len
        declare(module, ffi_funcs, "jit_v2_array_len_f64", &sig);
    }

    // ========================================================================
    // Array — i64
    // ========================================================================

    // jit_v2_array_new_i64(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_new_i64", &sig);
    }

    // jit_v2_array_get_i64(arr: ptr, index: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_get_i64", &sig);
    }

    // jit_v2_array_set_i64(arr: ptr, index: i64, val: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_set_i64", &sig);
    }

    // jit_v2_array_push_i64(arr: ptr, val: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_push_i64", &sig);
    }

    // jit_v2_array_len_i64(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_array_len_i64", &sig);
    }

    // ========================================================================
    // Array — i32
    // ========================================================================

    // jit_v2_array_new_i32(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_new_i32", &sig);
    }

    // jit_v2_array_get_i32(arr: ptr, index: i64) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32)); // NATIVE I32
        declare(module, ffi_funcs, "jit_v2_array_get_i32", &sig);
    }

    // jit_v2_array_set_i32(arr: ptr, index: i64, val: i32) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32)); // val NATIVE I32
        declare(module, ffi_funcs, "jit_v2_array_set_i32", &sig);
    }

    // jit_v2_array_push_i32(arr: ptr, val: i32) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32)); // val NATIVE I32
        declare(module, ffi_funcs, "jit_v2_array_push_i32", &sig);
    }

    // jit_v2_array_len_i32(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_array_len_i32", &sig);
    }

    // ========================================================================
    // Array — bool (encoded as u8 internally)
    // ========================================================================

    // jit_v2_array_new_bool(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_new_bool", &sig);
    }

    // jit_v2_array_get_bool(arr: ptr, index: i64) -> u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I8));
        declare(module, ffi_funcs, "jit_v2_array_get_bool", &sig);
    }

    // jit_v2_array_set_bool(arr: ptr, index: i64, val: u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I8));
        declare(module, ffi_funcs, "jit_v2_array_set_bool", &sig);
    }

    // jit_v2_array_push_bool(arr: ptr, val: u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I8));
        declare(module, ffi_funcs, "jit_v2_array_push_bool", &sig);
    }

    // jit_v2_array_len_bool(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_array_len_bool", &sig);
    }

    // ========================================================================
    // Struct field access
    // ========================================================================

    // jit_v2_field_load_f64(ptr, offset: u32) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_field_load_f64", &sig);
    }

    // jit_v2_field_load_i64(ptr, offset: u32) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_load_i64", &sig);
    }

    // jit_v2_field_load_i32(ptr, offset: u32) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_field_load_i32", &sig);
    }

    // jit_v2_field_load_ptr(ptr, offset: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_load_ptr", &sig);
    }

    // jit_v2_field_store_f64(ptr, offset: u32, val: f64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_field_store_f64", &sig);
    }

    // jit_v2_field_store_i64(ptr, offset: u32, val: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_store_i64", &sig);
    }

    // jit_v2_field_store_i32(ptr, offset: u32, val: i32) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_field_store_i32", &sig);
    }

    // jit_v2_field_store_ptr(ptr, offset: u32, val: ptr) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_store_ptr", &sig);
    }

    // ========================================================================
    // Refcount
    // ========================================================================

    // jit_v2_retain(ptr) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_retain", &sig);
    }

    // jit_v2_release(ptr) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_release", &sig);
    }

    // ========================================================================
    // Struct allocation
    // ========================================================================

    // jit_v2_alloc_struct(size: u32, kind: u16) -> ptr
    // Note: u16 is promoted to i32 in C ABI
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // size
        sig.params.push(AbiParam::new(types::I32)); // kind (u16 promoted to i32)
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        declare(module, ffi_funcs, "jit_v2_alloc_struct", &sig);
    }
}
