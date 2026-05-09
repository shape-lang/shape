//! Array set operations
//!
//! Handles: union, intersect, except, unique, distinct, distinct_by
//!
//! Wave-β `A-array-sort-sets` sub-cluster (ADR-006 §2.7.6 / §2.7.7 /
//! §2.7.8 / Q8-Q10): the `MethodFnV2` native ABI takes `args: &mut [u64]`
//! raw bits with **no parallel `NativeKind` track**. Every interaction
//! these handlers need — interpreting `args[i]` as an array vs.
//! closure, iterating `TypedArrayData` element-by-element with a
//! per-element kind, equality-comparing values across heterogeneous
//! element kinds, retaining heap shares into a result buffer, calling
//! back into `op_call_value` with kinded callee + arg + count slots —
//! is impossible without a kind-aware extension to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted
//!     tag_bits dispatch, §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op"
//!     (forbidden §2.7.7 #9 — the W-series rationalization the
//!     playbook names verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7), or
//!   - reaching for `vw_clone(bits)` / `vw_drop(bits)` (forbidden
//!     §2.7.7 #8 — replaced by `clone_with_kind` / `drop_with_kind`
//!     which require a kinded slot).
//!
//! This is the same architectural ABI gap that `D-array-joins` surfaced
//! at close (`2fe4a6b`); the Phase-2c follow-up there — extend
//! `MethodFnV2` to a kind-aware `args: &mut [(u64, NativeKind)]` (or
//! parallel-track equivalent) — also unblocks this file. Per playbook
//! §7.4 (REVISED) the correct refusal shape is `NotImplemented(SURFACE:
//! ...)` with the kind-source gap named explicitly, never a forbidden-
//! pattern workaround.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::{KindedSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `union` — set union of two arrays (deduplicated, order-preserving)
///
/// args: [array, other_array]
pub(crate) fn handle_union_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_union_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `intersect` — set intersection of two arrays (deduplicated, order-preserving)
///
/// args: [array, other_array]
pub(crate) fn handle_intersect_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_intersect_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `except` — set difference of two arrays (deduplicated, order-preserving)
///
/// args: [array, other_array]
pub(crate) fn handle_except_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_except_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `unique` — deduplicate array elements (order-preserving)
///
/// args: [array]
pub(crate) fn handle_unique_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_unique_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `distinct` — alias for `unique`
///
/// args: [array]
pub(crate) fn handle_distinct_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_distinct_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `distinctBy` — deduplicate by a key function (order-preserving)
///
/// args: [array, key_fn]
pub(crate) fn handle_distinct_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_distinct_by_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}
