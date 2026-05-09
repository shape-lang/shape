//! DataTable simulation method: simulate.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! `handle_simulate` is a placeholder (`NotImplemented(SURFACE)`) per
//! playbook §7.4 REVISED. The pre-Wave-6.5 body was the heaviest
//! ValueWord-shaped handler in the territory: it built per-row RowView /
//! correlated-context TypedObjects via deleted
//! `ValueWord::from_row_view` / `ValueWord::from_heap_value` /
//! `nb_to_slot_with_field_type`, retained additional Arc shares via the
//! deleted `shape_value::vw_clone(bits)` helper (CLAUDE.md "Forbidden
//! Patterns" #8 — the deleted `vw_clone` / `vw_drop` pair, replaced
//! repo-wide by `clone_with_kind` / `drop_with_kind`), and assembled the
//! return TypedObject via deleted `vmarray_from_vec` / `from_array`
//! constructors.
//!
//! The kinded re-implementation (a) pulls the receiver and config args
//! as `KindedSlot`, (b) builds RowView / context TypedObjects as
//! `Arc::into_raw + push_kinded` per playbook §3, (c) threads
//! `KindedSlot` argument lists into `op_call_value` for the per-row
//! handler callback, (d) calls `clone_with_kind(bits, kind)` for
//! retain-on-read shares (replacing every `vw_clone(bits)` site), and
//! (e) emits the return TypedObject via `Arc::into_raw +
//! push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))` against
//! the `vm.builtin_schemas.simulate_return` schema.

use shape_value::{KindedSlot, VMError};
use shape_runtime::context::ExecutionContext;

use crate::executor::VirtualMachine;

/// `dt.simulate(handler, config?)` — unified simulation method.
///
/// The pre-Wave-6.5 contract:
/// - Single mode: `handler(row, state, idx)` where `row` is RowView.
/// - Correlated mode (`config.tables` present): `handler(ctx, state, idx)`
///   where `ctx` is a TypedObject with named RowView fields.
/// - Handler result with `state` key extracts state + optional `result` /
///   `event_type`; otherwise the entire result is the new state.
/// - Returns a TypedObject with the `simulate_return` schema (6 fields:
///   final_state, results, elements_processed, completed, event_log, seed).
pub(crate) fn handle_simulate(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.simulate — SURFACE: phase-2c body migration. Receiver kind \
         = NativeKind::Ptr(HeapKind::DataTable). The body must (1) dispatch \
         on receiver via slot.as_heap_value() + HeapValue::DataTable per \
         ADR-005 §1, (2) build per-row RowView TypedObjects as `Arc::into_raw\
         (Arc<TableViewData>) + push_kinded(bits, NativeKind::Ptr(HeapKind::\
         TableView))` (replaces deleted `ValueWord::from_row_view`), (3) for \
         correlated mode, build the ctx TypedObject as `Arc::into_raw\
         (Arc<TypedObjectStorage>) + push_kinded(bits, NativeKind::Ptr\
         (HeapKind::TypedObject))` against the predeclared cache schema \
         (replaces deleted `ValueWord::from_heap_value`), (4) thread the \
         (row|ctx, state, idx) KindedSlot argument list through op_call_value\
         (replaces deleted `call_value_immediate_raw` raw-bits closure-call \
         API), (5) replace every `shape_value::vw_clone(bits)` retain-on-read \
         site with `clone_with_kind(bits, kind)` per CLAUDE.md Forbidden \
         Patterns #8, and (6) emit the return TypedObject via Arc::into_raw \
         + push_kinded against `vm.builtin_schemas.simulate_return` per \
         playbook §3."
            .to_string(),
    ))
}
