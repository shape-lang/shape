//! Typed formatted-string FFI symbol registration.

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use crate::ffi::formatting::jit_format_value;

pub fn register_formatting_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_format_value", jit_format_value as *const u8);
}

/// Declare `(value_bits, source_kind, spec, precision) -> Arc<String> bits`.
pub fn declare_formatting_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I8));
    sig.params.push(AbiParam::new(types::I8));
    sig.params.push(AbiParam::new(types::I8));
    sig.returns.push(AbiParam::new(types::I64));
    let func_id = module
        .declare_function("jit_format_value", Linkage::Import, &sig)
        .expect("Failed to declare jit_format_value");
    ffi_funcs.insert("jit_format_value".to_string(), func_id);
}
