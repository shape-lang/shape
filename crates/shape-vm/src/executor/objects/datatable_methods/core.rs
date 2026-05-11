//! Core DataTable methods: origin, len, columns, column, slice, head, tail,
//! first, last, select, toMat, limit, execute, rows, columnsRef.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! ABI: `args[0]` is the receiver — `NativeKind::Ptr(HeapKind::DataTable)`
//! (or `Ptr(HeapKind::TableView)` for the typed/indexed/row-view/column-ref
//! variants). Per-arg kinds come from the §2.7.7 stack parallel-`Vec<NativeKind>`
//! track at the dispatch boundary; `args` is borrow-only — handlers do not
//! consume any share.
//!
//! Body pattern: borrow the receiver Arc payload via
//! `unsafe { &*(args[0].slot.raw() as *const DataTable) }` (and
//! `*const TableViewData` for `TableView` receivers) — soundness rests on
//! the §2.7.6 / Q8 construction-side contract that each `Ptr(HeapKind::*)`
//! kind carries the result of `Arc::into_raw::<T>` for the matching `T`.
//! `args[0].slot.as_heap_value()` is unsound on typed-Arc slots (it
//! reinterprets the bits as `*const HeapValue`, the deleted Box-wrap
//! shape — see Wave-γ G-heap-filter-expr soundness fix at
//! `Arc::increment_strong_count::<FilterNode>`); the typed-Arc dispatch
//! pattern lives in `executor/window_join.rs::exec_bind_schema` (Wave-α
//! D-window-join precedent).
//!
//! Result construction:
//!   - DataTable result: wrap in `Arc::new`, `Arc::into_raw` to the slot,
//!     push as `NativeKind::Ptr(HeapKind::DataTable)` per playbook §3.
//!   - TableView result: same pattern with `Arc<TableViewData>` and
//!     `NativeKind::Ptr(HeapKind::TableView)`.
//!   - Scalar result: `KindedSlot::from_int` / `from_number` / `from_bool`.
//!   - String result: `KindedSlot::from_string_arc(Arc<String>)`.
//!   - Array result: `KindedSlot::from_typed_array(Arc<TypedArrayData>)`.
//!
//! Closure-callback handlers (filter / orderBy / group_by / map / forEach /
//! aggregate-with-spec) live in `query.rs` / `aggregation.rs` — they
//! surface because `op_call_value` itself is at SURFACE per
//! `executor/control_flow/mod.rs:372` (PHASE_2C_CALL_REBUILD_SURFACE).

use shape_runtime::context::ExecutionContext;
use shape_value::{
    AlignedTypedBuffer, AlignedVec, DataTable, HeapValue, KindedSlot, NativeKind, TableViewData,
    TypedArrayData, TypedBuffer, ValueSlot, VMError, heap_value::HeapKind,
};
use std::sync::Arc;

use crate::executor::VirtualMachine;

use super::common::{borrow_data_table, push_data_table_result};

/// `dt.origin()` — returns the table origin string (`type_name`, falling
/// back to "DataTable").
pub(crate) fn handle_origin(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "origin")?;
    let name = dt.type_name().unwrap_or("DataTable").to_string();
    Ok(KindedSlot::from_string_arc(Arc::new(name)))
}

/// `dt.len()` — returns the row count as `Int64`.
pub(crate) fn handle_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "len")?;
    Ok(KindedSlot::from_int(dt.row_count() as i64))
}

/// `dt.columns()` — returns an `Array<String>` of column names.
pub(crate) fn handle_columns(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "columns")?;
    let names: Vec<Arc<String>> = dt.column_names().into_iter().map(Arc::new).collect();
    let buf = TypedBuffer::from_vec(names);
    Ok(KindedSlot::from_typed_array(Arc::new(TypedArrayData::String(
        Arc::new(buf),
    ))))
}

