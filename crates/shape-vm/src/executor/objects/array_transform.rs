//! Array transformation operations
//!
//! Handles: map, filter, sort, slice, concat, take, drop, skip, flatten,
//! flat_map, group_by
//!
//! ## Wave-β `A-array-transform` migration (playbook §10 / ADR-006 §2.7.6 /
//! §2.7.7 / Q8 / Q9)
//!
//! The `MethodFnV2` native ABI takes `args: &mut [u64]` raw bits with **no
//! parallel `NativeKind` track**. Every interaction this file needs —
//! interpreting `args[0]` as a `TypedArray` receiver (vs the deleted
//! `as_any_array` ValueWord probe), iterating element bits with a
//! per-element kind, invoking closure callbacks through `op_call_value`
//! (formerly `call_value_immediate_raw` paired with the deleted
//! `raw_helpers::is_callable_raw` / `callable_arity_raw` /
//! `is_truthy_raw` / `extract_number_coerce` family) — is impossible
//! without a kind-aware extension to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted
//!     tag_bits dispatch — §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op" (forbidden
//!     §2.7.7 #9 — the W-series rationalization the playbook names
//!     verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7), or
//!   - calling the deleted `raw_helpers::is_callable_raw` /
//!     `callable_arity_raw` / `is_truthy_raw` /
//!     `extract_number_coerce` raw-bits helpers (deleted in Wave-α
//!     `D-raw-helpers` close — surfaced here as an upstream cascade,
//!     and additionally the `vmarray_from_vec` / `vw_clone` /
//!     `ValueWord::from_array` carrier construction APIs are gone with
//!     the v1 dynamic representation).
//!
//! Per playbook §7 (REVISED) the correct shape is `NotImplemented(
//! SURFACE: ...)` — surface the gap to the supervisor rather than paper
//! over with a forbidden pattern. The MethodFnV2 ABI extension (parallel-
//! kind track on `args` analogous to §2.7.7's stack track and §2.7.8's
//! cell-store track), plus a kinded `op_call_value` callback path, are
//! the architectural prerequisites before these handlers can be
//! migrated.
//!
//! Every higher-order op (map / filter / sort with comparator / flatMap /
//! groupBy) is doubly blocked — element kind AND closure-kind — and the
//! plain transforms (slice / concat / take / drop / skip / flatten) are
//! still single-blocked on receiver + element kind.
//!
//! Mirrors the surface shape established by `array_joins.rs` (Wave-α
//! `D-array-joins` close).

use crate::executor::VirtualMachine;
use shape_value::VMError;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — all surface-and-stop per playbook §7
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn handle_map_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: higher-order op needs (1) receiver-kind proof
    // `NativeKind::Ptr(HeapKind::TypedArray)` to dispatch on the inner
    // `TypedArrayData::*` variant, (2) per-element-kind iteration to
    // push each element into the kinded callback, (3) kinded mapper
    // closure via `op_call_value` (the deleted
    // `call_value_immediate_raw` + `raw_helpers::is_callable_raw` /
    // `callable_arity_raw` were the pre-Wave-6.5 cover for this), and
    // (4) result-array construction with per-element kind threaded
    // through (the `vmarray_from_vec` / `ValueWord::from_array` carrier
    // APIs are gone). All four steps need the kind track the V2 ABI
    // does not provide.
    Err(VMError::NotImplemented(
        "map — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Higher-order op needs kinded \
         receiver + kinded mapper + element-kind iteration; \
         closure_raw::is_callable / arity / call_value_immediate_raw and \
         the vmarray_from_vec result-construction path were deleted with \
         the v1 ValueWord representation. Phase-2c follow-up: extend \
         MethodFnV2 with parallel kind track + kinded op_call_value path."
            .to_string(),
    ))
}

