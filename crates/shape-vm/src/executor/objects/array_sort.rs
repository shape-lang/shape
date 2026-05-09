//! Array sort operations
//!
//! Handles: order_by, then_by, join_str
//!
//! Wave-β `A-array-sort-sets` sub-cluster (ADR-006 §2.7.6 / §2.7.7 /
//! §2.7.8 / Q8-Q10): the `MethodFnV2` native ABI takes `args: &mut [u64]`
//! raw bits with **no parallel `NativeKind` track**. Every interaction
//! these handlers need — interpreting `args[i]` as an array vs. closure
//! vs. string, iterating `TypedArrayData` element-by-element with a
//! per-element kind, comparing/keying values across heterogeneous
//! element kinds, calling back into `op_call_value` with kinded callee
//! + arg + count slots — is impossible without a kind-aware extension
//! to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted
//!     tag_bits dispatch, §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op"
//!     (forbidden §2.7.7 #9 — the W-series rationalization the
//!     playbook names verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7).
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

/// v2 `orderBy` — sort an array by a key function (optionally with direction)
///
/// args: [array, key_fn, direction?]
pub(crate) fn handle_order_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_order_by_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `thenBy` — sort an already-ordered array by a secondary key
///
/// args: [array, key_fn, direction?]
pub(crate) fn handle_then_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_then_by_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// v2 `join` — join array elements into a single string with a separator
///
/// args: [array, separator?]
pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_join_str_v2 — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}
