//! Method handlers for the Set collection type.
//!
//! W9-set-methods (Phase 1.B-vm Wave 9): bodies remain
//! `NotImplemented(SURFACE)` per Wave 9 playbook §4 surface-and-stop
//! triggers — `Set` has no live heap representation so no method body has
//! a kinded receiver to dispatch on. ADR-006 §2.7.4 (Phase-2c deferral)
//! applies to every entry in this file.
//!
//! ## Audit findings (W9 cluster owner, 2026-05-10)
//!
//! 1. `HeapKind` enumeration has no `Set` variant
//!    (`crates/shape-value/src/heap_variants.rs`). `HeapValue` likewise
//!    has no `Set` arm. The Phase-2 ValueWord bulldozer removed the
//!    pre-existing `HeapValue::Set { items: Vec<ValueWord> }` payload
//!    along with the rest of the heterogeneous-element collections.
//! 2. `BuiltinFunction::SetCtor` exists in the bytecode opcode table
//!    (`bytecode/opcode_defs.rs:2268`) but the executor body in
//!    `vm_impl/builtins.rs:491` is itself a `todo!()` ("phase-1b-vm
//!    wave 5e — collection ctor body migration pending"). Set values
//!    cannot reach a method handler from any execution path today, so
//!    even the `args[0]` receiver is unreachable.
//! 3. The Wave 9 playbook (§1 recipe) prescribes `args[0].slot
//!    .as_heap_value()` receiver classification followed by
//!    `vm.call_value_immediate_nb` for closure-callback ops
//!    (`forEach` / `map` / `filter`); the precondition for both is a
//!    surviving `HeapValue::Set` arm.
//!
//! ## Replacement design space (out of W9 scope)
//!
//! Two paths are coherent with ADR-006 §2.3 typed-Arc and ADR-005 §1
//! single-discriminator discipline:
//!
//! - **Path A — `Arc<HashSetData>` adjacent to Stage C P1(b)
//!   `HashMapData`.** Same insertion-ordered `TypedBuffer<Arc<String>>`
//!   keys + bucket-index hash store, no values buffer. Closure-callback
//!   ops (`map` / `filter` / `forEach`) iterate the keys buffer and
//!   dispatch via `call_value_immediate_nb`. Mutation ops (`add` /
//!   `delete`) need the **HashMapData typed-buffer mutation API**
//!   follow-up: `Arc::make_mut` over the inner
//!   `TypedBuffer<Arc<String>>` plus a parallel rebuild of the
//!   bucket index — neither HashMap nor Set has a mutation entry-point
//!   today (`HashMapData` is documented as immutable at the marshal
//!   boundary, see `heap_value.rs:577`). This is the cluster of work
//!   tracked as "HashMapData typed-buffer mutation API" in the Phase-2c
//!   backlog.
//! - **Path B — Monomorphized `TypedSet<T>` per element kind.** Mirrors
//!   `TypedArrayData::*` arms with a hash-side index for O(1) `has`.
//!   Wider surface (one variant per element kind) but cleaner kind
//!   discipline for non-string element types.
//!
//! Either choice is a Phase-2c Stage C decision and an ADR-006
//! amendment, not a Wave-β / Wave 9 migration.
//!
//! Per Wave 9 playbook §3 (forbidden) #6, every entry in this file
//! carries an explicit ADR-006 §2.7.4 surface comment plus the
//! "HashMapData typed-buffer mutation API" follow-up reference (see
//! `surface()` below).

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

/// Build the canonical SURFACE error for every entry-point in this file.
///
/// ADR-006 §2.7.4 deferral: no live `HeapKind::Set` /
/// `HeapValue::Set` arm exists, so receiver classification (Wave 9
/// playbook §1 step 1) cannot run. Reintroduction is gated on the
/// "HashMapData typed-buffer mutation API" follow-up plus an ADR-006
/// amendment selecting between Path A (`Arc<HashSetData>`) and Path B
/// (`TypedSet<T>` per element kind) — see file-level comment.
#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Set.{}(): no surviving HeapKind::Set / HeapValue::Set \
         arm (ADR-006 §2.3 trim, §2.7.4 deferral). Reintroducing Set \
         requires (1) a typed-Arc heap variant — Path A `Arc<HashSetData>` \
         adjacent to Stage C P1(b) HashMapData, or Path B `TypedSet<T>` \
         per element kind — and (2) the \"HashMapData typed-buffer \
         mutation API\" follow-up so add/delete/etc. can reach the inner \
         `TypedBuffer` via `Arc::make_mut` and rebuild the bucket index. \
         Closure-callback ops (forEach/map/filter) additionally need a \
         live receiver to dispatch through `call_value_immediate_nb` per \
         ADR-006 §2.7.11. Tracked as Phase-2c Stage C, not a Wave-9 \
         migration; see `set_methods.rs` file-level audit.",
        method
    ))
}

pub fn v2_add(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("add"))
}

pub fn v2_has(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("has"))
}

pub fn v2_delete(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("delete"))
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

pub fn v2_union(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("union"))
}

pub fn v2_intersection(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("intersection"))
}

pub fn v2_difference(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("difference"))
}

pub fn v2_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("forEach"))
}

pub fn v2_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("map"))
}

pub fn v2_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("filter"))
}
