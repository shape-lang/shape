//! Method handlers for the PriorityQueue collection type.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `PriorityQueue` is **not** a surviving `HeapKind` variant per ADR-006
//! §2.3 trim (`crates/shape-value/src/heap_variants.rs`); the
//! heterogeneous-element `PriorityQueueData` payload depended on the
//! deleted `ValueWord` per-element representation. Re-introducing
//! PriorityQueue requires a typed-Arc replacement — a monomorphized
//! `TypedPriorityQueue<T>` per element kind (mirroring `TypedArrayData`).
//! That is a Phase 2c Stage C item, not a Wave-β migration.
//!
//! The pre-Wave-6 implementation used the deleted
//! `ValueWord::from_priority_queue`, `as_priority_queue_mut`,
//! `raw_helpers::extract_priority_queue` (deleted in cluster
//! D-raw-helpers), `vmarray_from_vec`, plus the kindless MethodHandler
//! ABI. Per playbook §4 #1 / #9 a Bool-default kinded shim is forbidden;
//! per §7.4 the correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — PriorityQueue.{}(): PriorityQueue is not a surviving \
         HeapKind variant per ADR-006 §2.3 trim; needs typed-Arc replacement \
         (TypedPriorityQueue<T>). MethodHandler ABI also needs kinded \
         migration (cluster E-builtins-backlog, Wave 5b template).",
        method
    ))
}

pub fn v2_push(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("push"))
}

pub fn v2_pop(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("pop"))
}

pub fn v2_peek(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("peek"))
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

pub fn v2_to_sorted_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toSortedArray"))
}
