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
use shape_runtime::context::ExecutionContext;
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
use shape_value::{KindedSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `innerJoin` — inner join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_inner_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_inner_join_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `leftJoin` — left join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_left_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_left_join_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product)
///
/// args: [left_array, right_array, result_selector_fn]
pub(crate) fn handle_cross_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_cross_join_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}
