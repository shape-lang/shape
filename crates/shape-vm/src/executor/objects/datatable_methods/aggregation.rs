//! DataTable aggregation methods: sum, mean, min, max, sort, count,
//! describe, aggregate.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! Receiver: `args[0].kind ∈ { NativeKind::Ptr(HeapKind::DataTable),
//! NativeKind::Ptr(HeapKind::TableView) }`. Borrowed via
//! `borrow_data_table` (see `common.rs`) — typed-Arc dispatch, NOT
//! `as_heap_value()` (unsound on typed-Arc slots per Wave-γ
//! G-heap-filter-expr soundness amendment).
//!
//! Form coverage:
//!   - Column-name forms (e.g. `dt.sum("price")`, `dt.mean("volume")`) —
//!     real bodies. Per-column kind dispatch via Arrow `DataType` match.
//!   - Closure forms (e.g. `dt.sum(|row| row.price * row.qty)`) — SURFACE.
//!     Closure dispatch goes through `op_call_value`, which is itself at
//!     SURFACE in `executor/control_flow/mod.rs::op_call_value` (the
//!     PHASE_2C_CALL_REBUILD_SURFACE constant). Per playbook §8 the
//!     correct shape is surface-and-stop until the closure rebuild
//!     lands.
//!   - Object-spec aggregate (e.g. `dt.aggregate({ total: "sum" })`) —
//!     SURFACE. Spec parsing requires HashMap dispatch which is its own
//!     cluster.

use arrow_array::{Array, BooleanArray, Float64Array, Int64Array};
use shape_runtime::context::ExecutionContext;
use shape_value::{
    DataTable, KindedSlot, NativeKind, TableViewData, ValueSlot, VMError,
    heap_value::HeapKind,
};
use std::sync::Arc;

use crate::executor::VirtualMachine;

use super::common::borrow_data_table;

/// Borrow `args[0]` as `Arc<DataTable>` for closure-driven handlers
/// (mirrors `query.rs::borrow_dt_arc`; see §2.7.6 / Q8 contract).
fn borrow_dt_arc(args: &[KindedSlot], method: &str) -> Result<Arc<DataTable>, VMError> {
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
        NativeKind::Ptr(HeapKind::DataTable) => unsafe {
            Arc::increment_strong_count(bits as *const DataTable);
            Ok(Arc::from_raw(bits as *const DataTable))
        },
        NativeKind::Ptr(HeapKind::TableView) => {
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

/// Build a per-row `KindedSlot` carrying an `Arc<TableViewData::RowView>`
/// (mirrors `query.rs::make_row_view_slot`).
fn make_row_view_slot(table: Arc<DataTable>, row_idx: usize) -> KindedSlot {
    let schema_id = table.schema_id().unwrap_or(0) as u64;
    let tv = TableViewData::RowView {
        schema_id,
        table,
        row_idx,
    };
    let bits = Arc::into_raw(Arc::new(tv)) as u64;
    KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::TableView),
    )
}