pub(crate) fn handle_filter_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_map_v2` — receiver kind + predicate
    // kind + element-kind iteration. The deleted `is_truthy_raw` was
    // the predicate-result extractor; its kinded equivalent is
    // `kinded_truthy(bits, kind)` from `executor/logical/mod.rs`,
    // which requires the kind side-channel.
    Err(VMError::NotImplemented(
        "filter — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as map plus the \
         deleted is_truthy_raw predicate-result extractor; kinded \
         equivalent is logical::kinded_truthy(bits, kind) which requires \
         the kind side-channel. Phase-2c follow-up: extend MethodFnV2 \
         with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_sort_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: sort needs (1) receiver-kind proof, (2) per-element-kind
    // comparison (the deleted `extract_number_coerce` numeric-coercion
    // path is forbidden tag_bits dispatch §2.7.7 #4 / #7; the kinded
    // equivalent is `kind_coerce::coerce_to_f64(slot)` from
    // `executor/builtins/kind_coerce.rs`, which requires KindedSlot
    // input — i.e. the kind track), and (3) optional comparator
    // closure dispatch with the same gap as `handle_map_v2`.
    Err(VMError::NotImplemented(
        "sort — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Element comparison needs \
         kind_coerce::coerce_to_f64(KindedSlot) (kinded equivalent of \
         the deleted extract_number_coerce); comparator-closure form \
         additionally needs kinded op_call_value. Phase-2c follow-up: \
         extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_slice_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: slice needs (1) receiver-kind proof + per-`TypedArrayData::*`
    // arm-driven slice (already implemented for the opcode form in
    // `array_operations.rs::slice_typed_array`, but only reachable from
    // the kinded `op_slice_access` path — not from the kind-blind
    // MethodFnV2 ABI), (2) start/end index kinds via the kinded API
    // (the deleted `as_number_coerce` / `as_i64` raw-bits probes were
    // forbidden tag_bits dispatch).
    Err(VMError::NotImplemented(
        "slice — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). The opcode form \
         (op_slice_access in array_operations.rs) already implements the \
         kinded slice; the MethodFnV2 form needs the same kind side- \
         channel for receiver + indices. Phase-2c follow-up: extend \
         MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_concat_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: concat needs (1) per-arg array-receiver-kind proof for
    // every args[i], (2) per-element-kind transfer between source and
    // destination buffers (cross-variant concat is itself a Phase-2c
    // semantic question — what is `[i64...].concat([f64...])` under
    // strict typing?). The deleted `view.to_generic()` + ValueWord
    // re-encoding path was the W-series cover for this.
    Err(VMError::NotImplemented(
        "concat — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Per-arg receiver-kind \
         proof + element-kind transfer between source / dest buffers \
         needed; cross-variant concat semantics under strict typing are \
         themselves Phase-2c. Phase-2c follow-up: extend MethodFnV2 with \
         parallel kind track + cross-TypedArrayData-variant concat \
         contract."
            .to_string(),
    ))
}

pub(crate) fn handle_take_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: take(n) is `slice(0, n)` semantically; same gap as
    // `handle_slice_v2`.
    Err(VMError::NotImplemented(
        "take — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as slice. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_drop_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: drop(n) is `slice(n, len)` semantically; same gap as
    // `handle_slice_v2`.
    Err(VMError::NotImplemented(
        "drop — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as slice. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

pub(crate) fn handle_skip_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // skip is an alias for drop in the pre-Wave-6.5 dispatch table —
    // forward to the same surface to keep the mapping stable.
    handle_drop_v2(vm, args, ctx)
}

pub(crate) fn handle_flatten_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: flatten needs (1) receiver-kind proof, (2) per-element
    // dispatch — each element bit-pattern must be re-classified as
    // either-array-or-scalar, which the deleted `as_any_array` probe
    // covered by walking ValueWord tag bits (forbidden §2.7.7 #4 /
    // #7). Strict-typing equivalent requires a per-element kind on
    // the source array — only reachable when the array is itself a
    // TypedArrayData::HeapValue arm, with each element carrying its
    // own kind via `Arc<HeapValue>`.
    Err(VMError::NotImplemented(
        "flatten — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Per-element \
         array-or-scalar reclassification was the deleted as_any_array \
         tag_bits probe; kinded equivalent needs per-element NativeKind \
         on TypedArrayData::HeapValue. Phase-2c follow-up: extend \
         MethodFnV2 with parallel kind track + per-element-kind metadata \
         on TypedArrayData::HeapValue."
            .to_string(),
    ))
}

pub(crate) fn handle_flat_map_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: combination of `handle_map_v2` and `handle_flatten_v2`
    // gaps — kinded mapper + kinded result-element classification.
    Err(VMError::NotImplemented(
        "flatMap — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Combines map's higher-order \
         dispatch gap with flatten's per-result-element \
         array-or-scalar reclassification gap. Phase-2c follow-up: \
         extend MethodFnV2 with parallel kind track + kinded \
         op_call_value path."
            .to_string(),
    ))
}

pub(crate) fn handle_group_by_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: groupBy needs (1) receiver-kind proof, (2) kinded key
    // function via `op_call_value` (deleted
    // `call_value_immediate_raw` path), (3) per-key bucketing — the
    // deleted `nb_to_string_coerce` was a ValueWord-flavoured key
    // stringifier; the kinded equivalent has no current home. (4) The
    // result HashMap-of-Array construction needs both the key-kind and
    // the element-kind to be threaded through, which the
    // `ValueWord::from_array` / `vmarray_from_vec` carrier APIs no
    // longer provide.
    Err(VMError::NotImplemented(
        "groupBy — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Higher-order op gap + \
         deleted nb_to_string_coerce key-stringifier (was \
         ValueWord-flavoured) + result-construction needs kinded \
         HashMap-of-TypedArray. Phase-2c follow-up: extend MethodFnV2 \
         with parallel kind track + kinded key-stringifier + kinded \
         result-construction path."
            .to_string(),
    ))
}
