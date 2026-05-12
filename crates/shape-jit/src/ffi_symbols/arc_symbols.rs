//! ARC FFI symbol registration for JIT compiler.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! `jit_arc_retain` and `jit_arc_release` register the typed-Arc
//! retain/release primitives. Under Route A the JIT-FFI carries
//! `*const HeapHeader` directly — kind is structurally encoded in the
//! heap header's `kind: u16` field (§2.7.6 / Q8 single-discriminator),
//! so the FFI body needs no kind side-channel argument. See
//! `super::super::ffi::arc` for the ABI contract and Route A invariants.

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::arc;

/// Register the typed-Arc retain/release symbols.
pub fn register_arc_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_arc_retain", arc::jit_arc_retain as *const u8);
    builder.symbol("jit_arc_release", arc::jit_arc_release as *const u8);
}

/// Declare the Cranelift signatures for `jit_arc_retain` / `jit_arc_release`.
///
/// Both entries are `extern "C" fn(ptr: i64)` — a single `*const HeapHeader`
/// argument, no return value. Kind dispatch happens inside the body via
/// the `HeapHeader.kind` field on the release path (refcount-zero).
pub fn declare_arc_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_arc_retain(ptr: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        if let Ok(func_id) = module.declare_function("jit_arc_retain", Linkage::Import, &sig) {
            ffi_funcs.insert("jit_arc_retain".to_string(), func_id);
        }
    }
    // jit_arc_release(ptr: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        if let Ok(func_id) = module.declare_function("jit_arc_release", Linkage::Import, &sig) {
            ffi_funcs.insert("jit_arc_release".to_string(), func_id);
        }
    }
}
