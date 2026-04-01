//! ARC FFI symbol registration for JIT compiler

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::arc;

/// Register ARC FFI symbols with the JIT builder
pub fn register_arc_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_arc_retain", arc::jit_arc_retain as *const u8);
    builder.symbol("jit_arc_release", arc::jit_arc_release as *const u8);
}

/// Declare ARC FFI function signatures in the Cranelift module
pub fn declare_arc_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_arc_retain(bits: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        if let Ok(func_id) =
            module.declare_function("jit_arc_retain", Linkage::Import, &sig)
        {
            ffi_funcs.insert("jit_arc_retain".to_string(), func_id);
        }
    }

    // jit_arc_release(bits: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        if let Ok(func_id) =
            module.declare_function("jit_arc_release", Linkage::Import, &sig)
        {
            ffi_funcs.insert("jit_arc_release".to_string(), func_id);
        }
    }
}
