// ============================================================================
// Vector Intrinsics
// ============================================================================
//
// Per ADR-006 §2.7.5, the JIT-FFI boundary carries raw `u64` plus a parallel
// `NativeKind` companion stamped at JIT compile time from the call signature.
// These extern "C" entry-points retain the raw `u64` ABI shape so the
// Cranelift call sites in the JIT codegen don't need to change today.
//
// **Phase-2c surface (ADR-006 §2.7.4): vector intrinsics rebuild.** The
// pre-bulldozer bodies decoded `&[ValueWord]` argument arrays via tag-bit
// dispatch (`as_any_array()`, `as_f64_slice()`, etc.) and dispatched to
// `shape_runtime::intrinsics::vector::intrinsic_vec_*` /
// `intrinsics::matrix::intrinsic_*` free functions. Both pieces are deleted:
// `ValueWord` is gone post-strict-typing, and the runtime now exposes vector
// / matrix intrinsics only as `ModuleExports` (`__intrinsic_vec_abs`, etc.)
// with kind-threaded `&[KindedSlot]` body shims that themselves return a
// deferred error in Phase 1.B (mirroring `multi_table::functions::align_tables`
// at `crates/shape-runtime/src/multi_table/functions.rs:30`).
//
// The kind-threaded rebuild (per-position `NativeKind` flowing from the
// JIT-emitted call signature into a `KindedSlot` carrier the runtime
// `__intrinsic_vec_*` body consumes) lands in Phase 2c. Until then every
// entry-point surface-and-stops per W10 playbook §5.

use super::super::context::JITContext;

#[inline]
fn vector_intrinsic_phase_2c() -> ! {
    todo!("phase-2c — see ADR-006 §2.7.4: vector intrinsics rebuild")
}

pub extern "C" fn jit_intrinsic_vec_abs(_ctx: *mut JITContext, _arg_bits: u64) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_sqrt(_ctx: *mut JITContext, _arg_bits: u64) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_ln(_ctx: *mut JITContext, _arg_bits: u64) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_exp(_ctx: *mut JITContext, _arg_bits: u64) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_add(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_sub(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_mul(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_div(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_max(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_vec_min(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_matmul_vec(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_matmul_mat(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

// ===== R5.4D: Matrix/Vec arithmetic intrinsics (unwired) =====
//
// Same Phase-2c surface as the rest of the family; tracked under the same
// rebuild ticket per ADR-006 §2.7.4.

pub extern "C" fn jit_intrinsic_vec_add_i64(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_mat_add(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}

pub extern "C" fn jit_intrinsic_mat_sub(
    _ctx: *mut JITContext,
    _a_bits: u64,
    _b_bits: u64,
) -> u64 {
    vector_intrinsic_phase_2c()
}
