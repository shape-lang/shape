//! SIMD-backed DataTable methods: correlation, covariance, rolling_sum,
//! rolling_mean, rolling_std, diff, pct_change, forward_fill.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! Bodies are placeholders (`NotImplemented(SURFACE)`) per playbook §7.4
//! REVISED. The pre-Wave-6.5 bodies fed string column-name args through
//! the deleted `raw_helpers::extract_str(u64)` ValueWord-bits accessor
//! and emitted result tables via deleted `ValueWord::from_datatable` /
//! `wrap_result_table_nb`. The kinded re-implementation pulls the column
//! name as `NativeKind::String` (Arc<String> via Arc::from_raw, see
//! playbook §3 string pop pattern), the window size as
//! `NativeKind::Int64`, and pushes the result table via `Arc::into_raw +
//! push_kinded(bits, NativeKind::Ptr(HeapKind::DataTable))`.

use shape_value::VMError;

use crate::executor::VirtualMachine;

#[inline]
fn stub(name: &str, kind_source: &str) -> VMError {
    VMError::NotImplemented(format!(
        "datatable.{} — SURFACE: phase-2c body migration. Receiver kind = \
         NativeKind::Ptr(HeapKind::DataTable) (or Ptr(HeapKind::TableView) \
         for typed/indexed variants); body re-shape requires kinded receiver \
         dispatch via slot.as_heap_value() + HeapValue::DataTable / \
         HeapValue::TableView match per ADR-005 §1, kinded column-name arg \
         (NativeKind::String) and integer window arg (NativeKind::Int64), \
         and result push via Arc::into_raw + push_kinded per playbook §3 \
         ({}).",
        name, kind_source
    ))
}

/// `dt.correlation(col_a, col_b)` — Pearson correlation between two columns.
pub(crate) fn handle_correlation(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("correlation", "result kind = NativeKind::Float64"))
}

/// `dt.covariance(col_a, col_b)` — covariance between two columns.
pub(crate) fn handle_covariance(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("covariance", "result kind = NativeKind::Float64"))
}

/// `dt.rolling_sum(col, window)` — append rolling-sum column.
pub(crate) fn handle_rolling_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("rolling_sum", "result kind = receiver kind"))
}

/// `dt.rolling_mean(col, window)`.
pub(crate) fn handle_rolling_mean(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("rolling_mean", "result kind = receiver kind"))
}

/// `dt.rolling_std(col, window)`.
pub(crate) fn handle_rolling_std(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("rolling_std", "result kind = receiver kind"))
}

/// `dt.diff(col)` — append first-difference column.
pub(crate) fn handle_diff(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("diff", "result kind = receiver kind"))
}

/// `dt.pct_change(col)` — append percent-change column.
pub(crate) fn handle_pct_change(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("pct_change", "result kind = receiver kind"))
}

/// `dt.forward_fill(col)` — append forward-filled column.
pub(crate) fn handle_forward_fill(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(stub("forward_fill", "result kind = receiver kind"))
}
