//! Method handlers for the Deque collection type.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Deque` is **not** a surviving `HeapKind` variant per ADR-006 §2.3 trim
//! (`crates/shape-value/src/heap_variants.rs`); the heterogeneous-element
//! `DequeData` payload depended on the deleted `ValueWord` per-element
//! representation. Re-introducing Deque requires a typed-Arc replacement —
//! a monomorphized `TypedDeque<T>` per element kind (mirroring
//! `TypedArrayData`). That is a Phase 2c Stage C item, not a Wave-β
//! migration.
//!
//! The pre-Wave-6 implementation used the deleted `ValueWord::from_deque`,
//! `as_deque_mut`, `raw_helpers::extract_deque` (deleted in cluster
//! D-raw-helpers), `vmarray_from_vec`, plus the kindless MethodHandler
//! ABI. Per playbook §4 #1 / #9 a Bool-default kinded shim is forbidden;
//! per §7.4 the correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Deque.{}(): Deque is not a surviving HeapKind variant per \
         ADR-006 §2.3 trim; needs typed-Arc replacement (TypedDeque<T>). \
         MethodHandler ABI also needs kinded migration (cluster \
         E-builtins-backlog, Wave 5b template).",
        method
    ))
}

pub fn v2_push_back(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("pushBack"))
}

pub fn v2_push_front(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("pushFront"))
}

pub fn v2_pop_back(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("popBack"))
}

pub fn v2_pop_front(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("popFront"))
}

pub fn v2_peek_back(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("peekBack"))
}

pub fn v2_peek_front(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("peekFront"))
}

pub fn v2_size(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("size"))
}

pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isEmpty"))
}

pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toArray"))
}

pub fn v2_get(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("get"))
}
