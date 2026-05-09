//! Native method handlers for bool values.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! The pre-Wave-6 implementation imported `shape_value::tag_bits::is_tagged`
//! (forbidden #7 — deleted tag_bits dispatch) and the deleted
//! `shape_value::value_word::*` constructor surface plus
//! `objects::raw_helpers::{extract_bool, type_error}` (the raw_helpers
//! family was deleted in cluster D-raw-helpers, leaving only the
//! FilterExpr extractor for logical/mod.rs). The MethodHandler ABI itself
//! (`fn(&mut VM, &mut [u64], _) -> Result<u64>`) is kind-less in both
//! directions; migration to the kinded API depends on the cluster
//! E-builtins-backlog ABI rewrite (Wave 5b template, commit `fa2bafc`).
//! Per playbook §4 #9 / ADR-006 §2.7.7 a Bool-default kinded shim
//! preserving the call-pattern shape is forbidden.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

/// `bool.toString()` / `bool.to_string()`.
pub fn bool_to_string(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — bool.toString(): MethodHandler ABI needs kinded migration \
         (cluster E-builtins-backlog, Wave 5b template); receiver is \
         NativeKind::Bool inline scalar per ADR-006 §2.7.6"
            .to_string(),
    ))
}
