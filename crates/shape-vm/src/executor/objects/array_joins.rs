//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join
//!
//! Wave 6.5 substep-2 sub-cluster D-array-joins (ADR-006 §2.7.7 / §2.7.8 /
//! Q9 / Q10): the `MethodFnV2` native ABI takes `args: &mut [u64]` raw bits
//! with **no parallel `NativeKind` track**. Every interaction this file
//! needs — interpreting `args[i]` as an array vs. closure, iterating
//! `TypedArrayData` element-by-element with a per-element kind, calling
//! back into `op_call_value` with kinded callee + arg + count slots — is
//! impossible without a kind-aware extension to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden the deleted tag_bits dispatch —
//!     §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op" (forbidden
//!     §2.7.7 #9 — the W-series rationalization the playbook names
//!     verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7).
//!
//! Per playbook §7.4 (REVISED) the correct shape is `NotImplemented(
//! SURFACE: ...)` — surface the gap to the supervisor rather than paper
//! over with a forbidden pattern. The MethodFnV2 ABI extension (parallel-
//! kind track on `args` analogous to §2.7.7's stack track and §2.7.8's
//! cell-store track) is the architectural next step before these handlers
//! can be migrated.

use crate::executor::VirtualMachine;
use shape_value::VMError;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `innerJoin` — inner join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_inner_join_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: the MethodFnV2 ABI's `args: &mut [u64]` is kind-blind. Each
    // arg is a raw u64 with no companion `NativeKind` — see ADR-006 §2.7.7
    // (stack track) and §2.7.8 (cell-store track) for the lockstep
    // discipline that the V2 method ABI does not yet have.
    //
    // This handler needs:
    //   1. To interpret args[0..2] as arrays — requires
    //      `kind == NativeKind::Ptr(HeapKind::TypedArray)` proof per §2.7.7,
    //      then `as_heap_value() + HeapValue::TypedArray(arc)` match per
    //      ADR-005 §1 single-discriminator.
    //   2. To iterate `TypedArrayData` and push each element via
    //      `push_kinded(bits, element_kind)` — requires per-`TypedArrayData::*`
    //      arm-driven `NativeKind` (e.g. `I64 => Int64`, `F64 => Float64`,
    //      `String => NativeKind::String`, `HeapValue(_)` cannot be uniformly
    //      kinded — see §8 surface trigger).
    //   3. To call back via `op_call_value` with (callee, args, arg_count)
    //      kinded slots — requires kinded args[2..5] (closure / fn-id) which
    //      the V2 ABI does not provide.
    //
    // Bool-defaulting any of the above is forbidden (§2.7.7 #9). Decoding
    // kind from the raw bits is forbidden (§2.7.7 #4 / #7). The correct
    // refusal shape is the surface below.
    Err(VMError::NotImplemented(
        "innerJoin — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot dispatch on receiver \
         array kind, iterate element kinds, or call back through op_call_value \
         without kind-aware extension. Phase-2c follow-up: extend MethodFnV2 \
         to `args: &mut [(u64, NativeKind)]` (or equivalent parallel track) \
         analogous to the stack/cell-store §2.7.7/§2.7.8 invariants."
            .to_string(),
    ))
}

/// v2 `leftJoin` — left join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_left_join_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_inner_join_v2` — the MethodFnV2 ABI's
    // `args: &mut [u64]` is kind-blind, and the unmatched-row branch
    // additionally needs a kinded "none" sentinel slot to push, which
    // §2.7.7 specifies as `(0u64, NativeKind::Bool)` only when the kind
    // is statically known to be the §2.7 sentinel — not as a forbidden
    // Bool-default for any-shaped slot.
    Err(VMError::NotImplemented(
        "leftJoin — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as innerJoin plus \
         the unmatched-row branch needs a kinded `none` argument that \
         depends on the result_selector signature kind, not a Bool-default. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product)
///
/// args: [left_array, right_array, result_selector_fn]
pub(crate) fn handle_cross_join_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_inner_join_v2` — kind-blind args,
    // kind-blind element iteration, kind-blind callback path. Crosss-join
    // is the simplest of the three (no key functions, no equality), but
    // the same architectural ABI gap blocks all three.
    Err(VMError::NotImplemented(
        "crossJoin — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot interpret arg kinds, \
         iterate element kinds, or push kinded args for op_call_value. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}