/// `dt.column(name)` — returns a `ColumnRef` (`TableView` variant).
pub(crate) fn handle_column(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "column")?;
    let name = arg_str(args, 1, "column", "column name")?;
    let col_id = dt_arc
        .column_names()
        .iter()
        .position(|n| n == name)
        .ok_or_else(|| VMError::RuntimeError(format!("column not found: {}", name)))?;
    let tv = TableViewData::ColumnRef {
        schema_id: dt_arc.schema_id().unwrap_or(0) as u64,
        table: dt_arc,
        col_id: col_id as u32,
    };
    let bits = Arc::into_raw(Arc::new(tv)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::TableView),
    ))
}

/// `dt.slice(offset, length)` — zero-copy sliced DataTable.
pub(crate) fn handle_slice(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "slice")?;
    let offset = arg_usize(args, 1, "slice", "offset")?;
    let length = arg_usize(args, 2, "slice", "length")?;
    let row_count = dt.row_count();
    if offset > row_count {
        return Err(VMError::RuntimeError(format!(
            "slice: offset {} out of range (row_count={})",
            offset, row_count
        )));
    }
    let length = length.min(row_count - offset);
    let sliced = dt.slice(offset, length);
    push_data_table_result(sliced)
}

/// `dt.head(n)` — first `n` rows.
pub(crate) fn handle_head(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "head")?;
    let n = arg_usize(args, 1, "head", "n")?;
    let n = n.min(dt.row_count());
    push_data_table_result(dt.slice(0, n))
}

/// `dt.tail(n)` — last `n` rows.
pub(crate) fn handle_tail(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "tail")?;
    let n = arg_usize(args, 1, "tail", "n")?;
    let row_count = dt.row_count();
    let n = n.min(row_count);
    let offset = row_count - n;
    push_data_table_result(dt.slice(offset, n))
}

/// `dt.first()` — first row as `RowView`.
pub(crate) fn handle_first(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "first")?;
    if dt_arc.row_count() == 0 {
        return Err(VMError::RuntimeError("first: empty table".to_string()));
    }
    let tv = TableViewData::RowView {
        schema_id: dt_arc.schema_id().unwrap_or(0) as u64,
        table: dt_arc,
        row_idx: 0,
    };
    let bits = Arc::into_raw(Arc::new(tv)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::TableView),
    ))
}

/// `dt.last()` — last row as `RowView`.
pub(crate) fn handle_last(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "last")?;
    let row_count = dt_arc.row_count();
    if row_count == 0 {
        return Err(VMError::RuntimeError("last: empty table".to_string()));
    }
    let tv = TableViewData::RowView {
        schema_id: dt_arc.schema_id().unwrap_or(0) as u64,
        table: dt_arc,
        row_idx: row_count - 1,
    };
    let bits = Arc::into_raw(Arc::new(tv)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::TableView),
    ))
}

/// `dt.select(col_names...)` — projection. Variadic column-name args.
pub(crate) fn handle_select(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "select")?;
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "select: at least one column name required".to_string(),
        ));
    }
    let mut indices: Vec<usize> = Vec::with_capacity(args.len() - 1);
    let names = dt.column_names();
    for (i, slot) in args[1..].iter().enumerate() {
        let name = slot.as_str().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "select: arg {} must be a column-name string, got {:?}",
                i + 1,
                slot.kind
            ))
        })?;
        let idx = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| VMError::RuntimeError(format!("select: unknown column: {}", name)))?;
        indices.push(idx);
    }
    use arrow_schema::{Field, Schema};
    let inner = dt.inner();
    let projected_fields: Vec<Field> = indices
        .iter()
        .map(|&i| inner.schema().field(i).clone())
        .collect();
    let projected_cols = indices
        .iter()
        .map(|&i| inner.column(i).clone())
        .collect::<Vec<_>>();
    let new_schema = Arc::new(Schema::new(projected_fields));
    let new_batch = arrow_array::RecordBatch::try_new(new_schema, projected_cols)
        .map_err(|e| VMError::RuntimeError(format!("select: {}", e)))?;
    push_data_table_result(DataTable::new(new_batch))
}

