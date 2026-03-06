//! Reference FFI Symbol Registration
//!
//! Registers the set_index_ref function symbol with the JIT builder
//! and declares its signature in the Cranelift module.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::references::jit_set_index_ref;

/// Register reference FFI symbols with the JIT builder
pub fn register_reference_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_set_index_ref", jit_set_index_ref as *const u8);
}

/// Declare reference FFI function signatures in the module
pub fn declare_reference_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    // jit_set_index_ref(ref_ptr: *mut u64, index: u64, value: u64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ref_ptr
        sig.params.push(AbiParam::new(types::I64)); // index (NaN-boxed)
        sig.params.push(AbiParam::new(types::I64)); // value (NaN-boxed)
        let func_id = module
            .declare_function("jit_set_index_ref", Linkage::Import, &sig)
            .expect("Failed to declare jit_set_index_ref");
        ffi_funcs.insert("jit_set_index_ref".to_string(), func_id);
    }
}
