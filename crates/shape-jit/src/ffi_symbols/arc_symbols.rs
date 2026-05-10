//! ARC FFI symbol registration for JIT compiler.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! The pre-strict-typing entry points (`jit_arc_retain` /
//! `jit_arc_release`) were removed from `super::super::ffi::arc`
//! because they wrapped the deleted kind-blind W-series helpers
//! (`vw_clone` / `vw_drop`). See `ffi/arc.rs` for the full SURFACE
//! comment.
//!
//! Symbol registration and signature declaration are no-ops until
//! the kind-aware FFI rebuild lands (W11 / deeper Phase-2c). The
//! caller side — `mir_compiler/rvalues.rs::Rvalue::Clone` and the
//! matching `Rvalue::Drop` lowering — is the upstream blocker; once
//! it stamps a kind companion onto the call signature, the
//! kind-aware retain/release entry points return here and these
//! symbols re-register with the new ABI shape.

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::FuncId;
use std::collections::HashMap;

/// SURFACE: no symbols to register pending the §2.7.5 kind-aware
/// FFI rebuild. JIT call sites that emit `arc_retain` / `arc_release`
/// will fail to find these symbols, surfacing the upstream blocker.
pub fn register_arc_symbols(_builder: &mut JITBuilder) {}

/// SURFACE: no signatures to declare pending the §2.7.5 rebuild.
pub fn declare_arc_functions(_module: &mut JITModule, _ffi_funcs: &mut HashMap<String, FuncId>) {}
