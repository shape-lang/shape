//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join
//!
//! ## Wave-δ `MR-array-sort-sets-joins` body migration (ADR-006 §2.7.10 / Q11)
//!
//! Every handler in this file requires the kinded closure-callback path
//! (`call_value_immediate_*` in `executor/call_convention.rs`,
//! `op_call_value` in `executor/control_flow/mod.rs`) for left-key /
//! right-key / result-selector closure invocations, plus per-element
//! kind dispatch on two `TypedArrayData` arms (the cross-array-shape
//! join product). The kinded `MethodFnV2` ABI itself landed (Wave-γ
//! `G-method-fn-v2-abi`) — `args[0..1]` arrive as
//! `KindedSlot { kind: NativeKind::Ptr(HeapKind::TypedArray) }` carriers
//! per ADR-005 §1 single-discriminator dispatch — but the closure-call
//! consumer side is unmigrated:
//!
//! - `executor/call_convention.rs::call_value_immediate_nb` /
//!   `call_value_immediate_raw` / `call_closure_with_nb_args*` /
//!   `call_function_with_raw_args` / `call_closure_with_raw_args` all
//!   contain `todo!("phase-2c — ADR-006 §2.7.8 cluster B-round-2: …
//!   kinded ABI rebuild pending")`.
//! - `executor/control_flow/mod.rs::op_call_value` /
//!   `dispatch_call_closure_like` / `op_make_closure` /
//!   `op_call_foreign` return `NotImplemented(SURFACE: ...
//!   PHASE_2C_CALL_REBUILD_SURFACE)`.
//!
//! The Wave-β `M-datatable` cluster's `executor/objects/datatable_methods/joins.rs`
//! (`handle_inner_join` / `handle_left_join`) flipped its ABI to
//! `&[KindedSlot] → Result<KindedSlot, _>` (commit `eb78699`) but kept
//! the bodies as `NotImplemented(SURFACE: phase-2c body migration)` for
//! the same closure-callback reason. Routing this file's handlers
//! through `datatable_methods::joins` does not unblock anything; the
//! shared blocker is the closure-callback ABI rebuild.
//!
//! Per playbook §7.4 REVISED: surface explicitly with the kind-source +
//! callback-path gap named, never a forbidden-pattern workaround.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::{KindedSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — closure-callback dependency surfaces
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `innerJoin` — inner join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
///
/// **SURFACE — Wave-δ closure-callback dependency.** Body shape would
/// invoke `left_key_fn(elem)` / `right_key_fn(elem)` per element to
/// build a hash-keyed multimap, then iterate matched-key pairs and
/// invoke `result_selector_fn(left_elem, right_elem)` per match,
/// producing a `TypedArrayData::HeapValue` result whose element kind
/// follows the selector's return kind. The closure-callback consumer
/// (`call_value_immediate_*` in `executor/call_convention.rs` and
/// `op_call_value` in `executor/control_flow/mod.rs`) is itself
/// `NotImplemented(SURFACE)` post-§2.7.10 pending the kinded callee
/// dispatch + `&[KindedSlot]` arg-slice rebuild (ADR-006 §2.7.4 /
/// §2.7.8 Phase-2c). Without it the handler cannot invoke the user's
/// closures.
pub(crate) fn handle_inner_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "innerJoin — SURFACE: closure-callback path unmigrated. \
         The kinded MethodFnV2 ABI landed (ADR-006 §2.7.10 / Q11) and \
         `args[0..1]` carry the receiver/right-array as KindedSlot \
         { kind: NativeKind::Ptr(HeapKind::TypedArray) }, but the \
         consumer side `call_value_immediate_*` / `op_call_value` \
         (executor/call_convention.rs, executor/control_flow/mod.rs) \
         still return NotImplemented(SURFACE) pending the kinded \
         callee dispatch + `&[KindedSlot]` arg-slice rebuild per \
         ADR-006 §2.7.4 / §2.7.8. Body shape: hash-keyed multimap from \
         `left_key_fn(elem)` / `right_key_fn(elem)`, then \
         `result_selector_fn(left, right)` per matched pair into a \
         TypedArrayData::HeapValue result. The Wave-β M-datatable \
         `executor/objects/datatable_methods/joins.rs::handle_inner_join` \
         flipped ABI but is itself SURFACE for the same reason; routing \
         through it does not unblock."
            .to_string(),
    ))
}

/// v2 `leftJoin` — left join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
///
/// **SURFACE — same closure-callback dependency as `innerJoin`.**
/// Unmatched-row branch additionally needs an empty-payload sentinel
/// fed to `result_selector_fn` — per Wave-β `M-datatable`'s
/// `handle_left_join` SURFACE the canonical sentinel for typed-object
/// element shapes is `NativeKind::Ptr(HeapKind::TypedObject)` with bits
/// = `Arc::into_raw(Arc<TypedObjectStorage>)` of an empty-schema
/// instance (NOT a Bool-default fallback — that would be the §2.7.7 #9
/// W-series rationalization). For typed-array element shapes the
/// equivalent is per-element-kind: `KindedSlot::from_int(0)` for I64
/// arms, etc. The kind cannot be sourced until the closure-callback
/// rebuild lands and the result-selector's input kinds are observable.
pub(crate) fn handle_left_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "leftJoin — SURFACE: closure-callback path unmigrated. \
         Same blocker as innerJoin (`call_value_immediate_*` / \
         `op_call_value` Phase-2c rebuild per ADR-006 §2.7.4 / §2.7.8). \
         Unmatched-row branch additionally needs a per-element-kind \
         empty-payload sentinel for the result-selector callback (per \
         Wave-β M-datatable handle_left_join SURFACE: \
         NativeKind::Ptr(HeapKind::TypedObject) with empty-schema \
         Arc<TypedObjectStorage> for typed-object element shapes; \
         per-NativeKind zero-payload for scalar arms). Bool-default \
         fallback is forbidden (§2.7.7 #9)."
            .to_string(),
    ))
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product).
///
/// args: [left_array, right_array, result_selector_fn]
///
/// **SURFACE — same closure-callback dependency as `innerJoin`.**
/// Cross-join is the closure-keyed degenerate case: no key extractors,
/// only a result-selector per (left, right) pair. The handler still
/// needs the kinded `op_call_value` path to invoke the selector
/// closure; the same Phase-2c rebuild blocker applies.
pub(crate) fn handle_cross_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "crossJoin — SURFACE: closure-callback path unmigrated. \
         Same blocker as innerJoin / leftJoin (`call_value_immediate_*` / \
         `op_call_value` Phase-2c rebuild per ADR-006 §2.7.4 / §2.7.8). \
         Cross-join body is the no-key-extractor degenerate case — \
         iterate the Cartesian product of left and right TypedArrayData \
         arms and invoke `result_selector_fn(left, right)` per pair into \
         a TypedArrayData::HeapValue result. Same closure-callback \
         consumer blocker."
            .to_string(),
    ))
}
