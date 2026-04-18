//! FFI Symbol Registration for JIT Compiler
//!
//! This module handles registration of FFI function symbols with the JIT
//! builder and declaration of their signatures in the Cranelift module.
//!
//! After the V6 cleanup (part of the v2 spec alignment), only the symbol
//! files corresponding to live `FFIFuncRefs` entries are registered. The
//! remaining Rust FFI impls that lost their symbol registration still
//! compile (some are direct-call helpers inside `ffi/math.rs` etc.) but are
//! no longer exposed to Cranelift.

use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::FuncId;
use std::collections::HashMap;

// Import live FFI symbol registration modules
mod arc_symbols;
mod array_symbols;
mod control_symbols;
mod math_symbols;
mod object_symbols;
mod v2_symbols;

// Import existing FFI implementation submodules (direct-call helpers;
// these do not register as Cranelift symbols but are reachable via Rust
// call sites — e.g. SIMD intrinsics invoked by FFI-level series kernels).
mod data_access; // Generic data source access (renamed from market_data for industry-agnostic design)
mod helpers;
mod intrinsics;
mod series;
mod simulation; // Generic simulation engine for stateful iteration
mod vector;

// Re-export for use by other modules
pub use arc_symbols::{declare_arc_functions, register_arc_symbols};
pub use array_symbols::{declare_array_functions, register_array_symbols};
pub use control_symbols::{declare_control_functions, register_control_symbols};
pub use math_symbols::{declare_math_functions, register_math_symbols};
pub use object_symbols::{declare_object_functions, register_object_symbols};
pub use v2_symbols::{declare_v2_functions, register_v2_symbols};

/// Register all FFI function symbols with the JIT builder
pub fn register_ffi_symbols(builder: &mut JITBuilder) {
    // Register symbols from each live module
    register_array_symbols(builder);
    register_object_symbols(builder);
    register_math_symbols(builder);
    register_control_symbols(builder);
    register_arc_symbols(builder);
    register_v2_symbols(builder);
}

/// Declare all FFI function signatures in the module
pub fn declare_ffi_functions(module: &mut JITModule) -> HashMap<String, FuncId> {
    let mut ffi_funcs = HashMap::new();

    // Declare functions from each live module
    declare_array_functions(module, &mut ffi_funcs);
    declare_object_functions(module, &mut ffi_funcs);
    declare_math_functions(module, &mut ffi_funcs);
    declare_control_functions(module, &mut ffi_funcs);
    declare_arc_functions(module, &mut ffi_funcs);
    declare_v2_functions(module, &mut ffi_funcs);

    ffi_funcs
}
