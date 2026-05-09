//! DataTable join methods: innerJoin, leftJoin.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! Both handlers SURFACE: the join contract takes per-side key-extractor
//! closures plus a result-selector closure (`dt.innerJoin(other, leftKey,
//! rightKey, resultSelector)`). Closure dispatch goes through
//! `op_call_value`, which is itself at SURFACE in
//! `executor/control_flow/mod.rs::op_call_value`
//! (PHASE_2C_CALL_REBUILD_SURFACE).
//!
//! The pre-Wave-6.5 bodies relied on the deleted ValueWord-bits closure
//! call API (`call_value_immediate_raw`), the deleted `as_number_coerce`
//! key-equality helper, and the deleted `vw_clone` retain-on-read pair —
//! every one a CLAUDE.md "Forbidden Patterns" target. Per playbook §8 the
//! correct shape is surface-and-stop until the closure rebuild lands.

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
        "datatable.innerJoin — SURFACE: per-side key-extractor + result-\
         selector closures depend on op_call_value rebuild \
         (executor/control_flow/mod.rs PHASE_2C_CALL_REBUILD_SURFACE). \
         The kinded ABI (args: &[KindedSlot] / Result<KindedSlot, _>) \
         landed in Wave-β M-datatable; the body migration awaits the \
         closure-call cluster."
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
        "datatable.leftJoin — SURFACE: same closure-driven shape as \
         innerJoin; depends on op_call_value rebuild. The unmatched-row \
         branch additionally needs an empty-TypedObject argument to the \
         result selector — kinded construction lands with the closure \
         migration."
            .to_string(),
    ))
}
