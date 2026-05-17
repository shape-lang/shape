//! DataTable query methods: filter, orderBy, group_by, forEach, map.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! W9-datatable migration: closure-callback bodies route through
//! `vm.call_value_immediate_nb(&closure_slot, &arg_slots, ctx)` per the
//! W7 closure-callback ABI (§2.7.11 / Q12). Per-row arguments are built
//! as `KindedSlot` carriers — `Arc<TableViewData::RowView>` for the row,
//! pushed as `NativeKind::Ptr(HeapKind::TableView)`.
//!
//! The 3-arg form of `filter` (`filter(col, op, value)`) does not depend
//! on closures and is implemented here. The expected ops are `=`, `!=`,
//! `<`, `<=`, `>`, `>=`. Result is a fresh DataTable containing the
//! filtered rows (zero-copy via `arrow_select::filter`).
//!
//! `group_by` and `map` (and the closure form of `aggregate` — see
//! `aggregation.rs`) keep `NotImplemented(SURFACE)` per playbook §4
//! cross-cluster cascade — they cross into property-access /
//! HashMap-spec parsing territory. `simulate` likewise (see
//! `simulation.rs`).

use arrow_array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use shape_runtime::context::ExecutionContext;
use shape_value::{
    DataTable, KindedSlot, NativeKind, TableViewData, ValueSlot, VMError,
    heap_value::HeapKind,
};
use std::sync::Arc;

use crate::executor::VirtualMachine;

use super::common::borrow_data_table;

/// Borrow `args[0]` as an `Arc<DataTable>` (bumping the strong count).
/// Used by closure-driven handlers that need to release the `&args`
/// borrow before calling back into the VM. Mirrors
/// `core.rs::borrow_data_table_arc` — kept module-local to avoid
/// over-exporting.
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
        NativeKind::Ptr(HeapKind::DataTable) => {
            // SAFETY: §2.7.6 / Q8 contract — Ptr(HeapKind::DataTable) bits =
            // Arc::into_raw::<DataTable>.
            unsafe {
                Arc::increment_strong_count(bits as *const DataTable);
                Ok(Arc::from_raw(bits as *const DataTable))
            }
        }
        NativeKind::Ptr(HeapKind::TableView) => {
            // SAFETY: §2.7.6 / Q8 contract — TableView payload is Arc<TableViewData>.
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

/// Build a per-row `KindedSlot` carrying an `Arc<TableViewData::RowView>`.
/// The slot owns one strong-count share of the freshly-allocated
/// `TableViewData` Arc; on Drop the share is released via the §2.7.6/Q8
/// `KindedSlot` Drop dispatch.
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

/// `dt.filter(closure)` / `dt.filter(col, op, value)`.
pub(crate) fn handle_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // Closure form: 1 arg of kind Closure-family.
    if args.len() == 2
        && matches!(
            args[1].kind,
            NativeKind::Ptr(HeapKind::Closure) | NativeKind::Ptr(HeapKind::Future)
        )
    {
        return filter_closure_form(vm, args, ctx);
    }

    // 3-arg form: `filter(col, op, value)`.
    let dt = borrow_data_table(args, "filter")?;
    if args.len() != 4 {
        return Err(VMError::RuntimeError(format!(
            "datatable.filter: expected (col, op, value) or (closure), got {} args",
            args.len() - 1
        )));
    }
    let col_name = args[1].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.filter: arg 1 (col) must be string, got {:?}",
            args[1].kind
        ))
    })?;
    let op = args[2].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.filter: arg 2 (op) must be string, got {:?}",
            args[2].kind
        ))
    })?;
    let value = &args[3];

    let col = dt.column_by_name(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!("datatable.filter: unknown column: {}", col_name))
    })?;
    let n = col.len();
    let mut mask: Vec<bool> = Vec::with_capacity(n);

    if let Some(f64a) = col.as_any().downcast_ref::<Float64Array>() {
        let target = match value.kind {
            NativeKind::Float64 => value.as_f64().unwrap(),
            NativeKind::Int64 => value.as_i64().unwrap() as f64,
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "datatable.filter: value kind {:?} incompatible with Float64 column",
                    value.kind
                )));
            }
        };
        for i in 0..n {
            if f64a.is_null(i) {
                mask.push(false);
                continue;
            }
            mask.push(cmp_f64(f64a.value(i), op, target)?);
        }
    } else if let Some(i64a) = col.as_any().downcast_ref::<Int64Array>() {
        let target = match value.kind {
            NativeKind::Int64 => value.as_i64().unwrap(),
            NativeKind::Float64 => value.as_f64().unwrap() as i64,
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "datatable.filter: value kind {:?} incompatible with Int64 column",
                    value.kind
                )));
            }
        };
        for i in 0..n {
            if i64a.is_null(i) {
                mask.push(false);
                continue;
            }
            mask.push(cmp_i64(i64a.value(i), op, target)?);
        }
    } else if let Some(s) = col.as_any().downcast_ref::<StringArray>() {
        let target = value.as_str().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "datatable.filter: value kind {:?} incompatible with String column",
                value.kind
            ))
        })?;
        for i in 0..n {
            if s.is_null(i) {
                mask.push(false);
                continue;
            }
            mask.push(cmp_str(s.value(i), op, target)?);
        }
    } else if let Some(b) = col.as_any().downcast_ref::<BooleanArray>() {
        let target = value.as_bool().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "datatable.filter: value kind {:?} incompatible with Bool column",
                value.kind
            ))
        })?;
        for i in 0..n {
            if b.is_null(i) {
                mask.push(false);
                continue;
            }
            mask.push(cmp_bool(b.value(i), op, target)?);
        }
    } else {
        return Err(VMError::RuntimeError(format!(
            "datatable.filter: column {} has unsupported type {:?}",
            col_name,
            col.data_type()
        )));
    }

    apply_mask(dt, &mask, "filter")
}

