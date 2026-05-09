//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every,
//! any, all, single, take_while, skip_while, for_each
//!
//! ## Wave-β `A-array-basic-query` migration (playbook §10 / §7 REVISED)
//!
//! The `MethodFnV2` native ABI takes `args: &mut [u64]` raw bits with **no
//! parallel `NativeKind` track**. Every handler in this file previously:
//!
//! 1. Reconstructed a `ValueWord` from `args[0]` via `ValueWord::from_raw_bits`
//!    (`borrow_vw`), called `as_any_array()` on it (forbidden tag_bits +
//!    `as_heap_ref` dispatch — §2.7.7 #4 / #7).
//! 2. For higher-order variants, queried `raw_helpers::is_callable_raw(bits)`
//!    and `raw_helpers::callable_arity_raw(bits)` — both deleted by Wave-α
//!    `D-raw-helpers` because they decoded callable-ness from the deleted
//!    NaN-boxed dynamic representation (forbidden §2.7.7 #7).
//! 3. Rebuilt result arrays through `shape_value::vmarray_from_vec` and
//!    retained borrowed elements via `shape_value::vw_clone` — both deleted
//!    in the substep-1 sweep; replaced by `clone_with_kind` /
//!    `drop_with_kind` on a parallel-kind track that the V2 ABI does not
//!    have.
//! 4. Decoded predicate booleans via `is_bool_true_raw` which probed
//!    `shape_value::tag_bits::is_tagged(bits)` — verbatim the deleted
//!    tag_bits dispatch §2.7.7 #4 / #7 forbids.
//!
//! The architectural surface is closure-call dispatch: every higher-order
//! method here calls `vm.call_value_immediate_raw(args[1], &[...], ctx)`
//! which (a) cannot be type-checked without `is_callable_raw` proof, (b)
//! cannot pass kinded args without a kinded callee/args ABI, and (c) cannot
//! interpret the returned `u64` without a kinded return slot. This is the
//! cell-extension consumer that ADR-006 §2.7.8 specifies as the prerequisite
//! for migrating the closure-call dispatch — i.e. `B7-closure-cells`,
//! `B8-shared-cell`, `B9-callframe-kind` STRUCTURAL Wave-α work plus the
//! V2 ABI parallel-kind track extension.
//!
//! Per playbook §7 REVISED the correct shape is `NotImplemented(SURFACE: ...)`.
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden the deleted tag_bits
//!     dispatch — §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` (forbidden §2.7.7 #9), or
//!   - probing a heap discriminant via the deleted `as_heap_ref` /
//!     ValueWord-synthesizer family (forbidden §2.7.7 #7).
//!
//! Reference template: cluster D `executor/objects/array_joins.rs` (whole-
//! file MethodFnV2 surface, same architectural ABI gap, same diagnostic
//! shape).

use crate::executor::VirtualMachine;
use shape_value::VMError;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — kind-blind ABI, all surfaces
// ═══════════════════════════════════════════════════════════════════════════

/// Surface message shared by every read-only query handler (no closure
/// callback). The receiver-array decode requires kinded `args[0]`; rebuilding
/// result arrays requires per-element kind threading. Both gaps are §2.7.7
/// / §2.7.8 follow-ups.
#[inline]
fn surface_v2_abi_gap(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "{method} — SURFACE: phase-2c — MethodFnV2 ABI lacks parallel \
         NativeKind track (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot \
         dispatch on receiver array kind, iterate element kinds, or rebuild \
         result arrays without kind-aware extension to the V2 ABI. The \
         pre-Wave-6.5 body decoded receiver via `ValueWord::from_raw_bits` + \
         `as_any_array` (forbidden tag_bits dispatch — §2.7.7 #4 / #7) and \
         rebuilt arrays via the deleted `shape_value::vmarray_from_vec` / \
         `shape_value::vw_clone`. Phase-2c follow-up: extend MethodFnV2 with \
         a parallel kind track on `args` analogous to §2.7.7/§2.7.8."
    ))
}

/// Surface message for higher-order query handlers (closure callback). Adds
/// the §2.7.8 cell-extension consumer migration requirement on top of the
/// generic V2 ABI gap.
#[inline]
fn surface_closure_dispatch_gap(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "{method} — SURFACE: phase-2c — closure-call dispatch needs §2.7.8 \
         cell-extension consumer migration. The pre-Wave-6.5 body called \
         `raw_helpers::is_callable_raw` / `raw_helpers::callable_arity_raw` \
         (both deleted by Wave-α D-raw-helpers as forbidden tag_bits dispatch \
         — §2.7.7 #7), then `vm.call_value_immediate_raw` with kind-blind \
         u64 args, then decoded the boolean return via \
         `shape_value::tag_bits::is_tagged` (forbidden tag_bits dispatch — \
         §2.7.7 #4 / #7) and `as_bool` on a `ValueWord::from_raw_bits` \
         reconstruction. Migration depends on (a) Wave-α B7-closure-cells / \
         B8-shared-cell / B9-callframe-kind STRUCTURAL items per ADR-006 \
         §2.7.8 (cell parallel-kind tracks), (b) MethodFnV2 ABI extension \
         with parallel-kind track on `args` (§2.7.7-shape applied to the \
         method ABI), and (c) `op_call_value` kinded callee/args/return \
         contract (B11-control-flow-heap)."
    ))
}

pub(crate) fn handle_where_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("where"))
}

pub(crate) fn handle_select_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("select"))
}

pub(crate) fn handle_find_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("find"))
}

pub(crate) fn handle_find_index_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("findIndex"))
}

pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // No closure callback in indexOf — the gap is the receiver/value kind
    // discipline (and `nb_equal` was a `vw_equals` ValueWord-flavoured
    // equality, which is itself §2.7.7 #4 / #7 forbidden once kinds are
    // present and the canonical shape is per-`NativeKind` equality).
    Err(surface_v2_abi_gap("indexOf"))
}

pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_v2_abi_gap("includes"))
}

pub(crate) fn handle_some_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("some"))
}

pub(crate) fn handle_every_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("every"))
}

pub(crate) fn handle_any_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // `any` is the pre-existing alias for `some` — same architectural surface.
    Err(surface_closure_dispatch_gap("any"))
}

pub(crate) fn handle_all_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // `all` is the pre-existing alias for `every` — same architectural surface.
    Err(surface_closure_dispatch_gap("all"))
}

pub(crate) fn handle_single_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("single"))
}

pub(crate) fn handle_take_while_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("takeWhile"))
}

pub(crate) fn handle_skip_while_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("skipWhile"))
}

pub(crate) fn handle_for_each_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_closure_dispatch_gap("forEach"))
}
