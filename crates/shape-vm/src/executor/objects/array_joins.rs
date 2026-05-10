//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join
//!
//! ## Wave-δ `MR-array-sort-sets-joins` body migration (ADR-006 §2.7.10 / Q11)
//!
//! Every handler in this file requires the kinded value-call path
//! (`call_value_immediate_nb` in `executor/call_convention.rs`,
//! `op_call_value` / `dispatch_call_value_immediate` in
//! `executor/control_flow/mod.rs`) for left-key / right-key /
//! result-selector closure invocations, plus per-element kind dispatch
//! on two `TypedArrayData` arms (the cross-array-shape join product).
//! The kinded `MethodFnV2` ABI landed in Wave-γ `G-method-fn-v2-abi` —
//! `args[0..1]` arrive as
//! `KindedSlot { kind: NativeKind::Ptr(HeapKind::TypedArray) }` carriers
//! per ADR-005 §1 single-discriminator dispatch.
//!
//! Wave 7 (W7-cv-static / W7-cv-method / W7-op-call-value) closed the
//! kinded value-call ABI per ADR-006 §2.7.11 / Q12:
//! `call_value_immediate_nb(callee: &KindedSlot, args: &[KindedSlot]) ->
//!  Result<KindedSlot, VMError>` is live in `call_convention.rs:767`,
//! `op_call_value` and `dispatch_call_value_immediate` are filled, and
//! the closure-self carrier integrates `closure_heap_kind: Option<NativeKind>`
//! per the §2.7.8/Q10 lockstep. The remaining upstream gate for *user-
//! produced* closures is `op_make_closure` in
//! `executor/control_flow/mod.rs:447`, still
//! `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)` pending the kinded
//! capture-read + closure-block construction rebuild (ADR-006 §2.7.4 /
//! §2.7.5 / §2.7.8). Without `op_make_closure`, no user closure
//! `KindedSlot { kind: NativeKind::Ptr(HeapKind::Closure) }` carrier
//! reaches a join handler's arg slice; filling the body here would
//! produce dead code rather than an end-to-end working dispatch path.
//!
//! The Wave-β `M-datatable` cluster's `executor/objects/datatable_methods/joins.rs`
//! (`handle_inner_join` / `handle_left_join`) flipped its ABI to
//! `&[KindedSlot] → Result<KindedSlot, _>` (commit `eb78699`) but kept
//! the bodies as `NotImplemented(SURFACE: phase-2c body migration)` for
//! the same upstream `op_make_closure` reason. Routing this file's
//! handlers through `datatable_methods::joins` does not unblock; the
//! shared blocker is closure construction.
//!
//! Per playbook §7.4 REVISED: surface explicitly with the upstream
//! `op_make_closure` gate named, never a forbidden-pattern workaround.

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
/// **SURFACE — `op_make_closure` upstream gate.** Body shape would
/// invoke `left_key_fn(elem)` / `right_key_fn(elem)` per element to
/// build a hash-keyed multimap, then iterate matched-key pairs and
/// invoke `result_selector_fn(left_elem, right_elem)` per match,
/// producing a `TypedArrayData::HeapValue` result whose element kind
/// follows the selector's return kind. The kinded value-call path
/// (`call_value_immediate_nb` in `call_convention.rs:767`,
/// `dispatch_call_value_immediate` in `control_flow/mod.rs:389`) is
/// live post-W7 (ADR-006 §2.7.11 / Q12), but the upstream
/// `op_make_closure` (`control_flow/mod.rs:447`) is itself
/// `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)` pending the kinded
/// capture-read + closure-block construction rebuild (ADR-006 §2.7.4 /
/// §2.7.5 / §2.7.8). Without it no user closure `KindedSlot` carrier
/// reaches `args[2..]`.
pub(crate) fn handle_inner_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "innerJoin — SURFACE: op_make_closure upstream gate. \
         The kinded MethodFnV2 ABI landed (ADR-006 §2.7.10 / Q11) and \
         `args[0..1]` carry the receiver/right-array as KindedSlot \
         { kind: NativeKind::Ptr(HeapKind::TypedArray) }; the kinded \
         value-call path `call_value_immediate_nb` / \
         `dispatch_call_value_immediate` is live post-W7 (ADR-006 \
         §2.7.11 / Q12). The upstream gate is `op_make_closure` in \
         executor/control_flow/mod.rs:447, still \
         NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE) per ADR-006 \
         §2.7.4 / §2.7.5 / §2.7.8 — without it no user closure \
         KindedSlot reaches args[2..]. Body shape: hash-keyed multimap \
         from `left_key_fn(elem)` / `right_key_fn(elem)`, then \
         `result_selector_fn(left, right)` per matched pair into a \
         TypedArrayData::HeapValue result. The Wave-β M-datatable \
         `executor/objects/datatable_methods/joins.rs::handle_inner_join` \
         is SURFACE for the same upstream reason."
            .to_string(),
    ))
}

/// v2 `leftJoin` — left join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
///
/// **SURFACE — same `op_make_closure` upstream gate as `innerJoin`.**
/// Unmatched-row branch additionally needs an empty-payload sentinel
/// fed to `result_selector_fn` — per Wave-β `M-datatable`'s
/// `handle_left_join` SURFACE the canonical sentinel for typed-object
/// element shapes is `NativeKind::Ptr(HeapKind::TypedObject)` with bits
/// = `Arc::into_raw(Arc<TypedObjectStorage>)` of an empty-schema
/// instance (NOT a Bool-default fallback — that would be the §2.7.7 #9
/// W-series rationalization). For typed-array element shapes the
/// equivalent is per-element-kind: `KindedSlot::from_int(0)` for I64
/// arms, etc. The kind cannot be sourced until `op_make_closure` lands
/// and the result-selector's input kinds are observable.
pub(crate) fn handle_left_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "leftJoin — SURFACE: op_make_closure upstream gate. \
         Same upstream gate as innerJoin (`op_make_closure` in \
         executor/control_flow/mod.rs:447 NotImplemented(PHASE_2C_CALL_\
         REBUILD_SURFACE) per ADR-006 §2.7.4 / §2.7.5 / §2.7.8). \
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
/// **SURFACE — same `op_make_closure` upstream gate as `innerJoin`.**
/// Cross-join is the closure-keyed degenerate case: no key extractors,
/// only a result-selector per (left, right) pair. The handler still
/// needs a user closure `KindedSlot` carrier in `args[2]` to invoke
/// the selector via the live `call_value_immediate_nb`; the upstream
/// `op_make_closure` gate is the same.
pub(crate) fn handle_cross_join_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "crossJoin — SURFACE: op_make_closure upstream gate. \
         Same upstream gate as innerJoin / leftJoin (`op_make_closure` \
         in executor/control_flow/mod.rs:447 NotImplemented(PHASE_2C_\
         CALL_REBUILD_SURFACE) per ADR-006 §2.7.4 / §2.7.5 / §2.7.8). \
         Cross-join body is the no-key-extractor degenerate case — \
         iterate the Cartesian product of left and right TypedArrayData \
         arms and invoke `result_selector_fn(left, right)` per pair via \
         the live `call_value_immediate_nb` (W7 / §2.7.11) into a \
         TypedArrayData::HeapValue result."
            .to_string(),
    ))
}