/// Closure form of `filter`: `dt.filter(|row| ...)`. Materializes a
/// boolean mask per row by calling the predicate closure with a
/// `RowView`. ADR-006 §2.7.11 / Q12: closure dispatch goes through
/// `vm.call_value_immediate_nb`.
fn filter_closure_form(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_dt_arc(args, "filter")?;
    let closure = &args[1];
    let n = dt_arc.row_count();
    let mut mask: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        let row_slot = make_row_view_slot(Arc::clone(&dt_arc), i);
        let result =
            vm.call_value_immediate_nb(closure, std::slice::from_ref(&row_slot), ctx.as_deref_mut())?;
        let keep = result.as_bool().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "datatable.filter: predicate returned non-Bool kind {:?}",
                result.kind
            ))
        })?;
        mask.push(keep);
    }
    apply_mask(&dt_arc, &mask, "filter")
}

/// Apply a boolean mask vector to the receiver table, producing a fresh
/// DataTable with the kept rows.
fn apply_mask(dt: &DataTable, mask: &[bool], method: &str) -> Result<KindedSlot, VMError> {
    let mask_array = BooleanArray::from(mask.to_vec());
    let inner = dt.inner();
    let n_cols = inner.num_columns();
    let mut new_cols = Vec::with_capacity(n_cols);
    for c in 0..n_cols {
        let filtered = arrow_select::filter::filter(inner.column(c), &mask_array)
            .map_err(|e| VMError::RuntimeError(format!("datatable.{}: {}", method, e)))?;
        new_cols.push(filtered);
    }
    let new_batch = arrow_array::RecordBatch::try_new(inner.schema(), new_cols)
        .map_err(|e| VMError::RuntimeError(format!("datatable.{}: {}", method, e)))?;
    let new_dt = DataTable::new(new_batch);
    let bits = Arc::into_raw(Arc::new(new_dt)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// `dt.orderBy(closure)` / `dt.orderBy(col, asc?)`. The column-name form
/// is identical to `aggregation::handle_sort`; the closure form sorts
/// rows by the closure-produced key.
pub(crate) fn handle_order_by(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2
        && matches!(
            args[1].kind,
            NativeKind::Ptr(HeapKind::Closure) | NativeKind::Ptr(HeapKind::Future)
        )
    {
        return sort_by_closure_form(vm, args, ctx);
    }
    super::aggregation::handle_sort(vm, args, ctx)
}

/// Closure form of `orderBy` / `sort`: `dt.orderBy(|row| key)`. Calls the
/// key extractor closure once per row, captures the result, and sorts the
/// row indices by the captured keys (heterogeneous keys: numeric,
/// string, bool — same total order as `cmp_keys`). Shared between
/// `handle_order_by` and `aggregation::handle_sort`'s closure forms.
pub(super) fn sort_by_closure_form(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt_arc = borrow_dt_arc(args, "orderBy")?;
    let closure = &args[1];
    let n = dt_arc.row_count();
    let mut keys: Vec<KindedSlot> = Vec::with_capacity(n);
    for i in 0..n {
        let row_slot = make_row_view_slot(Arc::clone(&dt_arc), i);
        let key = vm.call_value_immediate_nb(
            closure,
            std::slice::from_ref(&row_slot),
            ctx.as_deref_mut(),
        )?;
        keys.push(key);
    }

    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by(|&a, &b| cmp_keys(&keys[a], &keys[b]));

    take_indices(&dt_arc, &indices, "orderBy")
}

/// Heterogeneous-kind total order on closure-produced keys. Same-kind
/// pairs use the natural order; mixed-numeric pairs widen to f64.
/// Disparate kinds (numeric vs string, etc.) compare as Equal — the
/// result is left unchanged for those rows.
fn cmp_keys(a: &KindedSlot, b: &KindedSlot) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.kind, b.kind) {
        (NativeKind::Int64, NativeKind::Int64) => {
            a.as_i64().unwrap().cmp(&b.as_i64().unwrap())
        }
        (NativeKind::Float64, NativeKind::Float64) => a
            .as_f64()
            .unwrap()
            .partial_cmp(&b.as_f64().unwrap())
            .unwrap_or(Ordering::Equal),
        (NativeKind::Int64, NativeKind::Float64) => (a.as_i64().unwrap() as f64)
            .partial_cmp(&b.as_f64().unwrap())
            .unwrap_or(Ordering::Equal),
        (NativeKind::Float64, NativeKind::Int64) => a
            .as_f64()
            .unwrap()
            .partial_cmp(&(b.as_i64().unwrap() as f64))
            .unwrap_or(Ordering::Equal),
        (NativeKind::Bool, NativeKind::Bool) => {
            a.as_bool().unwrap().cmp(&b.as_bool().unwrap())
        }
        (NativeKind::String, NativeKind::String) => match (a.as_str(), b.as_str()) {
            (Some(sa), Some(sb)) => sa.cmp(sb),
            _ => Ordering::Equal,
        },
        _ => Ordering::Equal,
    }
}

