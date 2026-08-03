//! Control Flow FFI Symbol Registration
//!
//! This module handles registration and declaration of control flow FFI symbols
//! for the JIT compiler, including function calls, iterations, and array operations.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::call_method::jit_call_method;
use super::super::ffi::control::{
    jit_call_foreign, jit_call_foreign_dynamic, jit_call_foreign_native, jit_call_foreign_native_0,
    jit_call_foreign_native_1, jit_call_foreign_native_2, jit_call_foreign_native_3,
    jit_call_foreign_native_4, jit_call_foreign_native_5, jit_call_foreign_native_6,
    jit_call_foreign_native_7, jit_call_foreign_native_8, jit_call_function, jit_call_value_f64,
    jit_call_value_i64, jit_call_value_ptr, jit_call_value_void, jit_control_every,
    jit_control_filter, jit_control_find, jit_control_find_index, jit_control_fold,
    jit_control_foreach, jit_control_map, jit_control_reduce, jit_control_some,
};
use super::super::ffi::iterator::{jit_iter_done, jit_iter_next};

/// Register control flow FFI symbols with the JIT builder
pub fn register_control_symbols(builder: &mut JITBuilder) {
    // #117 / R15 native-entry callback. Registered unconditionally so a witness
    // session can be activated at any point without a second JIT build shape;
    // the call is only EMITTED while a session collects.
    builder.symbol(
        "jit_witness_native_entry",
        super::super::ffi::witness::jit_witness_native_entry as *const u8,
    );
    builder.symbol("jit_call_function", jit_call_function as *const u8);
    // ADR-020 / #239 §4.1: one entry point per Cranelift return ABI class.
    // The kind-blind `jit_call_value` is gone — it could not say which
    // representation the destination wanted, nor tell "unit callee produced
    // nothing" from "callee failed to produce anything" (#259).
    builder.symbol("jit_call_value_i64", jit_call_value_i64 as *const u8);
    builder.symbol("jit_call_value_f64", jit_call_value_f64 as *const u8);
    builder.symbol("jit_call_value_ptr", jit_call_value_ptr as *const u8);
    builder.symbol("jit_call_value_void", jit_call_value_void as *const u8);
    builder.symbol("jit_call_foreign", jit_call_foreign as *const u8);
    builder.symbol(
        "jit_call_foreign_native",
        jit_call_foreign_native as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_dynamic",
        jit_call_foreign_dynamic as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_0",
        jit_call_foreign_native_0 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_1",
        jit_call_foreign_native_1 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_2",
        jit_call_foreign_native_2 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_3",
        jit_call_foreign_native_3 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_4",
        jit_call_foreign_native_4 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_5",
        jit_call_foreign_native_5 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_6",
        jit_call_foreign_native_6 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_7",
        jit_call_foreign_native_7 as *const u8,
    );
    builder.symbol(
        "jit_call_foreign_native_8",
        jit_call_foreign_native_8 as *const u8,
    );
    builder.symbol("jit_iter_next", jit_iter_next as *const u8);
    builder.symbol("jit_iter_done", jit_iter_done as *const u8);
    builder.symbol("jit_call_method", jit_call_method as *const u8);
    builder.symbol("jit_control_fold", jit_control_fold as *const u8);
    builder.symbol("jit_control_reduce", jit_control_reduce as *const u8);
    builder.symbol("jit_control_map", jit_control_map as *const u8);
    builder.symbol("jit_control_filter", jit_control_filter as *const u8);
    builder.symbol("jit_control_foreach", jit_control_foreach as *const u8);
    builder.symbol("jit_control_find", jit_control_find as *const u8);
    builder.symbol(
        "jit_control_find_index",
        jit_control_find_index as *const u8,
    );
    builder.symbol("jit_control_some", jit_control_some as *const u8);
    builder.symbol("jit_control_every", jit_control_every as *const u8);
}

