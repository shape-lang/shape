//! Core DataTable methods: origin, len, columns, column, slice, head, tail,
//! first, last, select, toMat, limit, execute, rows, columnsRef.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! All handlers are placeholders (`NotImplemented(SURFACE)`) per playbook
//! §7.4 REVISED. The pre-Wave-6.5 bodies were keyed on the deleted
//! `ValueWord` carrier and called deleted constructors
//! (`ValueWord::from_datatable`, `from_typed_table`, `from_indexed_table`,
//! `from_row_view`, `from_string`, `from_array`, `from_column_ref`,
//! `vmarray_from_vec`, ...). The kinded re-implementation will pull the
//! receiver via `slot.as_heap_value()` + `HeapValue::DataTable(arc)` /
//! `HeapValue::TableView(tv)` match (ADR-005 §1, Q8) and push results as
//! `Arc::into_raw + push_kinded(bits, NativeKind::Ptr(HeapKind::*))` per
//! playbook §3.

use shape_value::VMError;

use crate::executor::VirtualMachine;

/// Helper: stub body for a method that takes raw u64 args (legacy
/// `MethodFnV2` ABI) and would otherwise need `ValueWord`-shaped helpers
/// to interpret them.
#[inline]
fn stub(name: &str, kind_source: &str) -> VMError {
    VMError::NotImplemented(format!(
        "datatable.{} — SURFACE: phase-2c body migration. Receiver is \
         NativeKind::Ptr(HeapKind::DataTable) (or Ptr(HeapKind::TableView) \
         for typed/indexed variants); body re-shape requires \
         slot.as_heap_value() + HeapValue::DataTable / HeapValue::TableView \
         match per ADR-005 §1 and result push as Arc::into_raw + push_kinded \
         per playbook §3 ({}).",
        name, kind_source
    ))
}

/// `dt.origin()` — returns the table origin string.
pub(crate) fn handle_origin(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("origin", "result kind = NativeKind::String"))
}

/// `dt.len()` — returns the row count as Int64.
pub(crate) fn handle_len(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("len", "result kind = NativeKind::Int64"))
}

/// `dt.columns()` — returns an Array<String> of column names.
pub(crate) fn handle_columns(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "columns",
        "result kind = NativeKind::Ptr(HeapKind::TypedArray) of String element kind",
    ))
}

/// `dt.column(name)` — returns a ColumnRef (TableView variant).
pub(crate) fn handle_column(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "column",
        "result kind = NativeKind::Ptr(HeapKind::TableView) (ColumnRef variant)",
    ))
}

/// `dt.slice(offset, length)` — returns a sliced DataTable.
pub(crate) fn handle_slice(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("slice", "result kind = receiver kind"))
}

/// `dt.head(n)` — first n rows.
pub(crate) fn handle_head(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("head", "result kind = receiver kind"))
}

/// `dt.tail(n)` — last n rows.
pub(crate) fn handle_tail(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("tail", "result kind = receiver kind"))
}

/// `dt.first()` — first row as RowView.
pub(crate) fn handle_first(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "first",
        "result kind = NativeKind::Ptr(HeapKind::TableView) (RowView variant)",
    ))
}

/// `dt.last()` — last row as RowView.
pub(crate) fn handle_last(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "last",
        "result kind = NativeKind::Ptr(HeapKind::TableView) (RowView variant)",
    ))
}

/// `dt.select(col_names...)` — projection.
pub(crate) fn handle_select(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("select", "result kind = receiver kind"))
}

/// `dt.toMat()` — convert to Array<Array<f64>>.
pub(crate) fn handle_to_mat(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "toMat",
        "result kind = NativeKind::Ptr(HeapKind::TypedArray) of TypedArray<f64> element kind",
    ))
}

/// `dt.limit(n)` — alias for take-first-n.
pub(crate) fn handle_limit(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("limit", "result kind = receiver kind"))
}

/// `dt.execute()` — terminal Queryable adapter.
pub(crate) fn handle_execute(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("execute", "result kind = receiver kind"))
}

/// `dt.rows()` — Array<RowView>.
pub(crate) fn handle_rows(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "rows",
        "result kind = NativeKind::Ptr(HeapKind::TypedArray) of TableView element kind",
    ))
}

/// `dt.columnsRef()` — Array<ColumnRef>.
pub(crate) fn handle_columns_ref(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "columnsRef",
        "result kind = NativeKind::Ptr(HeapKind::TypedArray) of TableView (ColumnRef) element kind",
    ))
}