/// Take rows by index permutation.
fn take_indices(
    dt: &DataTable,
    indices: &[usize],
    method: &str,
) -> Result<KindedSlot, VMError> {
    let idx_array = arrow_array::UInt32Array::from(
        indices.iter().map(|&i| i as u32).collect::<Vec<_>>(),
    );
    let inner = dt.inner();
    let n_cols = inner.num_columns();
    let mut new_cols = Vec::with_capacity(n_cols);
    for c in 0..n_cols {
        let taken = arrow_select::take::take(inner.column(c), &idx_array, None)
            .map_err(|e| VMError::RuntimeError(format!("datatable.{}: take: {}", method, e)))?;
        new_cols.push(taken);
    }
    let new_batch = arrow_array::RecordBatch::try_new(inner.schema(), new_cols)
        .map_err(|e| VMError::RuntimeError(format!("datatable.{}: {}", method, e)))?;
    let new_dt = DataTable::new(new_batch);
    let bits = Arc::into_raw(Arc::new(new_dt)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// `dt.group_by(col)` / `dt.group_by(col, agg_spec)`.
///
/// **SURFACE — §2.7.4 cross-cluster cascade.** `group_by` produces a
/// HashMap<KeyValue, DataTable>-shaped output (single-arg form) or a
/// DataTable with aggregate columns produced from a TypedObject /
/// HashMap spec (two-arg form). Both result shapes cross into
/// property-access / HashMap-construction territory whose kinded
/// surface is itself at SURFACE in `executor/objects/property_access.rs`
/// and `hashmap_methods.rs`. Per playbook §4 the body stays surfaced
/// until the property-access cluster lands its body migrations; the
/// kinded ABI (`args: &[KindedSlot]` / `Result<KindedSlot, _>`) is in
/// place so this entry-point will be the single edit-site when the
/// dependency clears.
pub(crate) fn handle_group_by(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.group_by — SURFACE: §2.7.4 cross-cluster cascade. \
         Result shape (HashMap<key, DataTable> for single-arg form, \
         agg-DataTable for spec form) crosses into property-access / \
         hashmap-construction cluster territory. The closure-callback \
         dispatch path itself is live (W7 §2.7.11/Q12); the unblock is \
         the property-access cluster's body migration."
            .to_string(),
    ))
}

