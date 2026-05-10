//! Iterator method handlers — Phase 2c rebuild stubs.
//!
//! ## Status
//!
//! This module previously implemented `Iterator` PHF method dispatch
//! (`map`/`filter`/`take`/`skip`/`flatMap`/`enumerate`/`chain`/
//! `collect`/`forEach`/`reduce`/`count`/`any`/`all`/`find`) plus the
//! receiver-bound `iter()` factories (`Array.iter`, `String.iter`,
//! `Range.iter`, `HashMap.iter`) on top of two now-deleted substrates:
//!
//! - **`shape_value::heap_value::IteratorState` /
//!   `shape_value::heap_value::IteratorTransform`** — the lazy iterator
//!   state machine that backed `HeapValue::Iterator`. Both types and the
//!   `HeapValue::Iterator` variant were deleted during the strict-typing
//!   bulldozer cycles. There is no `Arc<IteratorState>` payload, no
//!   `HeapValue::Iterator(Arc<...>)`, and no `NativeKind::Ptr(HeapKind::Iterator)`
//!   in the post-§2.7.7 model.
//! - **`shape_value::ValueWord` / `ValueWordExt` / `value_word_drop` /
//!   `tag_bits` / `unified_array` / `ValueBits`** — the v1 dynamic-tag
//!   carrier and its companion helpers (all deleted). Every constructor
//!   (`from_iterator`, `from_array`, `from_i64`, `from_string`,
//!   `from_bool`, `none`, `from_raw_bits`), every accessor
//!   (`as_iterator`, `as_any_array`, `as_number_coerce`, `as_heap_ref`,
//!   `is_function`, `is_module_function`, `is_heap`, `heap_kind`,
//!   `raw_bits`, `into_raw_bits`, `clone_from_bits`, `type_name`), and
//!   every owning-share helper (`vw_drop`, `vw_clone`) were deleted.
//!
//! ## Disposition (W9-iterator-methods, Wave-γ continuation)
//!
//! Per `docs/cluster-audits/wave-9-method-refill-playbook.md` §4
//! (surface-and-stop triggers) and ADR-006 §2.7.4 (Phase-2c deferral
//! pattern): every PHF-bound entry below is **live** in
//! `executor/objects/method_registry.rs` (`ARRAY_METHODS["iter"]`,
//! `RANGE_METHODS["iter"]`, the entire `ITERATOR_METHODS` map). The
//! handlers therefore cannot simply be deleted — the PHF map is OOR and
//! a missing symbol breaks the registry build. Each handler ships with
//! the §2.7.10/Q11 kinded `MethodFnV2` ABI signature
//! (`fn(&mut VirtualMachine, &[KindedSlot], Option<&mut ExecutionContext>)
//! -> Result<KindedSlot, VMError>`) but the **body** is a
//! `NotImplemented(SURFACE: §2.7.4 — IteratorState rebuild)` stub: the
//! `HeapValue::Iterator(Arc<IteratorState>)` substrate is deleted (no
//! lazy-iterator carrier in the post-§2.7.6 heap layout), so there is
//! nothing to dispatch on. This is the W9 playbook §4 surface-and-stop
//! case — body genuinely depends on Phase-2c work (the typed iterator
//! carrier + transform-chain Arcs).
//!
//! Note: §2.7.10/Q11 (kinded MethodFnV2 ABI) **has landed**; the
//! signature here is already kinded. The surface is exclusively the
//! body-side IteratorState rebuild dependency.
//!
//! The two pre-existing `pub` helpers (`iter_source_len`,
//! `iter_source_element_at`) were called by `executor/loops/mod.rs`
//! (`B10-loops-heap` territory) and `executor/tests/iterator_ops.rs`
//! (test harness, OOR). Both consumer files are pre-existing
//! Wave-α-broken: `loops/mod.rs` references the deleted
//! `HeapValue::Iterator(state)` arm and `as_heap_ref()` /
//! `ValueWord::none()`; the test file references deleted
//! `ValueWord::from_heap_value` / `HeapValue::Range`. Re-emitting
//! either helper requires the deleted `IteratorState` / `ValueWord`
//! types, so they are deleted from this module — the consumer-side
//! breakage is documented in `B10-loops-heap` (loops) and the test
//! harness rebuild (Phase 2c). Re-introducing them here under any
//! signature (renamed, gated, "for one edge case") would be a
//! `CLAUDE.md` "Renames to refuse on sight" defection.
//!
//! Re-implementing the iterator pipeline post-Phase-2c will rebuild on:
//! - A typed iterator carrier (likely `HeapValue::TypedIterator(Arc<...>)`
//!   with a parallel `NativeKind::Ptr(HeapKind::TypedIterator)`),
//! - Per-source kinded element fetch (no `as_heap_ref` polymorphism —
//!   dispatch via `slot.as_heap_value()` + `HeapValue::*` match per Q8),
//! - Kinded transform-chain Arcs (no `value_word_drop` retain/release —
//!   `clone_with_kind` / `drop_with_kind` per ADR-006 §2.7.7).
//!
//! See `docs/adr/006-value-and-memory-model.md` §2.7.4, §2.7.6, §2.7.7,
//! §2.7.10/Q11, and `CLAUDE.md` "Forbidden Patterns" + "Renames to
//! refuse on sight".

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

