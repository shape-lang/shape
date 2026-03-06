//! GC FFI Symbol Registration
//!
//! Registers the GC safepoint function symbol with the JIT builder
//! and declares its signature in the Cranelift module.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::gc::jit_gc_safepoint;

/// Register GC FFI symbols with the JIT builder
pub fn register_gc_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_gc_safepoint", jit_gc_safepoint as *const u8);
}

/// Declare GC FFI function signatures in the module
pub fn declare_gc_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_gc_safepoint(ctx: *mut JITContext) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx pointer
        let func_id = module
            .declare_function("jit_gc_safepoint", Linkage::Import, &sig)
            .expect("Failed to declare jit_gc_safepoint");
        ffi_funcs.insert("jit_gc_safepoint".to_string(), func_id);
    }
}