/// `dt.forEach(closure)` — per-row callback. Returns a none/null slot
/// (`KindedSlot::none()`).
pub(crate) fn handle_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "datatable.forEach: expected (closure), got {} args",
            args.len() - 1
        )));
    }
    if !matches!(
        args[1].kind,
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::Ptr(HeapKind::Future)
    ) {
        return Err(VMError::RuntimeError(format!(
            "datatable.forEach: arg 1 must be a closure, got {:?}",
            args[1].kind
        )));
    }
    let dt_arc = borrow_dt_arc(args, "forEach")?;
    let closure = &args[1];
    let n = dt_arc.row_count();
    for i in 0..n {
        let row_slot = make_row_view_slot(Arc::clone(&dt_arc), i);
        let _ = vm.call_value_immediate_nb(
            closure,
            std::slice::from_ref(&row_slot),
            ctx.as_deref_mut(),
        )?;
    }
    Ok(KindedSlot::none())
}

/// `dt.map(closure)` — per-row transformation.
///
/// **SURFACE — §2.7.4 cross-cluster cascade.** Result-type
/// classification (Float / Int / TypedObject per closure return) and
/// per-element heap-wrap construction cross into the heterogeneous-array
/// construction territory (`array_ops.rs::slot_to_heap_arc` is the
/// closest helper, but the destination kind for closure-returned
/// `TypedObject` rows is the new-DataTable construction path that
/// itself depends on property-access cluster). The closure-callback
/// dispatch is live (W7 §2.7.11/Q12); the unblock is the
/// array-construction cluster's body migration alongside the
/// property-access cluster.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.map — SURFACE: §2.7.4 cross-cluster cascade. The \
         result-type classification step (Float / Int / TypedObject per \
         closure return) and the heap-wrap construction for \
         heterogeneous results cross into the array-construction / \
         property-access cluster territory. Closure dispatch itself \
         (W7 §2.7.11/Q12) is live; this entry-point unblocks alongside \
         the array-of-TypedObject construction body."
            .to_string(),
    ))
}

// ── comparison helpers ──────────────────────────────────────────────────────

fn cmp_f64(a: f64, op: &str, b: f64) -> Result<bool, VMError> {
    Ok(match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        other => {
            return Err(VMError::RuntimeError(format!(
                "datatable.filter: unknown op: {}",
                other
            )));
        }
    })
}

fn cmp_i64(a: i64, op: &str, b: i64) -> Result<bool, VMError> {
    Ok(match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        other => {
            return Err(VMError::RuntimeError(format!(
                "datatable.filter: unknown op: {}",
                other
            )));
        }
    })
}

fn cmp_str(a: &str, op: &str, b: &str) -> Result<bool, VMError> {
    Ok(match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        other => {
            return Err(VMError::RuntimeError(format!(
                "datatable.filter: unknown op: {}",
                other
            )));
        }
    })
}

fn cmp_bool(a: bool, op: &str, b: bool) -> Result<bool, VMError> {
    Ok(match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        other => {
            return Err(VMError::RuntimeError(format!(
                "datatable.filter: op {} not supported on Bool column",
                other
            )));
        }
    })
}
