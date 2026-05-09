//! DataTable aggregation methods: sum, mean, min, max, sort, count,
//! describe, aggregate.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! Bodies are placeholders (`NotImplemented(SURFACE)`) per playbook §7.4
//! REVISED. The pre-Wave-6.5 bodies (a) closure-dispatched per row via
//! the deleted `call_value_immediate_raw` + ValueWord-bits pipeline,
//! (b) interpreted ValueWord-shaped agg specs via deleted
//! `as_any_array()`/`as_str()` accessors, and (c) returned numeric /
//! `Arc<DataTable>` results via deleted `ValueWord::from_*` constructors.
//! The kinded re-implementation pulls the receiver and per-arg slots
//! through the kinded carrier (`KindedSlot`), threads kinded argument
//! lists into closure callbacks, and pushes results via `Arc::into_raw +
//! push_kinded(bits, NativeKind::*)` per playbook §3.

use shape_value::{KindedSlot, VMError};

use crate::executor::VirtualMachine;

/// Helper: stub body for an aggregation handler.
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

/// `dt.sum()` / `dt.sum(col)` / `dt.sum(closure)` — column-wise or
/// closure-driven sum.
pub(crate) fn handle_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "sum",
        "result kind = NativeKind::Float64 (numeric column) / NativeKind::Int64 (i64 column)",
    ))
}

/// `dt.mean()` / `dt.mean(col)` / `dt.mean(closure)`.
pub(crate) fn handle_mean(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("mean", "result kind = NativeKind::Float64"))
}

/// `dt.min()` / `dt.min(col)` / `dt.min(closure)`.
pub(crate) fn handle_min(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("min", "result kind = column kind (Float64 / Int64 / String)"))
}

/// `dt.max()` / `dt.max(col)` / `dt.max(closure)`.
pub(crate) fn handle_max(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("max", "result kind = column kind (Float64 / Int64 / String)"))
}

/// `dt.sort(col, asc?)` — sort rows by column.
pub(crate) fn handle_sort(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("sort", "result kind = receiver kind"))
}

/// `dt.count()` — row count.
pub(crate) fn handle_count(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("count", "result kind = NativeKind::Int64"))
}

/// `dt.describe()` — summary stats DataTable.
pub(crate) fn handle_describe(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "describe",
        "result kind = NativeKind::Ptr(HeapKind::DataTable)",
    ))
}

/// `dt.aggregate({ out_col: \"fn\" | [\"fn\", \"col\"] })` — multi-aggregation.
pub(crate) fn handle_aggregate(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub(
        "aggregate",
        "result kind = NativeKind::Ptr(HeapKind::DataTable); spec arg kind = \
         Ptr(HeapKind::TypedObject), entries dispatched on per-key kind",
    ))
}

/// SURFACE placeholder for the aggregation-spec parser keyed on the
/// deleted `ValueWord` carrier. The kinded replacement takes a
/// `&KindedSlot` and dispatches via `slot.as_heap_value()` on
/// `HeapValue::String(arc)` / `HeapValue::TypedArray(arr)`.
#[allow(dead_code)]
pub(in crate::executor::objects) fn parse_agg_spec_kinded(
    _spec: &KindedSlot,
    _output_col: &str,
) -> Result<(String, String), VMError> {
    Err(VMError::NotImplemented(
        "parse_agg_spec — SURFACE: phase-2c body migration. Spec arg kind = \
         NativeKind::String (single-fn shorthand) or NativeKind::Ptr(HeapKind::\
         TypedArray) (2-element [fn, col] form); kinded dispatch via \
         slot.as_heap_value() + HeapValue::String / HeapValue::TypedArray \
         match per ADR-005 §1."
            .to_string(),
    ))
}

/// SURFACE placeholder for the aggregation evaluator. Result is a
/// `KindedSlot` whose kind matches the agg function (Float64 for
/// sum/mean, Int64 for count, column-kind for min/max).
#[allow(dead_code)]
pub(in crate::executor::objects) fn compute_aggregation_kinded(
    _dt: &shape_value::datatable::DataTable,
    _agg_fn: &str,
    _source_col: &str,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "compute_aggregation — SURFACE: phase-2c body migration. Result kind \
         is agg-fn-dependent: NativeKind::Float64 for sum/mean (Float64 / \
         Int64 columns coerce to f64), NativeKind::Int64 for count, column \
         kind for min/max. Pre-Wave-6.5 body returned ValueWord (deleted)."
            .to_string(),
    ))
}
