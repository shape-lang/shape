//! DataTable query methods: filter, orderBy, group_by, forEach, map.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! Bodies are placeholders (`NotImplemented(SURFACE)`) per playbook §7.4
//! REVISED. Each handler in this file's pre-Wave-6.5 incarnation
//! closure-dispatched per row through the deleted
//! `call_value_immediate_raw` raw-bits API and built result tables via
//! deleted `ValueWord::from_row_view` / `ArgVec` / `vmarray_from_vec`
//! constructors. The kinded re-implementation threads `KindedSlot`
//! argument lists into `op_call_value` and assembles the result via
//! `Arc::into_raw + push_kinded(bits, NativeKind::Ptr(HeapKind::DataTable))`
//! per playbook §3.

use shape_value::VMError;

use crate::executor::VirtualMachine;

#[inline]
fn stub(name: &str, kind_source: &str) -> VMError {
    VMError::NotImplemented(format!(
        "datatable.{} — SURFACE: phase-2c body migration. Receiver kind = \
         NativeKind::Ptr(HeapKind::DataTable) (or Ptr(HeapKind::TableView) \
         for typed/indexed variants); body re-shape requires kinded receiver \
         dispatch via slot.as_heap_value() + HeapValue::DataTable / \
         HeapValue::TableView match per ADR-005 §1, kinded closure callback \
         (replaces deleted `call_value_immediate_raw`), and result push via \
         Arc::into_raw + push_kinded per playbook §3 ({}).",
        name, kind_source
    ))
}

/// `dt.filter(closure)` / `dt.filter(col, op, value)` — row filter.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("filter", "result kind = receiver kind"))
}

/// `dt.orderBy(closure)` / `dt.orderBy(col, asc?)`.
pub(crate) fn handle_order_by(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("orderBy", "result kind = receiver kind"))
}

/// `dt.group_by(col)` / `dt.group_by(col, agg_spec)`.
pub(crate) fn handle_group_by(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "group_by",
        "result kind = NativeKind::Ptr(HeapKind::DataTable); aggregate spec \
         arg dispatch on Ptr(HeapKind::TypedObject)",
    ))
}

/// `dt.forEach(closure)` — side-effect each row, returns receiver.
pub(crate) fn handle_for_each(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("forEach", "result kind = receiver kind"))
}

/// `dt.map(closure)` — per-row transformation, returns Array<TypedObject>.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "map",
        "result kind = NativeKind::Ptr(HeapKind::TypedArray) (closure return \
         type drives element kind)",
    ))
}