/// Per-column numeric aggregation, dispatched on Arrow `DataType` of the
/// referenced column. Kept module-local so each handler can call it
/// without re-implementing the column-kind cascade.
fn col_numeric_agg(
    dt: &DataTable,
    col_name: &str,
    method: &str,
    op: AggOp,
) -> Result<KindedSlot, VMError> {
    let col = dt.column_by_name(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!("datatable.{}: unknown column: {}", method, col_name))
    })?;
    if let Some(f64a) = col.as_any().downcast_ref::<Float64Array>() {
        let n = f64a.len();
        if n == 0 {
            return Err(VMError::RuntimeError(format!(
                "datatable.{}: empty column",
                method
            )));
        }
        // Treat nulls as skipped — track count of valid entries.
        let mut count = 0usize;
        let mut acc = 0.0f64;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for i in 0..n {
            if f64a.is_null(i) {
                continue;
            }
            let v = f64a.value(i);
            acc += v;
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            count += 1;
        }
        if count == 0 {
            return Err(VMError::RuntimeError(format!(
                "datatable.{}: column is all-null",
                method
            )));
        }
        let result = match op {
            AggOp::Sum => acc,
            AggOp::Mean => acc / (count as f64),
            AggOp::Min => min,
            AggOp::Max => max,
        };
        Ok(KindedSlot::from_number(result))
    } else if let Some(i64a) = col.as_any().downcast_ref::<Int64Array>() {
        let n = i64a.len();
        if n == 0 {
            return Err(VMError::RuntimeError(format!(
                "datatable.{}: empty column",
                method
            )));
        }
        let mut count = 0usize;
        let mut acc: i128 = 0;
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        for i in 0..n {
            if i64a.is_null(i) {
                continue;
            }
            let v = i64a.value(i);
            acc += v as i128;
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            count += 1;
        }
        if count == 0 {
            return Err(VMError::RuntimeError(format!(
                "datatable.{}: column is all-null",
                method
            )));
        }
        match op {
            AggOp::Sum => {
                // Sum of i64 may overflow i64 — surface as a runtime error
                // rather than silently wrap. Callers needing a wider type
                // can `mean` (returns f64) or pre-coerce to Float64.
                if acc > i64::MAX as i128 || acc < i64::MIN as i128 {
                    return Err(VMError::RuntimeError(format!(
                        "datatable.{}: sum overflow on Int64 column",
                        method
                    )));
                }
                Ok(KindedSlot::from_int(acc as i64))
            }
            AggOp::Mean => Ok(KindedSlot::from_number((acc as f64) / (count as f64))),
            AggOp::Min => Ok(KindedSlot::from_int(min)),
            AggOp::Max => Ok(KindedSlot::from_int(max)),
        }
    } else if let Some(b) = col.as_any().downcast_ref::<BooleanArray>() {
        // Count-true semantics for Bool columns. Sum/mean/min/max generalize:
        //   sum = count_true; mean = count_true / count; min = false if any
        //   false; max = true if any true.
        let n = b.len();
        let mut count = 0usize;
        let mut count_true = 0usize;
        for i in 0..n {
            if b.is_null(i) {
                continue;
            }
            count += 1;
            if b.value(i) {
                count_true += 1;
            }
        }
        if count == 0 {
            return Err(VMError::RuntimeError(format!(
                "datatable.{}: column is all-null",
                method
            )));
        }
        match op {
            AggOp::Sum => Ok(KindedSlot::from_int(count_true as i64)),
            AggOp::Mean => Ok(KindedSlot::from_number((count_true as f64) / (count as f64))),
            AggOp::Min => Ok(KindedSlot::from_bool(count_true == count)),
            AggOp::Max => Ok(KindedSlot::from_bool(count_true > 0)),
        }
    } else {
        Err(VMError::RuntimeError(format!(
            "datatable.{}: column {} is non-numeric ({:?})",
            method,
            col_name,
            col.data_type()
        )))
    }
}

#[derive(Debug, Clone, Copy)]
enum AggOp {
    Sum,
    Mean,
    Min,
    Max,
}

fn dispatch_agg(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    method: &str,
    op: AggOp,
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: column name or closure required",
            method
        )));
    }
    let arg = &args[1];
    if matches!(arg.kind, NativeKind::Ptr(HeapKind::Closure)) {
        return closure_form_agg(vm, args, method, op, ctx);
    }
    if let Some(name) = arg.as_str() {
        let dt = borrow_data_table(args, method)?;
        return col_numeric_agg(dt, name, method, op);
    }
    Err(VMError::RuntimeError(format!(
        "datatable.{}: arg 1 must be a column-name string or closure, got {:?}",
        method, arg.kind
    )))
}

/// Closure form of sum/mean/min/max: invoke the closure once per row,
/// reduce the numeric results to a single scalar. ADR-006 §2.7.11/Q12
/// closure-callback dispatch through `vm.call_value_immediate_nb`.
fn closure_form_agg(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    method: &str,
    op: AggOp,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_dt_arc(args, method)?;
    let closure = &args[1];
    let n = dt_arc.row_count();
    if n == 0 {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: empty table",
            method
        )));
    }
    // Reduce in f64-domain (mirrors col_numeric_agg's Float64 arm). Int
    // closure returns widen.
    let mut count = 0usize;
    let mut acc = 0.0f64;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for i in 0..n {
        let row_slot = make_row_view_slot(Arc::clone(&dt_arc), i);
        let result = vm.call_value_immediate_nb(
            closure,
            std::slice::from_ref(&row_slot),
            ctx.as_deref_mut(),
        )?;
        let v = match result.kind {
            NativeKind::Float64 => result.as_f64().unwrap(),
            NativeKind::Int64 => result.as_i64().unwrap() as f64,
            other => {
                return Err(VMError::RuntimeError(format!(
                    "datatable.{}: closure returned non-numeric kind {:?}",
                    method, other
                )));
            }
        };
        acc += v;
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        count += 1;
    }
    let result = match op {
        AggOp::Sum => acc,
        AggOp::Mean => acc / (count as f64),
        AggOp::Min => min,
        AggOp::Max => max,
    };
    Ok(KindedSlot::from_number(result))
}

/// `dt.sum()` / `dt.sum(col)` / `dt.sum(closure)`.
pub(crate) fn handle_sum(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    dispatch_agg(vm, args, "sum", AggOp::Sum, ctx)
}

