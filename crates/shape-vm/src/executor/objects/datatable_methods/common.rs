//! Shared helpers for DataTable method handlers.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! All helpers here take `args: &[KindedSlot]` (the §2.7.10 dispatch-slice
//! carrier) and dispatch on `args[0].kind` per §2.7.6 / Q8. For
//! `Ptr(HeapKind::DataTable)` the slot bits are
//! `Arc::into_raw::<DataTable>` per playbook §3; for
//! `Ptr(HeapKind::TableView)` the slot bits are
//! `Arc::into_raw::<TableViewData>`. Both are typed-Arc payloads, so
//! `args[0].slot.as_heap_value()` (which reinterprets bits as
//! `*const HeapValue` — the deleted Box-wrap shape) is unsound; the
//! correct dispatch is direct `unsafe { &*(bits as *const T) }` per the
//! Wave-α D-window-join `exec_bind_schema` precedent
//! (`executor/window_join.rs` lines 197-238) and the Wave-γ
//! G-heap-filter-expr soundness amendment (`HeapKind::FilterExpr` arm).
//!
//! Pre-Wave-6.5 helpers (`extract_dt_nb`, `wrap_result_table_nb`,
//! `extract_array_value_nb`, `typed_object_entries_nb_vm`,
//! `build_datatable_from_objects_nb`, `extract_col_nb`,
//! `extract_indexed_table_nb`, `collect_closure_numbers_nb`,
//! `cmp_nb_values`, `apply_comparison_nb`, `array_values_equal`,
//! `append_f64_column`, `typed_object_to_hashmap_nb_vm`) were keyed on
//! the deleted `ValueWord` carrier and are gone with their callers'
//! pre-§2.7.10 bodies. The shapes that survive — `borrow_data_table`,
//! `push_data_table_result` — operate on the §2.7.10 carrier directly.

use shape_value::{DataTable, KindedSlot, NativeKind, TableViewData, ValueSlot, VMError};
use shape_value::heap_value::HeapKind;
use std::sync::Arc;

/// Borrow the receiver `DataTable` from `args[0]` without consuming the
/// carrier's strong-count share. For `TableView` receivers, follows the
/// inner `Arc<DataTable>` field to surface the underlying table.
///
/// Returns a `&DataTable` whose lifetime is bounded by `args` (the
/// dispatch shell owns the share for the call duration). This avoids
/// the refcount churn of `Arc::clone`-on-borrow when the body just
/// needs to read the table; for handlers that need to reseat the
/// `Arc<DataTable>` into a new `TableViewData`, use the borrow-and-clone
/// pattern at the call site (see `core.rs::borrow_data_table_arc`).
///
/// SAFETY: relies on the §2.7.6 / Q8 construction-side contract — when
/// `args[0].kind == NativeKind::Ptr(HeapKind::DataTable)`, the slot
/// bits are `Arc::into_raw::<DataTable>`; when the kind is
/// `NativeKind::Ptr(HeapKind::TableView)`, the slot bits are
/// `Arc::into_raw::<TableViewData>`. Type confusion is the soundness
/// hazard the Wave-γ G-heap-filter-expr amendment closed.
pub(super) fn borrow_data_table<'a>(
    args: &'a [KindedSlot],
    method: &str,
) -> Result<&'a DataTable, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: missing receiver",
            method
        )));
    }
    let recv = &args[0];
    let bits = recv.slot.raw();
    if bits == 0 {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: null receiver",
            method
        )));
    }
    match recv.kind {
        NativeKind::Ptr(HeapKind::DataTable) => {
            // SAFETY: §2.7.6 / Q8 construction-side contract.
            let dt: &DataTable = unsafe { &*(bits as *const DataTable) };
            Ok(dt)
        }
        NativeKind::Ptr(HeapKind::TableView) => {
            // SAFETY: same contract; TableView payload is Arc<TableViewData>.
            let tv: &TableViewData = unsafe { &*(bits as *const TableViewData) };
            let table_ref: &DataTable = match tv {
                TableViewData::TypedTable { table, .. }
                | TableViewData::IndexedTable { table, .. }
                | TableViewData::RowView { table, .. }
                | TableViewData::ColumnRef { table, .. } => table.as_ref(),
            };
            Ok(table_ref)
        }
        other => Err(VMError::RuntimeError(format!(
            "datatable.{}: expected DataTable/TableView receiver, got {:?}",
            method, other
        ))),
    }
}

/// Wrap a freshly-built `DataTable` into a result `KindedSlot` carrying
/// the `NativeKind::Ptr(HeapKind::DataTable)` discriminator. The new
/// `Arc<DataTable>`'s sole strong-count share moves into the slot per
/// playbook §3 push pattern; the dispatch shell takes ownership and
/// pushes the bits onto the kinded stack.
pub(super) fn push_data_table_result(dt: DataTable) -> Result<KindedSlot, VMError> {
    let bits = Arc::into_raw(Arc::new(dt)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}
