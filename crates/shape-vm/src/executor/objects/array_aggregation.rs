//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce
//!
//! ## Wave-β `A-array-transform` migration (playbook §10 / ADR-006 §2.7.6 / §2.7.7 / Q8 / Q9)
//!
//! The `MethodFnV2` native ABI takes `args: &mut [u64]` raw bits with **no
//! parallel `NativeKind` track**. Every interaction this file needs —
//! interpreting `args[0]` as an array (vs the deleted `as_any_array`
//! ValueWord probe), iterating `TypedArrayData` element-by-element with a
//! per-element kind for sum/avg/min/max numeric coercion, calling back
//! through `op_call_value` for `reduce`/`count(predicate)` with kinded
//! callee + arg + count slots — is impossible without a kind-aware
//! extension to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted
//!     tag_bits dispatch — §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op" (forbidden
//!     §2.7.7 #9 — the W-series rationalization the playbook names
//!     verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7), or
//!   - calling the deleted `extract_number_coerce` raw-bits helper
//!     (deleted alongside `raw_helpers::is_callable_raw` /
//!     `callable_arity_raw` / `is_truthy_raw` per Wave-α
//!     `D-raw-helpers` close — surfaced here as an upstream cascade).
//!
//! Per playbook §7 (REVISED) the correct shape is `NotImplemented(
//! SURFACE: ...)` — surface the gap to the supervisor rather than paper
//! over with a forbidden pattern. The MethodFnV2 ABI extension (parallel-
//! kind track on `args` analogous to §2.7.7's stack track and §2.7.8's
//! cell-store track) is the architectural next step before these handlers
//! can be migrated.
//!
//! Mirrors the surface shape established by `array_joins.rs` (Wave-α
//! `D-array-joins` close).

use crate::executor::VirtualMachine;
use shape_value::VMError;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — all surface-and-stop per playbook §7
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn handle_sum_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: needs (1) receiver-kind proof
    // `NativeKind::Ptr(HeapKind::TypedArray)` to dispatch on the inner
    // `TypedArrayData::*` variant per ADR-005 §1, then (2) per-variant
    // numeric reduction (I64 → i64 sum, F64 → f64 sum, mixed-int-width
    // requires per-arm dispatch). The pre-Wave-6.5 body's
    // `view.as_any_array().as_f64_slice()` / `as_i64_slice()` /
    // `to_generic()` fast-path probed `ValueWord` tag bits — forbidden
    // §2.7.7. The kinded equivalent is `Arc::<TypedArrayData>::from_raw`
    // + match on the variant, which requires the receiver kind, which
    // the V2 ABI does not provide.
    Err(VMError::NotImplemented(
        "sum — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot dispatch on receiver \
         array kind or iterate element kinds without kind-aware extension. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_avg_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_sum_v2` — receiver-kind proof needed
    // to dispatch on `TypedArrayData::*`, then per-variant accumulation
    // and division. The numeric-coercion path through the deleted
    // `as_number_coerce` is the same forbidden tag_bits dispatch the
    // playbook §4 entries 4 / 7 / 8 rule out.
    Err(VMError::NotImplemented(
        "avg — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as sum — needs \
         receiver-kind proof + per-TypedArrayData-variant numeric reduction. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_min_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_sum_v2`. The result-kind sourcing
    // also splits per `TypedArrayData::*` arm (I64 returns Int64,
    // F64 returns Float64, etc.); papering over with a single
    // "coerce-to-f64-everywhere" path was the deleted ValueWord shape.
    Err(VMError::NotImplemented(
        "min — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Needs receiver-kind proof + \
         per-TypedArrayData-variant min reduction with arm-driven result \
         kind. Phase-2c follow-up: extend MethodFnV2 with parallel kind \
         track."
            .to_string(),
    ))
}

pub(crate) fn handle_max_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_min_v2`.
    Err(VMError::NotImplemented(
        "max — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Needs receiver-kind proof + \
         per-TypedArrayData-variant max reduction with arm-driven result \
         kind. Phase-2c follow-up: extend MethodFnV2 with parallel kind \
         track."
            .to_string(),
    ))
}

pub(crate) fn handle_count_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: count() (no-arg) needs only `view.len()` from the
    // receiver, but reaching the receiver from a raw u64 still needs
    // the receiver-kind proof + Arc-from-raw on
    // `NativeKind::Ptr(HeapKind::TypedArray)`. count(predicate) needs,
    // additionally, callable-arity dispatch (deleted
    // `is_callable_raw`/`callable_arity_raw` were Wave-α
    // `D-raw-helpers` deletion targets) and a kinded
    // `op_call_value` callback — same architectural gap.
    Err(VMError::NotImplemented(
        "count — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Needs receiver-kind proof \
         even for no-arg form; predicate form additionally needs the \
         kinded op_call_value callback closure_raw::is_callable / arity \
         was deleted with the raw_helpers tag_bits family. Phase-2c \
         follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_reduce_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: reduce(reducer, initial) needs (1) receiver-kind proof,
    // (2) per-element-kind iteration over `TypedArrayData::*`, (3)
    // kinded callback through `op_call_value` with the reducer
    // function — args[1] (the reducer) has no kind in the V2 ABI, so
    // the closure_raw::is_callable check (deleted) cannot be replayed
    // and the per-iteration accumulator-kind threading has nowhere to
    // store its NativeKind. Same architectural gap as the higher-order
    // ops in `array_transform.rs`.
    Err(VMError::NotImplemented(
        "reduce/fold — SURFACE: MethodFnV2 ABI lacks parallel NativeKind \
         track (ADR-006 §2.7.7 / §2.7.8 follow-up). Higher-order op \
         dispatch needs kinded reducer + kinded accumulator + element-kind \
         iteration; closure_raw::is_callable / arity were deleted with \
         the raw_helpers tag_bits family. Phase-2c follow-up: extend \
         MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}