/// Declare control flow FFI function signatures in the module
pub fn declare_control_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_witness_native_entry(unit_index: i64) -> void
    // #117 / R15: emitted as the first instruction of an instrumented function
    // entry block so the witness observes native dispatch rather than inferring
    // it from installation.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // unit_index
        let func_id = module
            .declare_function("jit_witness_native_entry", Linkage::Import, &sig)
            .expect("Failed to declare jit_witness_native_entry");
        ffi_funcs.insert("jit_witness_native_entry".to_string(), func_id);
    }

    // jit_call_function(ctx: *mut JITContext, function_id: u16, args: *const u64, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I16)); // function_id (u16)
        sig.params.push(AbiParam::new(types::I64)); // args (deprecated, pass null)
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_call_function", Linkage::Import, &sig)
            .expect("Failed to declare jit_call_function");
        ffi_funcs.insert("jit_call_function".to_string(), func_id);
    }

    // ADR-020 / #239 §4.1 — the monomorphized value-call return channel.
    // Each reads callee and args from ctx.stack identically; they differ only
    // in the Cranelift type the result comes back in, which is selected at the
    // emit site from the destination slot's PROVEN kind.
    //
    // `_i64` and `_ptr` are the same Cranelift signature deliberately (§4.2):
    // they cost nothing to keep apart at the ABI, and at the Rust level they
    // are the only remaining carrier of "this return transfers an owned share"
    // vs "this return is a number".
    for (name, ret) in [
        ("jit_call_value_i64", Some(types::I64)),
        ("jit_call_value_ptr", Some(types::I64)),
        ("jit_call_value_f64", Some(types::F64)),
        ("jit_call_value_void", None),
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        if let Some(ret) = ret {
            sig.returns.push(AbiParam::new(ret));
        }
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("Failed to declare {name}: {e}"));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_call_foreign(ctx: *mut JITContext, foreign_idx: u32, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // foreign_idx
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_call_foreign", Linkage::Import, &sig)
            .expect("Failed to declare jit_call_foreign");
        ffi_funcs.insert("jit_call_foreign".to_string(), func_id);
    }

    // jit_call_foreign_native(ctx: *mut JITContext, foreign_idx: u32, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // foreign_idx
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_call_foreign_native", Linkage::Import, &sig)
            .expect("Failed to declare jit_call_foreign_native");
        ffi_funcs.insert("jit_call_foreign_native".to_string(), func_id);
    }

    // jit_call_foreign_dynamic(ctx: *mut JITContext, foreign_idx: u32, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // foreign_idx
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_call_foreign_dynamic", Linkage::Import, &sig)
            .expect("Failed to declare jit_call_foreign_dynamic");
        ffi_funcs.insert("jit_call_foreign_dynamic".to_string(), func_id);
    }

    // Arity-specialized native calls:
    // jit_call_foreign_native_N(ctx: *mut JITContext, foreign_idx: u32, arg0: u64, ... argN-1: u64) -> u64
    for arity in 0..=8usize {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // foreign_idx
        for _ in 0..arity {
            sig.params.push(AbiParam::new(types::I64)); // arg_i (NaN-boxed)
        }
        sig.returns.push(AbiParam::new(types::I64)); // result
        let name = format!("jit_call_foreign_native_{arity}");
        let func_id = module
            .declare_function(&name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name, func_id);
    }

    // jit_iter_next(arr_bits: u64, index: i64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_iter_next", Linkage::Import, &sig)
            .expect("Failed to declare jit_iter_next");
        ffi_funcs.insert("jit_iter_next".to_string(), func_id);
    }

    // jit_iter_done(arr_bits: u64, index: i64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_iter_done", Linkage::Import, &sig)
            .expect("Failed to declare jit_iter_done");
        ffi_funcs.insert("jit_iter_done".to_string(), func_id);
    }

    // jit_call_method(ctx: *mut JITContext, stack_count: usize) -> u64
    // Reads receiver, method_name, and args from ctx.stack
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // stack_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_call_method", Linkage::Import, &sig)
            .expect("Failed to declare jit_call_method");
        ffi_funcs.insert("jit_call_method".to_string(), func_id);
    }

    // Control flow functions (ctx) -> u64
    // jit_control_fold, jit_control_reduce, jit_control_map, jit_control_filter, etc.
    for name in [
        "jit_control_fold",
        "jit_control_reduce",
        "jit_control_map",
        "jit_control_filter",
        "jit_control_find",
        "jit_control_find_index",
        "jit_control_some",
        "jit_control_every",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_control_foreach(ctx, count) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // count
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_control_foreach", Linkage::Import, &sig)
            .expect("Failed to declare jit_control_foreach");
        ffi_funcs.insert("jit_control_foreach".to_string(), func_id);
    }
}
