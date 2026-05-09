//! DataTable join methods: innerJoin, leftJoin.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! ABI: handlers take `args: &[KindedSlot]` (kinded carrier per Q7) and
//! return `Result<KindedSlot, VMError>` so the caller side
//! (`window_join.rs` `handle_join_execute`) can dispatch without
//! kind-stripping at the boundary. The legacy `&mut [u64]` MethodFnV2
//! shape was kind-blind by construction (CLAUDE.md "Forbidden Patterns",
//! the deleted ValueWord-bits carrier). The flip is the cross-cluster
//! cascade closure surfaced from D-window-join Wave-α.
//!
//! Bodies are placeholders (`NotImplemented(SURFACE)`) per playbook §7.4
//! REVISED: closure callbacks for key extraction and result selection
//! must be re-shaped to thread `KindedSlot` argument lists through
//! `op_call_value`, equality comparison on join keys must dispatch on
//! slot kinds (no `as_number_coerce` on a deleted carrier), and the
//! result table must be assembled via per-slot
//! `Arc::into_raw + NativeKind::Ptr(HeapKind::DataTable)` per playbook §3.
//! The legacy bodies relied on `ValueWord::from_row_view`, `ArgVec`,
//! `vw_clone`, `is_heap()`, `as_typed_object()`, and friends — all
//! deleted with the dynamic-fallback path.

use shape_value::{KindedSlot, VMError};

use crate::executor::VirtualMachine;

/// `dt.innerJoin(other, leftKey, rightKey, resultSelector)` — kinded ABI.
///
/// Args (post-receiver-on-stack convention):
///   `args[0]` = receiver `NativeKind::Ptr(HeapKind::DataTable)` (or
///       `Ptr(HeapKind::TableView)` for typed/indexed variants),
///   `args[1]` = right table (same kind family as receiver),
///   `args[2]` = left key closure (`Ptr(HeapKind::Closure)` /
///       `Ptr(HeapKind::Future)` — see playbook §3 callable family),
///   `args[3]` = right key closure (same shape),
///   `args[4]` = result selector closure (same shape).
///
/// Returns: `KindedSlot { kind: NativeKind::Ptr(HeapKind::DataTable),
///                        slot: ValueSlot::from_data_table(Arc<DataTable>) }`.
pub(crate) fn handle_inner_join(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "innerJoin — SURFACE: phase-2c body migration. Kinded ABI flip \
         landed (M-datatable Wave-β closes the D-window-join cross-cluster \
         cascade); body re-shape requires (1) receiver kind dispatch via \
         slot.as_heap_value() + HeapValue::DataTable / HeapValue::TableView \
         match per ADR-005 §1, (2) per-row RowView construction as kinded \
         slots (replaces deleted `ValueWord::from_row_view`), (3) closure \
         callback through op_call_value with kinded arg lists (replaces \
         deleted `call_value_immediate_raw`), (4) join-key equality via \
         per-kind dispatch (replaces deleted `as_number_coerce` on the \
         legacy ValueWord carrier), (5) result table assembly via \
         `Arc::into_raw(Arc<DataTable>) + push_kinded(bits, \
         NativeKind::Ptr(HeapKind::DataTable))` per playbook §3."
            .to_string(),
    ))
}

/// `dt.leftJoin(other, leftKey, rightKey, resultSelector)` — kinded ABI.
///
/// Args identical to `handle_inner_join`. Unmatched-row branch must
/// supply a kinded `none` slot to the result selector — per ADR-006
/// §2.7 the canonical sentinel is `(0u64, NativeKind::Bool)` only when
/// the kind is statically known to be the §2.7 sentinel; for an empty
/// `TypedObject` payload the correct kind is
/// `NativeKind::Ptr(HeapKind::TypedObject)` with bits =
/// `Arc::into_raw(Arc<TypedObjectStorage>)` of an empty-schema instance.
///
/// Returns: `KindedSlot { kind: NativeKind::Ptr(HeapKind::DataTable),
///                        slot: ValueSlot::from_data_table(Arc<DataTable>) }`.
pub(crate) fn handle_left_join(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "leftJoin — SURFACE: phase-2c body migration. Same kinded ABI as \
         innerJoin (M-datatable Wave-β); unmatched-row branch additionally \
         needs an empty-TypedObject argument to the result selector with \
         kind = NativeKind::Ptr(HeapKind::TypedObject) and bits = \
         Arc::into_raw(Arc<TypedObjectStorage>) of an empty-schema instance \
         (NOT a Bool-default forbidden by §2.7.7 #9). Closure dispatch and \
         result table assembly identical to innerJoin."
            .to_string(),
    ))
}
