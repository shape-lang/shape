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
    AlignedVec, DataTable, KindedSlot, NativeKind, TableViewData,
    TypedObjectStorage, ValueSlot, VMError,
    heap_value::{HeapKind, MatrixData},
};
use std::sync::Arc;
use arrow_array::Array;

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
    let _ = borrow_data_table(args, "columns")?;
    Err(VMError::NotImplemented(
        "DataTable.columns: SURFACE — V3-S5 ckpt-5 consumer-cascade tier \
         3 surface. The deleted typed-array-data String result carrier DELETED at \
         ckpt-1..ckpt-4 per W12 audit §3.5 + §B + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. Rebuild lands at ckpt-6 STRICT close per v2-raw \
         `TypedArray<*const StringObj>` direct-access. REFUSED ON SIGHT \
         (Refusal #1)."
            .to_string(),
    ))
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

/// `dt.toMat()` — convert to a flat row-major matrix. Each row's values
/// widen to `f64` (Float64 / Int64 supported).
///
/// W17-out-of-bundle-A-followups (2026-05-12): per the C+ precedent in
/// `phase-2d-playbook.md` §3, rewires from the pre-Q25.A polymorphic
/// `Array<Array<f64>>` (which routed through the deleted
/// `TypedArrayData::HeapValue` carrier) to a direct `MatrixData` —
/// the rectangular-numeric shape is already monomorphic and `MatrixData`
/// is the existing carrier for it. User-visible: `mat.length` returns
/// the row count via Matrix's iteration shape; `mat[i]` returns row `i`
/// as a `FloatSlice` view.
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

    // Build flat row-major buffer; MatrixData stores rows*cols f64s.
    let total = row_count.checked_mul(col_count).ok_or_else(|| {
        VMError::RuntimeError("toMat: row_count * col_count overflow".to_string())
    })?;
    let mut flat: Vec<f64> = Vec::with_capacity(total);
    for r in 0..row_count {
        for c in 0..col_count {
            flat.push(cols[c][r]);
        }
    }
    let aligned = AlignedVec::from_vec(flat);
    let matrix = MatrixData::from_flat(aligned, row_count as u32, col_count as u32);
    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix is its
    // own `HeapKind::Matrix` carrier — push directly as
    // `Ptr(HeapKind::Matrix)`, not wrapped under `TypedArrayData::Matrix`.
    Ok(KindedSlot::from_matrix(Arc::new(matrix)))
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

/// `dt.rows()` — `Array<{column_name: value, ...}>`. Each row materializes
/// as a TypedObject whose schema mirrors the table's column names + types.
///
/// W17-out-of-bundle-A-followups (2026-05-12): per the C+ precedent in
/// `phase-2d-playbook.md` §3, rewires from `Array<TableView>` (which
/// routed through the deleted `TypedArrayData::HeapValue` carrier) to
/// `Array<TypedObject>` via Q25.A's specialized list. Each row reads
/// column data at construction time and writes a per-field slot — the
/// `TableView`'s schema_id-only carrier is no longer needed at the row
/// layer.
///
/// Supported column kinds: i64, f64, bool, string. Other Arrow types
/// surface via SURFACE with an §-cite at construction time so the
/// failure is structural rather than a silent slot-kind mismatch.
pub(crate) fn handle_rows(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "rows")?;
    let row_count = dt_arc.row_count();
    let col_names = dt_arc.column_names();

    // Auto-register the row schema. Field order = column order.
    let schema_id =
        shape_runtime::type_schema::register_predeclared_any_schema(&col_names);

    // Pre-derive per-column kind so each row's slots are typed
    // consistently. Build a per-column reader closure that produces a
    // (ValueSlot, NativeKind, bool/heap) triple for row `i`.
    let col_readers = build_column_readers(&dt_arc, "rows")?;
    let field_kinds: Arc<[NativeKind]> = Arc::from(
        col_readers
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let heap_mask: u64 = {
        let mut m: u64 = 0;
        for (i, r) in col_readers.iter().enumerate() {
            if r.is_heap {
                if i >= 64 {
                    return Err(VMError::NotImplemented(format!(
                        "DataTable.rows: {} columns exceed 64-bit heap_mask \
                         capacity; ADR-006 §2.7.24 Q25.A row schema cap.",
                        col_readers.len()
                    )));
                }
                m |= 1u64 << i;
            }
        }
        m
    };

    // V3-S5 ckpt-5: `TypedArrayData::TypedObject` carrier deleted at
    // ckpt-1..ckpt-4 per W12 audit §3.5 + §B. The pre-ckpt-1 body built
    // row_count `TypedObjectStorage` rows and wrapped them in the deleted
    // carrier. Rebuild lands at ckpt-6 STRICT close per the v2-raw
    // `TypedArray<TypedObjectPtr>` direct-access target.
    let _ = (col_readers, row_count, heap_mask, &field_kinds, schema_id);
    Err(VMError::NotImplemented(
        "DataTable.rows: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 \
         surface. The deleted typed-array-data TypedObject result carrier DELETED at \
         ckpt-1..ckpt-4. Rebuild at ckpt-6 STRICT close. Refusal #1."
            .to_string(),
    ))
}

