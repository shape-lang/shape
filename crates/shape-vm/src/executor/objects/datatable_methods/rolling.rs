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

use shape_value::{KindedSlot, VMError};
use shape_runtime::context::ExecutionContext;

use crate::executor::VirtualMachine;

// Pre-§2.7.9 surface helper `stub` deleted with its callers' bodies —
// each handler now carries the §2.7.9-aware migration contract inline
// per the M-datatable Wave-β `joins.rs` precedent. Wave-γ-followup
// body migration territory (D-datatable-rolling-class).

/// `dt.correlation(col_a, col_b)` — Pearson correlation between two columns.
pub(crate) fn handle_correlation(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_correlation — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.covariance(col_a, col_b)` — covariance between two columns.
pub(crate) fn handle_covariance(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_covariance — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.rolling_sum(col, window)` — append rolling-sum column.
pub(crate) fn handle_rolling_sum(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_rolling_sum — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.rolling_mean(col, window)`.
pub(crate) fn handle_rolling_mean(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_rolling_mean — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.rolling_std(col, window)`.
pub(crate) fn handle_rolling_std(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_rolling_std — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.diff(col)` — append first-difference column.
pub(crate) fn handle_diff(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_diff — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.pct_change(col)` — append percent-change column.
pub(crate) fn handle_pct_change(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_pct_change — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// `dt.forward_fill(col)` — append forward-filled column.
pub(crate) fn handle_forward_fill(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_forward_fill — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}
