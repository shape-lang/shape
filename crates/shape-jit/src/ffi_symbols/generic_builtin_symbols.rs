//! Generic Builtin FFI Symbol Registration

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::generic_builtin::jit_generic_builtin;

/// Register generic builtin FFI symbol with the JIT builder
pub fn register_generic_builtin_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_generic_builtin", jit_generic_builtin as *const u8);
}

/// Declare generic builtin FFI function signature in the module
pub fn declare_generic_builtin_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    // jit_generic_builtin(ctx: *mut JITContext, builtin_id: u16, arg_count: u16) -> u64
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64)); // ctx
    sig.params.push(AbiParam::new(types::I16)); // builtin_id
    sig.params.push(AbiParam::new(types::I16)); // arg_count
    sig.returns.push(AbiParam::new(types::I64)); // result
    let func_id = module
        .declare_function("jit_generic_builtin", Linkage::Import, &sig)
        .expect("Failed to declare jit_generic_builtin");
    ffi_funcs.insert("jit_generic_builtin".to_string(), func_id);
}
