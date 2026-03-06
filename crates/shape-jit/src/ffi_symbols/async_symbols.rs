//! Async Task FFI Symbol Registration
//!
//! Registers JIT FFI symbols for async task operations:
//! spawn, join_init, join_await, cancel, scope_enter, scope_exit.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::async_ops::{
    jit_async_scope_enter, jit_async_scope_exit, jit_cancel_task, jit_join_await, jit_join_init,
    jit_spawn_task,
};

/// Register async task FFI symbols with the JIT builder
pub fn register_async_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_spawn_task", jit_spawn_task as *const u8);
    builder.symbol("jit_join_init", jit_join_init as *const u8);
    builder.symbol("jit_join_await", jit_join_await as *const u8);
    builder.symbol("jit_cancel_task", jit_cancel_task as *const u8);
    builder.symbol("jit_async_scope_enter", jit_async_scope_enter as *const u8);
    builder.symbol("jit_async_scope_exit", jit_async_scope_exit as *const u8);
}

/// Declare async task FFI function signatures in the module
pub fn declare_async_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_spawn_task(ctx: *mut JITContext, callable_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // callable_bits
        sig.returns.push(AbiParam::new(types::I64)); // Future bits
        let func_id = module
            .declare_function("jit_spawn_task", Linkage::Import, &sig)
            .expect("Failed to declare jit_spawn_task");
        ffi_funcs.insert("jit_spawn_task".to_string(), func_id);
    }

    // jit_join_init(ctx: *mut JITContext, packed: u16) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I16)); // packed (u16)
        sig.returns.push(AbiParam::new(types::I64)); // TaskGroup bits
        let func_id = module
            .declare_function("jit_join_init", Linkage::Import, &sig)
            .expect("Failed to declare jit_join_init");
        ffi_funcs.insert("jit_join_init".to_string(), func_id);
    }

    // jit_join_await(ctx: *mut JITContext, task_group_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // task_group_bits
        sig.returns.push(AbiParam::new(types::I64)); // result (TAG_NULL on suspension)
        let func_id = module
            .declare_function("jit_join_await", Linkage::Import, &sig)
            .expect("Failed to declare jit_join_await");
        ffi_funcs.insert("jit_join_await".to_string(), func_id);
    }

    // jit_cancel_task(ctx: *mut JITContext, future_bits: u64) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // future_bits
        sig.returns.push(AbiParam::new(types::I32)); // 0 success, -1 failure
        let func_id = module
            .declare_function("jit_cancel_task", Linkage::Import, &sig)
            .expect("Failed to declare jit_cancel_task");
        ffi_funcs.insert("jit_cancel_task".to_string(), func_id);
    }

    // jit_async_scope_enter(ctx: *mut JITContext) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I32)); // 0 success, -1 failure
        let func_id = module
            .declare_function("jit_async_scope_enter", Linkage::Import, &sig)
            .expect("Failed to declare jit_async_scope_enter");
        ffi_funcs.insert("jit_async_scope_enter".to_string(), func_id);
    }

    // jit_async_scope_exit(ctx: *mut JITContext) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I32)); // 0 success, -1 failure
        let func_id = module
            .declare_function("jit_async_scope_exit", Linkage::Import, &sig)
            .expect("Failed to declare jit_async_scope_exit");
        ffi_funcs.insert("jit_async_scope_exit".to_string(), func_id);
    }
}
