//! DataTable simulation method: simulate.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! `simulate` SURFACEs: it is the heaviest closure-driven body in the
//! territory — per-row handler callback `(row | ctx, state, idx) ->
//! state | { state, result?, event_type? }` plus correlated-mode ctx
//! TypedObject construction. Closure dispatch goes through
//! `op_call_value`, which is itself at SURFACE in
//! `executor/control_flow/mod.rs::op_call_value`
//! (PHASE_2C_CALL_REBUILD_SURFACE).
//!
//! Once the closure-call rebuild lands, the kinded re-implementation
//! (a) borrows the receiver via `common::borrow_data_table`,
//! (b) builds per-row `Arc<TableViewData::RowView>` and pushes as
//!     `NativeKind::Ptr(HeapKind::TableView)`,
//! (c) for correlated mode, builds the ctx `Arc<TypedObjectStorage>`
//!     against the predeclared cache schema and pushes as
//!     `NativeKind::Ptr(HeapKind::TypedObject)`,
//! (d) threads `(row|ctx, state, idx)` `KindedSlot` argument lists
//!     through `op_call_value`,
//! (e) emits the return TypedObject via `Arc::into_raw +
//!     push_kinded(NativeKind::Ptr(HeapKind::TypedObject))` against
//!     `vm.builtin_schemas.simulate_return`.
//!
//! Every bit of (a)/(b)/(c)/(e) is locally available today; (d) is the
//! load-bearing dependency — the closure-call shell is at SURFACE and
//! cannot be partially wired without reintroducing the deleted
//! `call_value_immediate_raw` raw-bits closure-call API (CLAUDE.md
//! "Forbidden Patterns").

use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

use crate::executor::VirtualMachine;

/// `dt.simulate(handler, config?)` — unified simulation method. SURFACE.
pub(crate) fn handle_simulate(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.simulate — SURFACE: per-row handler closure dispatch \
         depends on op_call_value rebuild (executor/control_flow/mod.rs \
         PHASE_2C_CALL_REBUILD_SURFACE). The kinded ABI \
         (args: &[KindedSlot] / Result<KindedSlot, _>) landed Wave-γ; \
         body migration awaits the closure-call cluster."
            .to_string(),
    ))
}
