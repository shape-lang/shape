//! FFI Symbol Registration for JIT Compiler
//!
//! This module handles registration of FFI function symbols with the JIT builder
//! and declaration of their signatures in the Cranelift module.

use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::FuncId;
use std::collections::HashMap;

// Import FFI symbol registration modules
mod arc_symbols;
mod array_symbols;
mod async_symbols;
mod control_symbols;
mod data_symbols;
mod gc_symbols;
mod generic_builtin_symbols;
mod math_symbols;
mod object_symbols;
mod reference_symbols;
mod result_option_symbols;
mod simd_symbols;
mod v2_symbols;

// Import existing FFI implementation submodules
mod data_access; // Generic data source access (renamed from market_data for industry-agnostic design)
mod helpers;
mod intrinsics;
mod series;
mod simulation; // Generic simulation engine for stateful iteration
mod vector;

// Re-export for use by other modules
pub use array_symbols::{declare_array_functions, register_array_symbols};
pub use async_symbols::{declare_async_functions, register_async_symbols};
pub use control_symbols::{declare_control_functions, register_control_symbols};
pub use data_symbols::{declare_data_functions, register_data_symbols};
pub use gc_symbols::{declare_gc_functions, register_gc_symbols};
pub use generic_builtin_symbols::{
    declare_generic_builtin_functions, register_generic_builtin_symbols,
};
pub use math_symbols::{declare_math_functions, register_math_symbols};
pub use object_symbols::{declare_object_functions, register_object_symbols};
pub use reference_symbols::{declare_reference_functions, register_reference_symbols};
pub use result_option_symbols::{declare_result_option_functions, register_result_option_symbols};
pub use arc_symbols::{declare_arc_functions, register_arc_symbols};
pub use simd_symbols::{declare_simd_functions, register_simd_symbols};
pub use v2_symbols::{declare_v2_functions, register_v2_symbols};

/// Register all FFI function symbols with the JIT builder
pub fn register_ffi_symbols(builder: &mut JITBuilder) {
    // Register symbols from each module
    register_array_symbols(builder);
    register_async_symbols(builder);
    register_object_symbols(builder);
    register_data_symbols(builder);
    register_math_symbols(builder);
    register_control_symbols(builder);
    register_result_option_symbols(builder);
    register_simd_symbols(builder);
    register_gc_symbols(builder);
    register_reference_symbols(builder);
    register_generic_builtin_symbols(builder);
    register_arc_symbols(builder);
    register_v2_symbols(builder);
}

/// Declare all FFI function signatures in the module
pub fn declare_ffi_functions(module: &mut JITModule) -> HashMap<String, FuncId> {
    let mut ffi_funcs = HashMap::new();

    // Declare functions from each module
    declare_array_functions(module, &mut ffi_funcs);
    declare_async_functions(module, &mut ffi_funcs);
    declare_object_functions(module, &mut ffi_funcs);
    declare_data_functions(module, &mut ffi_funcs);
    declare_math_functions(module, &mut ffi_funcs);
    declare_control_functions(module, &mut ffi_funcs);
    declare_result_option_functions(module, &mut ffi_funcs);
    declare_simd_functions(module, &mut ffi_funcs);
    declare_gc_functions(module, &mut ffi_funcs);
    declare_reference_functions(module, &mut ffi_funcs);
    declare_generic_builtin_functions(module, &mut ffi_funcs);
    declare_arc_functions(module, &mut ffi_funcs);
    declare_v2_functions(module, &mut ffi_funcs);

    ffi_funcs
}
