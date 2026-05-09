//! DataTable query methods: filter, orderBy, group_by, forEach, map.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! Closure-driven forms surface (`op_call_value` is itself at SURFACE per
//! `executor/control_flow/mod.rs::op_call_value` —
//! PHASE_2C_CALL_REBUILD_SURFACE; per playbook §8 the correct shape is
//! surface-and-stop until the closure-call rebuild lands).
//!
//! `filter` has a 3-arg non-closure form `filter(col, op, value)` that
//! does not depend on closures and is implemented here. The expected
//! ops are `=`, `!=`, `<`, `<=`, `>`, `>=`. Result is a fresh DataTable
//! containing the filtered rows (zero-copy via `arrow_select::filter`).

use arrow_array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use shape_runtime::context::ExecutionContext;
use shape_value::{DataTable, KindedSlot, NativeKind, ValueSlot, VMError, heap_value::HeapKind};
use std::sync::Arc;

use crate::executor::VirtualMachine;

use super::common::borrow_data_table;

/// `dt.filter(closure)` / `dt.filter(col, op, value)`.
///
/// The 3-arg form is implemented here; the closure form surfaces.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "filter")?;

    // Closure form: 1 arg of kind Closure-family.
    if args.len() == 2
        && matches!(
            args[1].kind,
            NativeKind::Ptr(HeapKind::Closure) | NativeKind::Ptr(HeapKind::Future)
        )
    {
        return Err(VMError::NotImplemented(
            "datatable.filter (closure form) — SURFACE: depends on \
             op_call_value rebuild (executor/control_flow/mod.rs \
             PHASE_2C_CALL_REBUILD_SURFACE)."
                .to_string(),
        ));
    }

    // 3-arg form: `filter(col, op, value)`.
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

    let mask_array = BooleanArray::from(mask);
    let inner = dt.inner();
    let n_cols = inner.num_columns();
    let mut new_cols = Vec::with_capacity(n_cols);
    for c in 0..n_cols {
        let filtered = arrow_select::filter::filter(inner.column(c), &mask_array)
            .map_err(|e| VMError::RuntimeError(format!("datatable.filter: {}", e)))?;
        new_cols.push(filtered);
    }
    let new_batch = arrow_array::RecordBatch::try_new(inner.schema(), new_cols)
        .map_err(|e| VMError::RuntimeError(format!("datatable.filter: {}", e)))?;
    let new_dt = DataTable::new(new_batch);
    let bits = Arc::into_raw(Arc::new(new_dt)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// `dt.orderBy(closure)` / `dt.orderBy(col, asc?)`. The column-name form
/// is identical to `aggregation::handle_sort`; the closure form
/// surfaces.
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
        return Err(VMError::NotImplemented(
            "datatable.orderBy (closure form) — SURFACE: depends on \
             op_call_value rebuild (executor/control_flow/mod.rs \
             PHASE_2C_CALL_REBUILD_SURFACE)."
                .to_string(),
        ));
    }
    super::aggregation::handle_sort(vm, args, ctx)
}

/// `dt.group_by(col)` / `dt.group_by(col, agg_spec)` — closure-/spec-driven
/// dispatch surfaces; the spec-driven form crosses into
/// `D-prop-access` / `D-typed-access` cluster territory.
pub(crate) fn handle_group_by(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.group_by — SURFACE: spec-driven dispatch crosses into \
         D-prop-access / D-typed-access cluster territory; the kinded \
         property-access surface is itself at SURFACE \
         (executor/objects/property_access.rs). Closure form additionally \
         depends on op_call_value rebuild."
            .to_string(),
    ))
}

/// `dt.forEach(closure)` — per-row callback. SURFACE: requires
/// `op_call_value`.
pub(crate) fn handle_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.forEach — SURFACE: per-row closure dispatch depends on \
         op_call_value rebuild (executor/control_flow/mod.rs \
         PHASE_2C_CALL_REBUILD_SURFACE)."
            .to_string(),
    ))
}

/// `dt.map(closure)` — per-row transformation. SURFACE: requires
/// `op_call_value`.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.map — SURFACE: per-row closure dispatch depends on \
         op_call_value rebuild (executor/control_flow/mod.rs \
         PHASE_2C_CALL_REBUILD_SURFACE). The result-type-classification \
         step that drove the pre-Wave-6.5 body (Float / Int / TypedObject \
         per closure return) also depends on the closure dispatch path."
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