/// `dt.mean()` / `dt.mean(col)` / `dt.mean(closure)`.
pub(crate) fn handle_mean(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    dispatch_agg(vm, args, "mean", AggOp::Mean, ctx)
}

/// `dt.min()` / `dt.min(col)` / `dt.min(closure)`.
pub(crate) fn handle_min(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    dispatch_agg(vm, args, "min", AggOp::Min, ctx)
}

/// `dt.max()` / `dt.max(col)` / `dt.max(closure)`.
pub(crate) fn handle_max(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    dispatch_agg(vm, args, "max", AggOp::Max, ctx)
}

/// `dt.sort(col)` / `dt.sort(col, asc)` / `dt.sort(|row| key)` — sort
/// rows by column or by closure-extracted key. Two-arg form is `(col,
/// asc: bool)`; closure form sorts by the closure's per-row return
/// (numeric / string / bool keys, heterogeneous-kind total order).
pub(crate) fn handle_sort(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    use arrow_array::StringArray;
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "datatable.sort: column name or closure required".to_string(),
        ));
    }
    let arg1 = &args[1];
    if matches!(arg1.kind, NativeKind::Ptr(HeapKind::Closure)) {
        return super::query::sort_by_closure_form(vm, args, ctx);
    }
    let dt = borrow_data_table(args, "sort")?;
    let col_name = arg1.as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.sort: arg 1 must be column-name string, got {:?}",
            arg1.kind
        ))
    })?;
    let ascending = if let Some(s) = args.get(2) {
        s.as_bool().unwrap_or(true)
    } else {
        true
    };

    let col = dt.column_by_name(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!("datatable.sort: unknown column: {}", col_name))
    })?;

    // Build the index permutation by stable sort over the column values.
    let n = col.len();
    let mut indices: Vec<usize> = (0..n).collect();

    if let Some(f64a) = col.as_any().downcast_ref::<Float64Array>() {
        indices.sort_by(|&a, &b| {
            let va = f64a.value(a);
            let vb = f64a.value(b);
            let ord = va
                .partial_cmp(&vb)
                .unwrap_or(std::cmp::Ordering::Equal);
            if ascending { ord } else { ord.reverse() }
        });
    } else if let Some(i64a) = col.as_any().downcast_ref::<Int64Array>() {
        indices.sort_by(|&a, &b| {
            let ord = i64a.value(a).cmp(&i64a.value(b));
            if ascending { ord } else { ord.reverse() }
        });
    } else if let Some(s) = col.as_any().downcast_ref::<StringArray>() {
        indices.sort_by(|&a, &b| {
            let ord = s.value(a).cmp(s.value(b));
            if ascending { ord } else { ord.reverse() }
        });
    } else if let Some(b) = col.as_any().downcast_ref::<BooleanArray>() {
        indices.sort_by(|&a, &c| {
            let ord = b.value(a).cmp(&b.value(c));
            if ascending { ord } else { ord.reverse() }
        });
    } else {
        return Err(VMError::RuntimeError(format!(
            "datatable.sort: column {} has unsupported type {:?}",
            col_name,
            col.data_type()
        )));
    }

    let idx_array =
        arrow_array::UInt32Array::from(indices.into_iter().map(|i| i as u32).collect::<Vec<_>>());
    let inner = dt.inner();
    let n_cols = inner.num_columns();
    let mut new_cols = Vec::with_capacity(n_cols);
    for c in 0..n_cols {
        let taken = arrow_select::take::take(inner.column(c), &idx_array, None)
            .map_err(|e| VMError::RuntimeError(format!("datatable.sort: take: {}", e)))?;
        new_cols.push(taken);
    }
    let new_batch = arrow_array::RecordBatch::try_new(inner.schema(), new_cols)
        .map_err(|e| VMError::RuntimeError(format!("datatable.sort: {}", e)))?;
    let new_dt = DataTable::new(new_batch);
    let bits = Arc::into_raw(Arc::new(new_dt)) as u64;
    Ok(KindedSlot::new(
        shape_value::ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// `dt.count()` — row count.
pub(crate) fn handle_count(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "count")?;
    Ok(KindedSlot::from_int(dt.row_count() as i64))
}

/// `dt.describe()` — summary stats DataTable.
///
/// Builds a result table with one row per numeric column and columns:
/// `column` (string), `count` (int), `mean` (number), `min` (number),
/// `max` (number).
pub(crate) fn handle_describe(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    use arrow_schema::{DataType, Field, Schema};
    use shape_value::DataTableBuilder;

    let dt = borrow_data_table(args, "describe")?;

    let names = dt.column_names();
    let mut col_names = Vec::new();
    let mut counts = Vec::new();
    let mut means = Vec::new();
    let mut mins = Vec::new();
    let mut maxs = Vec::new();

    for name in &names {
        let col = match dt.column_by_name(name) {
            Some(c) => c,
            None => continue,
        };
        let (count, mean, min, max) = if let Some(f) = col.as_any().downcast_ref::<Float64Array>() {
            let n = f.len();
            let mut c = 0usize;
            let mut s = 0.0f64;
            let mut mn = f64::INFINITY;
            let mut mx = f64::NEG_INFINITY;
            for i in 0..n {
                if f.is_null(i) {
                    continue;
                }
                let v = f.value(i);
                s += v;
                if v < mn { mn = v; }
                if v > mx { mx = v; }
                c += 1;
            }
            if c == 0 { continue; }
            (c as i64, s / (c as f64), mn, mx)
        } else if let Some(i) = col.as_any().downcast_ref::<Int64Array>() {
            let n = i.len();
            let mut c = 0usize;
            let mut s: i128 = 0;
            let mut mn = i64::MAX;
            let mut mx = i64::MIN;
            for k in 0..n {
                if i.is_null(k) {
                    continue;
                }
                let v = i.value(k);
                s += v as i128;
                if v < mn { mn = v; }
                if v > mx { mx = v; }
                c += 1;
            }
            if c == 0 { continue; }
            (c as i64, (s as f64) / (c as f64), mn as f64, mx as f64)
        } else {
            // Skip non-numeric columns silently — `describe` output is
            // numeric-only by convention.
            continue;
        };
        col_names.push(name.as_str());
        counts.push(count);
        means.push(mean);
        mins.push(min);
        maxs.push(max);
    }

    let schema = Schema::new(vec![
        Field::new("column", DataType::Utf8, false),
        Field::new("count", DataType::Int64, false),
        Field::new("mean", DataType::Float64, false),
        Field::new("min", DataType::Float64, false),
        Field::new("max", DataType::Float64, false),
    ]);
    let mut b = DataTableBuilder::new(schema);
    b.add_string_column(col_names);
    b.add_i64_column(counts);
    b.add_f64_column(means);
    b.add_f64_column(mins);
    b.add_f64_column(maxs);
    let result = b
        .finish()
        .map_err(|e| VMError::RuntimeError(format!("datatable.describe: {}", e)))?;
    let bits = Arc::into_raw(Arc::new(result)) as u64;
    Ok(KindedSlot::new(
        shape_value::ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// `dt.aggregate({ out_col: "fn" | ["fn", "col"] })` — multi-aggregation.
///
/// SURFACE: spec parsing depends on `HashMap` / `TypedObject` field
/// inspection helpers that themselves cross into the
/// `D-prop-access` / `D-typed-access` cluster territory; the agg-spec
/// dispatch is best surfaced rather than partially wired.
pub(crate) fn handle_aggregate(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.aggregate — SURFACE: agg-spec parsing (HashMap/TypedObject \
         field walk) crosses into D-prop-access / D-typed-access cluster \
         territory; the kinded property-access surface is itself at SURFACE \
         (executor/objects/property_access.rs). Single-column aggregations \
         (`dt.sum(col)`, etc.) are migrated; the multi-aggregation entry \
         point waits on the property-access cluster's body migration."
            .to_string(),
    ))
}

/// SURFACE placeholder for the aggregation-spec parser keyed on the
/// deleted `ValueWord` carrier.
#[allow(dead_code)]
pub(in crate::executor::objects) fn parse_agg_spec_kinded(
    _spec: &KindedSlot,
    _output_col: &str,
) -> Result<(String, String), VMError> {
    Err(VMError::NotImplemented(
        "parse_agg_spec — SURFACE: depends on the property-access cluster \
         (HashMap / TypedObject kinded field walk) per the handle_aggregate \
         migration note."
            .to_string(),
    ))
}

/// Aggregation evaluator — kinded result. Wraps `col_numeric_agg` for
/// callers (currently the unused `parse_agg_spec_kinded` follow-up;
/// kept dead-code-allow until `handle_aggregate` lands).
#[allow(dead_code)]
pub(in crate::executor::objects) fn compute_aggregation_kinded(
    dt: &DataTable,
    agg_fn: &str,
    source_col: &str,
) -> Result<KindedSlot, VMError> {
    let op = match agg_fn {
        "sum" => AggOp::Sum,
        "mean" | "avg" => AggOp::Mean,
        "min" => AggOp::Min,
        "max" => AggOp::Max,
        "count" => {
            return Ok(KindedSlot::from_int(dt.row_count() as i64));
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "compute_aggregation: unknown agg function: {}",
                other
            )));
        }
    };
    col_numeric_agg(dt, source_col, agg_fn, op)
}