/// `dt.columnsRef()` — `Array<{name, kind}>`. Each column materializes
/// as a small TypedObject describing the column metadata.
///
/// W17-out-of-bundle-A-followups (2026-05-12): per the C+ precedent in
/// `phase-2d-playbook.md` §3, rewires from `Array<TableView::ColumnRef>`
/// (which routed through the deleted `TypedArrayData::HeapValue`) to
/// `Array<TypedObject>` with a fixed `{name: string, kind: string}`
/// schema. The pre-rewire `ColumnRef` carrier had no method surface
/// reachable through `Array<TableView>` element-access anyway (callers
/// used `dt.column(name)` directly for column data); this preserves
/// the metadata-as-array shape that user code naturally addresses.
pub(crate) fn handle_columns_ref(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_data_table_arc(args, "columnsRef")?;
    let col_count = dt_arc.column_count();
    let col_names = dt_arc.column_names();

    let fields = ["name".to_string(), "kind".to_string()];
    let schema_id = shape_runtime::type_schema::register_predeclared_any_schema(&fields);
    let field_kinds: Arc<[NativeKind]> = Arc::from(
        vec![NativeKind::String, NativeKind::String].into_boxed_slice(),
    );
    let heap_mask: u64 = 0b11;

    // V3-S5 ckpt-5: same surface as `handle_rows` above; rebuild target
    // is the v2-raw `TypedArray<TypedObjectPtr>` direct-access carrier.
    let _ = (col_count, col_names, schema_id, heap_mask, &field_kinds, &dt_arc);
    Err(VMError::NotImplemented(
        "DataTable.columnsRef: SURFACE — V3-S5 ckpt-5 consumer-cascade \
         tier 3 surface. The deleted typed-array-data TypedObject result carrier \
         DELETED at ckpt-1..ckpt-4. Rebuild at ckpt-6 STRICT close. \
         Refusal #1."
            .to_string(),
    ))
}

/// Per-column reader: closure that produces a `ValueSlot` for row `i`,
/// plus the column's `NativeKind` and whether the slot is heap-resident.
/// Built once per `dt.rows()` call; reused across rows.
struct ColumnReader {
    read: Box<dyn Fn(usize) -> ValueSlot>,
    kind: NativeKind,
    is_heap: bool,
}

fn build_column_readers(
    dt: &Arc<DataTable>,
    method: &str,
) -> Result<Vec<ColumnReader>, VMError> {
    use arrow_array::{BooleanArray, Float64Array, Int64Array, StringArray};
    let col_count = dt.column_count();
    let mut readers: Vec<ColumnReader> = Vec::with_capacity(col_count);
    for col_idx in 0..col_count {
        let col = dt.inner().column(col_idx);
        let col_clone = col.clone();
        if let Some(_i64a) = col.as_any().downcast_ref::<Int64Array>() {
            let arr: Arc<Int64Array> = Arc::new(
                col_clone
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .clone(),
            );
            readers.push(ColumnReader {
                read: Box::new(move |i| ValueSlot::from_raw(arr.value(i) as u64)),
                kind: NativeKind::Int64,
                is_heap: false,
            });
        } else if let Some(_f64a) = col.as_any().downcast_ref::<Float64Array>() {
            let arr: Arc<Float64Array> = Arc::new(
                col_clone
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap()
                    .clone(),
            );
            readers.push(ColumnReader {
                read: Box::new(move |i| ValueSlot::from_raw(arr.value(i).to_bits())),
                kind: NativeKind::Float64,
                is_heap: false,
            });
        } else if let Some(_b) = col.as_any().downcast_ref::<BooleanArray>() {
            let arr: Arc<BooleanArray> = Arc::new(
                col_clone
                    .as_any()
                    .downcast_ref::<BooleanArray>()
                    .unwrap()
                    .clone(),
            );
            readers.push(ColumnReader {
                read: Box::new(move |i| {
                    ValueSlot::from_raw(if arr.value(i) { 1 } else { 0 })
                }),
                kind: NativeKind::Bool,
                is_heap: false,
            });
        } else if let Some(_s) = col.as_any().downcast_ref::<StringArray>() {
            let arr: Arc<StringArray> = Arc::new(
                col_clone
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .clone(),
            );
            readers.push(ColumnReader {
                read: Box::new(move |i| {
                    ValueSlot::from_string_arc(Arc::new(arr.value(i).to_string()))
                }),
                kind: NativeKind::String,
                is_heap: true,
            });
        } else {
            return Err(VMError::NotImplemented(format!(
                "DataTable.{}: column {} has unsupported Arrow type {:?} — \
                 only Int64/Float64/Boolean/String columns lower to TypedObject \
                 row slots; other column types tracked as a follow-up \
                 (ADR-006 §2.7.24 Q25.A row-schema extension).",
                method,
                col_idx,
                col.data_type()
            )));
        }
    }
    Ok(readers)
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
