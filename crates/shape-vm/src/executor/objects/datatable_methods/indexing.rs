//! DataTable indexing methods: index_by.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! `index_by(col)` produces a `TableViewData::IndexedTable` view keyed
//! by the named column. The result kind is
//! `NativeKind::Ptr(HeapKind::TableView)` per playbook §3 push pattern.
//! The pre-Wave-6.5 `HeapValue::IndexedTable` variant was retired in the
//! ADR-006 §2.3 trim — the canonical replacement is the
//! `IndexedTable { schema_id, table, index_col }` arm of
//! `TableViewData`.

use shape_runtime::context::ExecutionContext;
use shape_value::{
    KindedSlot, NativeKind, TableViewData, ValueSlot, VMError, heap_value::HeapKind,
};
use std::sync::Arc;

use crate::executor::VirtualMachine;

/// `dt.index_by(col)` — produce an `IndexedTable` view keyed by `col`.
pub(crate) fn handle_index_by(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "datatable.index_by: missing receiver".to_string(),
        ));
    }
    let recv = &args[0];
    let bits = recv.slot.raw();
    if bits == 0 {
        return Err(VMError::RuntimeError(
            "datatable.index_by: null receiver".to_string(),
        ));
    }
    // Index-by requires the bare DataTable variant: returning a
    // double-wrapped TableView from a TableView receiver would alias
    // schemas. Surface the kind hint clearly.
    let table = match recv.kind {
        NativeKind::Ptr(HeapKind::DataTable) => {
            // SAFETY: §2.7.6 / Q8 contract.
            unsafe {
                Arc::increment_strong_count(bits as *const shape_value::DataTable);
                Arc::from_raw(bits as *const shape_value::DataTable)
            }
        }
        NativeKind::Ptr(HeapKind::TableView) => {
            // SAFETY: §2.7.6 / Q8 contract.
            let tv: &TableViewData = unsafe { &*(bits as *const TableViewData) };
            match tv {
                TableViewData::TypedTable { table, .. }
                | TableViewData::IndexedTable { table, .. }
                | TableViewData::RowView { table, .. }
                | TableViewData::ColumnRef { table, .. } => Arc::clone(table),
            }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "datatable.index_by: expected DataTable/TableView receiver, got {:?}",
                other
            )));
        }
    };

    let col_slot = args.get(1).ok_or_else(|| {
        VMError::RuntimeError("datatable.index_by: missing column-name arg".to_string())
    })?;
    let col_name = col_slot.as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.index_by: column-name arg must be string, got {:?}",
            col_slot.kind
        ))
    })?;
    let col_id = table
        .column_names()
        .iter()
        .position(|n| n == col_name)
        .ok_or_else(|| {
            VMError::RuntimeError(format!("datatable.index_by: unknown column: {}", col_name))
        })? as u32;

    let view = TableViewData::IndexedTable {
        schema_id: table.schema_id().unwrap_or(0) as u64,
        table,
        index_col: col_id,
    };
    let out_bits = Arc::into_raw(Arc::new(view)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(out_bits),
        NativeKind::Ptr(HeapKind::TableView),
    ))
}
