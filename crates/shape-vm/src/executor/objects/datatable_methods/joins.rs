//! DataTable join methods: innerJoin, leftJoin.
//!
//! ADR-006 §2.7.10 / Q11 — W9-datatable body migration.
//!
//! Both handlers stay at SURFACE per playbook §4 cross-cluster cascade.
//! The closure-callback dispatch path itself is live (W7 §2.7.11/Q12,
//! `vm.call_value_immediate_nb`) — the gate is downstream:
//!
//! - **Result construction.** Both joins build a fresh result DataTable
//!   from per-row TypedObject results returned by the user-supplied
//!   `resultSelector` closure. The previous `build_datatable_from_objects`
//!   helper walked TypedObject / HashMap fields to materialize the
//!   result schema and column buffers; the kinded successor lives in
//!   property-access cluster territory (`property_access.rs` /
//!   `hashmap_methods.rs`), both of which are at SURFACE.
//!
//! - **JoinExecute opcode boundary.** `executor/window_join.rs::handle_join_execute`
//!   (the SQL-style `JOIN` opcode dispatch) is itself a W8-WJ surface
//!   that explicitly defers to this cluster's ABI flip — see
//!   `window_join.rs:479-487`. The two halves of the join pipeline must
//!   move together; the method-call path can land independently of the
//!   opcode path, but the result-DataTable construction is the shared
//!   dependency.
//!
//! Per playbook §4 surface-and-stop trigger ("cross-cluster cascade —
//! body needs a method registry change affecting other sub-clusters"),
//! the handlers stay surfaced with explicit §2.7.4 citation. The kinded
//! ABI (`args: &[KindedSlot]` / `Result<KindedSlot, _>`) is in place so
//! these are single-edit-site unblocks once the property-access cluster
//! lands.

use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

use crate::executor::VirtualMachine;

/// `dt.innerJoin(other, leftKey, rightKey, resultSelector)` — kinded ABI.
pub(crate) fn handle_inner_join(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.innerJoin — SURFACE: §2.7.4 cross-cluster cascade. \
         Closure-callback dispatch (key extractors + resultSelector) is \
         live via W7 §2.7.11/Q12 vm.call_value_immediate_nb; the gate is \
         the result-DataTable construction from closure-returned \
         TypedObject rows (property-access cluster territory) and the \
         W8-WJ JoinExecute opcode boundary (window_join.rs:479-487). \
         Single-edit unblock once property-access lands."
            .to_string(),
    ))
}

/// `dt.leftJoin(other, leftKey, rightKey, resultSelector)` — kinded ABI.
pub(crate) fn handle_left_join(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.leftJoin — SURFACE: §2.7.4 cross-cluster cascade. \
         Same shape as innerJoin: closure-callback dispatch is live but \
         the result-DataTable construction (TypedObject row → DataTable \
         schema + columns) crosses into property-access cluster \
         territory. The unmatched-row branch additionally needs an \
         empty-TypedObject argument to the result selector — kinded \
         construction lands with the property-access cluster body \
         migration. W8-WJ JoinExecute opcode (window_join.rs:479-487) \
         is the second consumer that unblocks together."
            .to_string(),
    ))
}
