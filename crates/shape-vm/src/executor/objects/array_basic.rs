//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse, clone, zip
//!
//! ## Wave-β `A-array-basic-query` migration (playbook §10 / §7 REVISED)
//!
//! The `MethodFnV2` native ABI takes `args: &mut [u64]` raw bits with **no
//! parallel `NativeKind` track**. Every handler in this file previously
//! reconstructed a `ValueWord` from raw bits via `ValueWord::from_raw_bits`
//! (`borrow_vw`), called `as_any_array()` on it (which dispatched via the
//! deleted tag_bits + `as_heap_ref` shape forbidden by ADR-006 §2.7.7 #4 /
//! #7), and rebuilt result arrays through `shape_value::vmarray_from_vec`
//! (deleted in the substep-1 ValueWord-flavoured-helpers sweep).
//!
//! Sourcing the receiver kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted tag_bits
//!     dispatch — §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op" (forbidden
//!     §2.7.7 #9 — the W-series rationalization the playbook names
//!     verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     `ValueWord` synthesizer (forbidden §2.7.7 #7).
//!
//! Per playbook §7 REVISED the correct shape is `NotImplemented(SURFACE: ...)`
//! — surface the gap to the supervisor rather than paper over with a
//! forbidden pattern. The MethodFnV2 ABI extension (parallel-kind track on
//! `args` analogous to §2.7.7's stack track and §2.7.8's cell-store track)
//! is the architectural next step before these handlers can be migrated.
//!
//! Reference templates: cluster D `executor/objects/array_joins.rs` (whole-
//! file MethodFnV2 surface), cluster D `executor/objects/array_operations.rs`
//! (the kinded opcode-handler pattern with TypedArrayData dispatch — applies
//! to opcodes, not the V2 ABI).

use crate::executor::VirtualMachine;
use shape_value::VMError;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — kind-blind ABI, all surfaces
// ═══════════════════════════════════════════════════════════════════════════

/// Surface message shared by every handler in this file. The MethodFnV2 ABI
/// gap is identical for every receiver/arg slot — repeat the same diagnostic
/// pattern as cluster D `array_joins.rs` so the supervisor's grep over
/// "MethodFnV2 ABI lacks parallel NativeKind track" finds them all.
#[inline]
fn surface_v2_abi_gap(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "{method} — SURFACE: phase-2c — MethodFnV2 ABI lacks parallel \
         NativeKind track (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot \
         dispatch on receiver array kind, iterate element kinds, or rebuild \
         result arrays without kind-aware extension to the V2 ABI. The \
         pre-Wave-6.5 body decoded receiver via `ValueWord::from_raw_bits` + \
         `as_any_array` (forbidden tag_bits dispatch — §2.7.7 #4 / #7) and \
         rebuilt arrays via the deleted `shape_value::vmarray_from_vec`. \
         Phase-2c follow-up: extend MethodFnV2 to `args: &mut [(u64, \
         NativeKind)]` (or equivalent parallel track) analogous to the \
         stack/cell-store §2.7.7/§2.7.8 invariants."
    ))
}

pub(crate) fn handle_len_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("len"))
}

pub(crate) fn handle_first_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("first"))
}

pub(crate) fn handle_last_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("last"))
}

pub(crate) fn handle_reverse_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("reverse"))
}

pub(crate) fn handle_push_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("push"))
}

pub(crate) fn handle_pop_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("pop"))
}

pub(crate) fn handle_zip_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // zip's gap is doubled — both `args[0]` and `args[1]` need to be proven
    // `Ptr(HeapKind::TypedArray)` and per-element kinds need to be tracked
    // for the inner pair-array allocations. Same architectural surface.
    Err(surface_v2_abi_gap("zip"))
}

pub(crate) fn handle_clone_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("clone"))
}
