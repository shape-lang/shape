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
use shape_runtime::context::ExecutionContext;
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
use shape_value::{KindedSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — kind-blind ABI, all surfaces
// ═══════════════════════════════════════════════════════════════════════════

// Pre-§2.7.9 surface helpers (`surface_v2_abi_gap`, `surface_closure_dispatch_gap`)
// deleted with their callers' bodies — the kinded ABI landed per
// ADR-006 §2.7.9 / Q11, so the previous "MethodFnV2 ABI lacks parallel
// NativeKind track" gap framing is stale. Each SURFACE handler body
// carries the §2.7.9-aware migration contract inline. Wave-γ-followup
// body migration territory (D-array-query-class).

pub(crate) fn handle_where_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_where_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_select_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_select_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_find_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_find_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_find_index_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_find_index_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_index_of_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_includes_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_some_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_some_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_every_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_every_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_any_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_any_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_all_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_all_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_single_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_single_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_take_while_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_take_while_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_skip_while_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_skip_while_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

pub(crate) fn handle_for_each_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_for_each_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}