// Shared SURFACE message for every iterator-method handler in this
// module. The kinded `MethodFnV2` ABI (§2.7.10/Q11) has landed — the
// signature here is already kinded. The body-side blocker is §2.7.4:
// the `HeapValue::Iterator(Arc<IteratorState>)` substrate (and the
// companion `IteratorTransform` chain) was deleted in Phase-1.A;
// there is no lazy-iterator carrier in the post-§2.7.6 heap layout to
// dispatch on, and no kinded element-fetch / transform-chain Arc
// shape exists yet. Phase-2c will rebuild on a typed iterator carrier
// (`HeapValue::TypedIterator(Arc<...>)` + `NativeKind::Ptr(HeapKind::
// TypedIterator)`); until that lands every body in this module must
// surface — never paper over with a Bool-default fallback (§2.7.7 #9)
// or by reintroducing `ValueWord::from_iterator` / `as_iterator` /
// `IteratorState` under any name (CLAUDE.md "Renames to refuse on
// sight"). W9-iterator-methods playbook §4 surface-and-stop case.
#[inline]
fn iterator_phase2c_surface(handler: &'static str) -> VMError {
    VMError::NotImplemented(format!(
        "{handler} — SURFACE: ADR-006 §2.7.4 — Phase-2c IteratorState \
         rebuild. The `HeapValue::Iterator(Arc<IteratorState>)` substrate \
         and `IteratorTransform` chain were deleted in Phase-1.A; the \
         post-§2.7.6 heap layout has no lazy-iterator carrier to dispatch \
         on. The §2.7.10/Q11 kinded `MethodFnV2` ABI has already landed \
         (signature is `&[KindedSlot] -> Result<KindedSlot, VMError>`), \
         so the surface is exclusively body-side: re-implementation \
         needs the typed iterator carrier (likely \
         `HeapValue::TypedIterator(Arc<...>)` + \
         `NativeKind::Ptr(HeapKind::TypedIterator)`), kinded per-source \
         element fetch via `slot.as_heap_value()` + `HeapValue::*` match \
         (Q8 single-discriminator), and kinded transform-chain Arcs via \
         `clone_with_kind` / `drop_with_kind` (§2.7.7). Reintroducing \
         `IteratorState` / `ValueWord::from_iterator` / `as_iterator` \
         under any name is a CLAUDE.md \"Renames to refuse on sight\" \
         defection."
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Receiver-bound iter() factories (live in ARRAY_METHODS / RANGE_METHODS / ...)
// ═══════════════════════════════════════════════════════════════════════════

/// Range.iter() — live `RANGE_METHODS["iter"]`. Phase-2c stub.
pub fn v2_range_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("v2_range_iter"))
}

/// Array.iter() — live `ARRAY_METHODS["iter"]`. Phase-2c stub.
pub(crate) fn handle_array_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_array_iter"))
}

/// String.iter() — live `STRING_METHODS["iter"]` (when wired). Phase-2c stub.
pub(crate) fn handle_string_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_string_iter"))
}

/// Range.iter() — alternate handler binding. Phase-2c stub.
pub(crate) fn handle_range_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_range_iter"))
}

/// HashMap.iter() — Phase-2c stub.
pub(crate) fn handle_hashmap_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_hashmap_iter"))
}

// ═══════════════════════════════════════════════════════════════════════════
// ITERATOR_METHODS PHF entries (all live in method_registry.rs:400-417)
// ═══════════════════════════════════════════════════════════════════════════

// ── Lazy transforms (return new Iterator) ─────────────────────────────────

/// Iterator.map(fn) — Phase-2c stub.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_map"))
}

/// Iterator.filter(fn) — Phase-2c stub.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_filter"))
}

/// Iterator.take(n) — Phase-2c stub.
pub(crate) fn handle_take(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_take"))
}

/// Iterator.skip(n) — Phase-2c stub.
pub(crate) fn handle_skip(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_skip"))
}

/// Iterator.flatMap(fn) — Phase-2c stub.
pub(crate) fn handle_flat_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_flat_map"))
}

/// Iterator.enumerate() — Phase-2c stub.
pub(crate) fn handle_enumerate(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_enumerate"))
}

/// Iterator.chain(other) — Phase-2c stub.
pub(crate) fn handle_chain(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_chain"))
}

// ── Terminal operations (consume the iterator) ────────────────────────────

/// Iterator.collect() / Iterator.toArray() — Phase-2c stub.
pub(crate) fn handle_collect(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_collect"))
}

/// Iterator.forEach(fn) — Phase-2c stub.
pub(crate) fn handle_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_for_each"))
}

/// Iterator.reduce(fn, init) — Phase-2c stub.
pub(crate) fn handle_reduce(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_reduce"))
}

/// Iterator.count() — Phase-2c stub.
pub(crate) fn handle_count(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_count"))
}

/// Iterator.any(fn) — Phase-2c stub.
pub(crate) fn handle_any(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_any"))
}

/// Iterator.all(fn) — Phase-2c stub.
pub(crate) fn handle_all(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_all"))
}

/// Iterator.find(fn) — Phase-2c stub.
pub(crate) fn handle_find(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(iterator_phase2c_surface("handle_find"))
}
