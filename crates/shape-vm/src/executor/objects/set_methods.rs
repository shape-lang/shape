//! Method handlers for the Set collection type.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Set` is **not** a surviving `HeapKind` variant per ADR-006 §2.3 trim
//! (`crates/shape-value/src/heap_variants.rs`); the heterogeneous-element
//! `SetData` payload depended on the deleted `ValueWord` per-element
//! representation. Re-introducing Set requires a typed-Arc replacement —
//! either a monomorphized `TypedSet<T>` per element kind (mirroring
//! `TypedArrayData`) or a kinded `Arc<HashSetData>` adjacent to the new
//! `HashMapData` shape (Stage C P1(b), 2026-05-07). Either path is a
//! Phase 2c Stage C item, not a Wave-β migration.
//!
//! The pre-Wave-6 implementation used the deleted `ValueWord::from_set`,
//! `as_set_mut`, `raw_helpers::extract_set` (deleted in cluster
//! D-raw-helpers), `value_word_drop::vw_drop` / `vw_clone`,
//! `vmarray_from_vec`, plus the kindless MethodHandler ABI. Per playbook
//! §4 #1 / #9 a Bool-default kinded shim is forbidden; per §7.4 the
//! correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Set.{}(): Set is not a surviving HeapKind variant per \
         ADR-006 §2.3 trim; needs typed-Arc replacement (TypedSet<T> or \
         Arc<HashSetData> per Stage C model). MethodHandler ABI also needs \
         kinded migration (cluster E-builtins-backlog, Wave 5b template).",
        method
    ))
}

pub fn v2_add(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("add"))
}

pub fn v2_has(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("has"))
}

pub fn v2_delete(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("delete"))
}

pub fn v2_size(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("size"))
}

pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isEmpty"))
}

pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toArray"))
}

pub fn v2_union(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("union"))
}

pub fn v2_intersection(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("intersection"))
}

pub fn v2_difference(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("difference"))
}

pub fn v2_for_each(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("forEach"))
}

pub fn v2_map(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("map"))
}

pub fn v2_filter(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("filter"))
}
