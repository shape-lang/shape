//! Typed formatted-string FFI symbol registration.

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use crate::ffi::formatting::{
    jit_format_default_bool, jit_format_default_f64, jit_format_default_i64,
    jit_format_default_string, jit_format_fixed_bool, jit_format_fixed_f64, jit_format_fixed_i64,
    jit_format_fixed_string,
};

pub fn register_formatting_symbols(builder: &mut JITBuilder) {
    builder.symbol(
        "jit_format_default_i64",
        jit_format_default_i64 as *const u8,
    );
    builder.symbol(
        "jit_format_default_bool",
        jit_format_default_bool as *const u8,
    );
    builder.symbol(
        "jit_format_default_f64",
        jit_format_default_f64 as *const u8,
    );
    builder.symbol(
        "jit_format_default_string",
        jit_format_default_string as *const u8,
    );
    builder.symbol("jit_format_fixed_i64", jit_format_fixed_i64 as *const u8);
    builder.symbol("jit_format_fixed_bool", jit_format_fixed_bool as *const u8);
    builder.symbol("jit_format_fixed_f64", jit_format_fixed_f64 as *const u8);
    builder.symbol(
        "jit_format_fixed_string",
        jit_format_fixed_string as *const u8,
    );
}

fn declare_formatting_function(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
    name: &str,
    params: &[types::Type],
) {
    let mut sig = module.make_signature();
    sig.params.extend(params.iter().copied().map(AbiParam::new));
    sig.returns.push(AbiParam::new(types::I64));
    let func_id = module
        .declare_function(name, Linkage::Import, &sig)
        .unwrap_or_else(|error| panic!("Failed to declare {name}: {error}"));
    ffi_funcs.insert(name.to_string(), func_id);
}

/// Declare source-kind- and policy-specific interpolation imports.
pub fn declare_formatting_functions(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
) {
    declare_formatting_function(module, ffi_funcs, "jit_format_default_i64", &[types::I64]);
    declare_formatting_function(module, ffi_funcs, "jit_format_default_bool", &[types::I8]);
    declare_formatting_function(module, ffi_funcs, "jit_format_default_f64", &[types::F64]);
    declare_formatting_function(
        module,
        ffi_funcs,
        "jit_format_default_string",
        &[types::I64],
    );
    declare_formatting_function(
        module,
        ffi_funcs,
        "jit_format_fixed_i64",
        &[types::I64, types::I8],
    );
    declare_formatting_function(
        module,
        ffi_funcs,
        "jit_format_fixed_bool",
        &[types::I8, types::I8],
    );
    declare_formatting_function(
        module,
        ffi_funcs,
        "jit_format_fixed_f64",
        &[types::F64, types::I8],
    );
    declare_formatting_function(
        module,
        ffi_funcs,
        "jit_format_fixed_string",
        &[types::I64, types::I8],
    );
}