/// `dt.toMat()` — convert to `Array<Array<f64>>`. Each row becomes a
/// row-vector; per-column kinds widen to `f64` (Float64 / Int64 supported).
pub(crate) fn handle_to_mat(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    use arrow_array::{Float64Array, Int64Array};
    let dt = borrow_data_table(args, "toMat")?;
    let row_count = dt.row_count();
    let col_count = dt.column_count();

    // Pre-collect each column as Vec<f64> for row-major reconstruction.
    let mut cols: Vec<Vec<f64>> = Vec::with_capacity(col_count);
    for col_idx in 0..col_count {
        let col = dt.inner().column(col_idx);
        if let Some(f64a) = col.as_any().downcast_ref::<Float64Array>() {
            cols.push((0..row_count).map(|i| f64a.value(i)).collect());
        } else if let Some(i64a) = col.as_any().downcast_ref::<Int64Array>() {
            cols.push((0..row_count).map(|i| i64a.value(i) as f64).collect());
        } else {
            return Err(VMError::RuntimeError(format!(
                "toMat: column {} is non-numeric ({:?})",
                col_idx,
                col.data_type()
            )));
        }
    }

    let mut row_arcs: Vec<Arc<HeapValue>> = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let row: Vec<f64> = (0..col_count).map(|c| cols[c][r]).collect();
        let aligned = AlignedVec::from_vec(row);
        let buf = AlignedTypedBuffer::from_aligned(aligned);
        let inner = TypedArrayData::F64(Arc::new(buf));
        let hv = HeapValue::TypedArray(Arc::new(inner));
        row_arcs.push(Arc::new(hv));
    }
    // W17-typed-carrier-bundle-A checkpoint 2/4: Array<Array<f64>> as a
    // value-of-array-of-array carrier has no specialized variant in
    // ADR-006 §2.7.24 Q25.A's spec list. The dispatcher will surface on
    // the nested-TypedArray HeapValue arm. Out-of-territory follow-up:
    // either add `TypedArrayData::TypedArray` (Array<Array<T>>) to Q25.A's
    // list, or route DataTable.toMat through a Matrix carrier directly.
    let outer = shape_value::TypedArrayData::build_specialized_from_heap_arcs(row_arcs)
        .map_err(|err| {
            VMError::NotImplemented(format!(
                "DataTable.toMat: {} — ADR-006 §2.7.24 Q25.A spec list \
                 lacks a `TypedArray`-element variant; out-of-territory \
                 follow-up.",
                err
            ))
        })?;
    Ok(KindedSlot::from_typed_array(Arc::new(outer)))
}

/// `dt.limit(n)` — alias for take-first-n.
pub(crate) fn handle_limit(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_head(vm, args, ctx)
}

/// `dt.execute()` — terminal Queryable adapter. The DataTable is already
/// materialized; `execute` returns it as-is (Queryable trait contract).
pub(crate) fn handle_execute(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "execute")?;
    let bits = Arc::into_raw(dt_arc) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// `dt.rows()` — `Array<RowView>` (each RowView is a TableView Arc).
pub(crate) fn handle_rows(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "rows")?;
    let row_count = dt_arc.row_count();
    let schema_id = dt_arc.schema_id().unwrap_or(0) as u64;
    let mut row_arcs: Vec<Arc<HeapValue>> = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let tv = TableViewData::RowView {
            schema_id,
            table: Arc::clone(&dt_arc),
            row_idx: r,
        };
        let hv = HeapValue::TableView(Arc::new(tv));
        row_arcs.push(Arc::new(hv));
    }
    // W17-typed-carrier-bundle-A checkpoint 2/4: `Array<TableView>` has
    // no specialized variant in ADR-006 §2.7.24 Q25.A's spec list. The
    // dispatcher will surface on the TableView heap arm. Out-of-territory
    // follow-up: add `TypedArrayData::TableView` (Array<RowView | ColumnRef>).
    let outer = shape_value::TypedArrayData::build_specialized_from_heap_arcs(row_arcs)
        .map_err(|err| {
            VMError::NotImplemented(format!(
                "DataTable.rows: {} — ADR-006 §2.7.24 Q25.A spec list lacks \
                 a `TableView`-element variant; out-of-territory follow-up.",
                err
            ))
        })?;
    Ok(KindedSlot::from_typed_array(Arc::new(outer)))
}

