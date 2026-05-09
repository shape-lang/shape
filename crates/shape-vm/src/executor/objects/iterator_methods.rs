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
//! ## Disposition (Wave-β `M-iterator`)
//!
//! Per `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10
//! (Wave-α/β file-list, M-iterator row) and ADR-006 §2.7.4 (Phase-2c
//! deferral pattern): every PHF-bound entry below is **live** in
//! `executor/objects/method_registry.rs` (`ARRAY_METHODS["iter"]`,
//! `RANGE_METHODS["iter"]`, the entire `ITERATOR_METHODS` map). The
//! handlers therefore cannot simply be deleted — the PHF map is OOR
//! and a missing symbol breaks the registry build. Instead each live
//! handler is rebuilt as a `NotImplemented(SURFACE: …)` stub that
//! preserves the `MethodFnV2` shape (`fn(&mut VirtualMachine, &mut [u64],
//! Option<&mut ExecutionContext>) -> Result<u64, VMError>`).
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
//! Q8, and `CLAUDE.md` "Forbidden Patterns" + "Renames to refuse on sight".

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

// Pre-§2.7.9 surface helper `surface() -> Result<u64, VMError>` deleted
// — the kinded ABI landed per ADR-006 §2.7.9 / Q11, so the helper's
// return type no longer matches `MethodFnV2`. Each SURFACE handler body
// carries the §2.7.9-aware migration contract inline. Wave-γ-followup
// territory (M-iterator-class).

// ═══════════════════════════════════════════════════════════════════════════
// Receiver-bound iter() factories (live in ARRAY_METHODS / RANGE_METHODS / ...)
// ═══════════════════════════════════════════════════════════════════════════

/// Range.iter() — live `RANGE_METHODS["iter"]`. Phase-2c stub.
pub fn v2_range_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_range_iter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Array.iter() — live `ARRAY_METHODS["iter"]`. Phase-2c stub.
pub(crate) fn handle_array_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_array_iter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// String.iter() — live `STRING_METHODS["iter"]` (when wired). Phase-2c stub.
pub(crate) fn handle_string_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_string_iter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Range.iter() — alternate handler binding. Phase-2c stub.
pub(crate) fn handle_range_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_range_iter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.iter() — Phase-2c stub.
pub(crate) fn handle_hashmap_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_hashmap_iter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
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
    Err(VMError::NotImplemented(
        "handle_map — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.filter(fn) — Phase-2c stub.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_filter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.take(n) — Phase-2c stub.
pub(crate) fn handle_take(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_take — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.skip(n) — Phase-2c stub.
pub(crate) fn handle_skip(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_skip — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.flatMap(fn) — Phase-2c stub.
pub(crate) fn handle_flat_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_flat_map — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.enumerate() — Phase-2c stub.
pub(crate) fn handle_enumerate(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_enumerate — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.chain(other) — Phase-2c stub.
pub(crate) fn handle_chain(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_chain — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

// ── Terminal operations (consume the iterator) ────────────────────────────

/// Iterator.collect() / Iterator.toArray() — Phase-2c stub.
pub(crate) fn handle_collect(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_collect — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.forEach(fn) — Phase-2c stub.
pub(crate) fn handle_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_for_each — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.reduce(fn, init) — Phase-2c stub.
pub(crate) fn handle_reduce(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_reduce — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.count() — Phase-2c stub.
pub(crate) fn handle_count(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_count — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.any(fn) — Phase-2c stub.
pub(crate) fn handle_any(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_any — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.all(fn) — Phase-2c stub.
pub(crate) fn handle_all(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_all — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// Iterator.find(fn) — Phase-2c stub.
pub(crate) fn handle_find(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "handle_find — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}
