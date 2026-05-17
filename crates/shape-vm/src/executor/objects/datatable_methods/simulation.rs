//! DataTable simulation method: simulate.
//!
//! ADR-006 §2.7.10 / Q11 — W9-datatable body migration.
//!
//! `simulate` stays at SURFACE per playbook §4 cross-cluster cascade.
//! The closure-callback path itself is live (W7 §2.7.11/Q12,
//! `vm.call_value_immediate_nb`); the deferred dependencies are:
//!
//! - **Correlated-mode ctx TypedObject construction.** The handler
//!   must build a per-iteration `Arc<TypedObjectStorage>` against
//!   `vm.builtin_schemas.simulate_return` for the correlated-mode
//!   handler signature `(ctx, state, idx) -> { state, result?,
//!   event_type? }`. TypedObjectStorage construction with a known
//!   schema crosses into property-access cluster territory.
//!
//! - **Result TypedObject extraction.** The handler dispatches on the
//!   closure return value's TypedObject field shape (`state` /
//!   `result` / `event_type`) to drive the simulation step. Field-walk
//!   over a kinded TypedObject is the same dependency as `aggregate` /
//!   `group_by` — property-access cluster.
//!
//! Once the property-access cluster lands its body migration the
//! re-implementation contract is:
//!
//! 1. borrow the receiver via `common::borrow_data_table` (or the
//!    `borrow_dt_arc` pattern in `query.rs`),
//! 2. build per-row `Arc<TableViewData::RowView>` and pass as
//!    `NativeKind::Ptr(HeapKind::TableView)`,
//! 3. for correlated mode, build the ctx `Arc<TypedObjectStorage>`
//!    against the predeclared cache schema and pass as
//!    `NativeKind::Ptr(HeapKind::TypedObject)`,
//! 4. thread `(row|ctx, state, idx)` `KindedSlot` argument lists
//!    through `vm.call_value_immediate_nb`,
//! 5. emit the return TypedObject via `Arc::into_raw +
//!    KindedSlot::from_typed_object` against
//!    `vm.builtin_schemas.simulate_return`.
//!
//! Step (4) is unblocked today; (1)-(3)-(5) wait on property-access.

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
        "datatable.simulate — SURFACE: §2.7.4 cross-cluster cascade. \
         Per-row handler closure dispatch is live (W7 §2.7.11/Q12 \
         vm.call_value_immediate_nb); the gate is the per-iteration \
         TypedObject construction (correlated-mode ctx + state \
         dispatch) which crosses into property-access cluster \
         territory. Single-edit unblock once property-access lands."
            .to_string(),
    ))
}