/// `dt.columnsRef()` — `Array<ColumnRef>`.
pub(crate) fn handle_columns_ref(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "columnsRef")?;
    let schema_id = dt_arc.schema_id().unwrap_or(0) as u64;
    let col_count = dt_arc.column_count();
    let mut col_arcs: Vec<Arc<HeapValue>> = Vec::with_capacity(col_count);
    for c in 0..col_count {
        let tv = TableViewData::ColumnRef {
            schema_id,
            table: Arc::clone(&dt_arc),
            col_id: c as u32,
        };
        let hv = HeapValue::TableView(Arc::new(tv));
        col_arcs.push(Arc::new(hv));
    }
    // W17-typed-carrier-bundle-A checkpoint 2/4: same TableView-not-in-Q25.A
    // gap as `handle_rows` above. Surface-and-stop with cite.
    let outer = shape_value::TypedArrayData::build_specialized_from_heap_arcs(col_arcs)
        .map_err(|err| {
            VMError::NotImplemented(format!(
                "DataTable.columnsRef: {} — ADR-006 §2.7.24 Q25.A spec list \
                 lacks a `TableView`-element variant; out-of-territory follow-up.",
                err
            ))
        })?;
    Ok(KindedSlot::from_typed_array(Arc::new(outer)))
}

// ── argument coercion helpers ───────────────────────────────────────────────

fn borrow_data_table_arc(args: &[KindedSlot], method: &str) -> Result<Arc<DataTable>, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: missing receiver",
            method
        )));
    }
    let recv = &args[0];
    match recv.kind {
        NativeKind::Ptr(HeapKind::DataTable) => {
            // Borrow without consuming; bump the strong count so the
            // returned Arc has its own share.
            let bits = recv.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(format!(
                    "datatable.{}: null receiver",
                    method
                )));
            }
            // SAFETY: §2.7.6 / Q8 construction-side contract guarantees
            // `Ptr(HeapKind::DataTable)` slot bits = `Arc::into_raw::<DataTable>`.
            unsafe {
                Arc::increment_strong_count(bits as *const DataTable);
                Ok(Arc::from_raw(bits as *const DataTable))
            }
        }
        NativeKind::Ptr(HeapKind::TableView) => {
            let bits = recv.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(format!(
                    "datatable.{}: null receiver",
                    method
                )));
            }
            // SAFETY: same construction-side contract for TableView.
            let tv: &TableViewData = unsafe { &*(bits as *const TableViewData) };
            let inner = match tv {
                TableViewData::TypedTable { table, .. }
                | TableViewData::IndexedTable { table, .. }
                | TableViewData::RowView { table, .. }
                | TableViewData::ColumnRef { table, .. } => Arc::clone(table),
            };
            Ok(inner)
        }
        other => Err(VMError::RuntimeError(format!(
            "datatable.{}: expected DataTable/TableView receiver, got {:?}",
            method, other
        ))),
    }
}

fn arg_str<'a>(
    args: &'a [KindedSlot],
    idx: usize,
    method: &str,
    name: &str,
) -> Result<&'a str, VMError> {
    let slot = args.get(idx).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: missing arg {} ({})",
            method, idx, name
        ))
    })?;
    slot.as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: arg {} ({}) must be string, got {:?}",
            method, idx, name, slot.kind
        ))
    })
}

fn arg_usize(
    args: &[KindedSlot],
    idx: usize,
    method: &str,
    name: &str,
) -> Result<usize, VMError> {
    let slot = args.get(idx).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: missing arg {} ({})",
            method, idx, name
        ))
    })?;
    let n = slot.as_i64().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: arg {} ({}) must be integer, got {:?}",
            method, idx, name, slot.kind
        ))
    })?;
    if n < 0 {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: arg {} ({}) must be non-negative, got {}",
            method, idx, name, n
        )));
    }
    Ok(n as usize)
}
