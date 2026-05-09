//! HashMap method handlers for the PHF method registry.
//!
//! ## Wave 6.5 Wave-β M-hashmap status
//!
//! Per the playbook §10 M-hashmap row + ADR-006 §2.7.6 / §2.7.7 / §2.7.4 +
//! the canonical D-array-joins surface template, every handler in this file
//! surfaces as `NotImplemented(SURFACE: ...)` rather than papering over a
//! forbidden-pattern dependency.
//!
//! The forbidden surface set this file's pre-Wave-β body relied on:
//!
//! - `raw_helpers::extract_hashmap` / `extract_hashmap_data` / `extract_str`
//!   (deleted as forbidden tag-decoding probes — D-raw-helpers swept these).
//! - `ValueWord` / `ValueWordExt` (deleted; `value_word_drop::vw_drop` /
//!   `vw_clone` likewise — playbook §4 forbidden #4 / #8).
//! - `HashMapData::find_key` / `rebuild_index` / `shape_id` / `shape_get` /
//!   in-place `Vec<ValueWord>` mutation via `as_hashmap_mut` (deleted with
//!   the `Vec<ValueWord>` storage shape; the new `HashMapData` carries
//!   `Arc<TypedBuffer<Arc<String>>>` keys and `Arc<TypedBuffer<Arc<HeapValue>>>`
//!   values with no insert/remove API).
//! - `vmarray_from_value_words` / `ValueWord::from_array` / `from_hashmap` /
//!   `from_hashmap_pairs` / `empty_hashmap` / `from_iterator` /
//!   `IteratorState` (deleted constructors and the iterator-state struct).
//! - `vm.call_value_immediate_raw` (still resident in `call_convention.rs`
//!   but body uses forbidden `tag_bits::*` + `ValueWord::from_raw_bits` —
//!   itself a downstream cluster cascade).
//!
//! ## Why every handler surfaces
//!
//! 1. **Read methods (`get`, `has`, `keys`, `values`, `entries`, `len`,
//!    `isEmpty`, `getOrDefault`, `toArray`).** The `MethodFnV2` ABI accepts
//!    `args: &mut [u64]` with **no parallel `NativeKind` track** — the
//!    canonical D-array-joins gap. Receiver-side, this file would need to
//!    treat `args[0]` as `Arc::into_raw(Arc<HashMapData>)`; that's a
//!    dispatcher-invariant assumption, but the same ABI gap blocks every
//!    *value-side* push (returning a `keys()` Array of `Arc<String>` keys
//!    requires per-element `NativeKind::String`, returning a `get()` value
//!    of `Arc<HeapValue>` requires a per-`HeapValue::*` arm dispatch to a
//!    `NativeKind::Ptr(HeapKind::*)` push — both of which the V2 ABI's
//!    return-type `Result<u64, VMError>` discards). Bool-defaulting any
//!    push kind is the W-series rationalization the playbook names
//!    verbatim (§2.7.7 #9 / playbook §4 row 9).
//!
//! 2. **Mutation methods (`set`, `delete`, `merge`).** The post-§2.7.4
//!    `HashMapData` (Stage C P1(b) — see `shape_value::heap_value::HashMapData`)
//!    deliberately drops the legacy mutation API. Insertion is via
//!    `HashMapData::from_pairs`, lookup is `&self`-only via `get` /
//!    `contains_key`, and there is no `Arc::make_mut`-driven `keys.push` /
//!    `values.push` / shape-id transition. The buffer-aware insert/remove
//!    path is a phase-2c rewrite tracked alongside the homogeneous-typed
//!    HashMap workstream (see `executor/objects/typed_access.rs:202` for
//!    the canonical `MapSetStrI64` SURFACE referencing the same gap).
//!
//! 3. **Closure-based methods (`forEach`, `filter`, `map`, `reduce`,
//!    `groupBy`).** Same `MethodFnV2` kind-blind ABI gap as D-array-joins
//!    (`crates/shape-vm/src/executor/objects/array_joins.rs`) — call-back
//!    through `op_call_value` requires a kinded callee + kinded args, and
//!    the closure-receiver entries in `args[1..]` cannot be classified
//!    without a parallel kind track.
//!
//! 4. **`iter`.** `IteratorState` is deleted (Phase-2c — the lazy iterator
//!    shape needs a kinded redesign per ADR-006 §2.7.4).
//!
//! ## Definition of done (playbook §7 REVISED)
//!
//! - **Zero shim hits, zero forbidden patterns introduced.** All `extract_*`,
//!   `ValueWord`, `vw_*`, `IteratorState`, `vmarray_from_value_words`,
//!   `from_hashmap*`, `as_hashmap*`, `shape_get`, `vw_hash`, `vw_equals`,
//!   `record_heap_write`, `write_barrier_vw` references in this file are
//!   removed (verified by grep at close).
//! - **Pre-existing migrated OFF.** The `HashMapData` legacy fields
//!   (`keys: Vec<ValueWord>`, `values: Vec<ValueWord>`, `index`,
//!   `shape_id`) are **not** referenced anywhere in this file — the file
//!   no longer has a body that depends on them.
//! - **Compiles OR placeholders.** Every handler returns
//!   `Err(VMError::NotImplemented(...))` with a documented SURFACE rationale.
//!   The file compiles standalone (no dependence on deleted symbols).
//!
//! Wave-β Wave 6.5 / ADR-006 §2.7.6 / §2.7.7 / §2.7.4 / Q8.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

// Pre-§2.7.9 surface helpers (`surface_kind_blind`, `surface_mutation_phase_2c`,
// `surface_closure_kind_blind`) deleted with their callers' bodies — the
// kinded ABI now in place per ADR-006 §2.7.9 / Q11 makes the previous
// surface rationale text stale ("MethodFnV2 ABI lacks parallel NativeKind
// track" was the gap; that gap is now closed at the type level). Each
// SURFACE handler body carries the §2.7.9-aware migration contract
// inline. Wave-γ-followup body migration territory.

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — every one is a documented SURFACE
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.get(key) -> value | none
///
/// SURFACE: returning a polymorphic `Arc<HeapValue>` value through the V2 ABI
/// requires a `NativeKind::Ptr(HeapKind::*)` arm per `HeapValue::*` variant
/// (ADR-005 §1 single-discriminator) — the V2 return type discards the kind.
pub fn v2_get(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_get — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.set(key, value) -> HashMap
///
/// SURFACE: post-Stage-C `HashMapData` has no insert API.
pub fn v2_set(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_set — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.has(key) -> bool
///
/// SURFACE: classifying `args[1]` as a `NativeKind::String` payload requires
/// the parallel kind track the V2 ABI lacks. Bool-defaulting the key kind
/// would be a §2.7.7 #9 forbidden pattern.
pub fn v2_has(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_has — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.delete(key) -> HashMap
///
/// SURFACE: post-Stage-C `HashMapData` has no remove API.
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_delete — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.keys() -> Array<string>
///
/// SURFACE: building an Array of `Arc<String>` keys requires per-element push
/// at `NativeKind::String` — the V2 ABI's `Result<u64>` return discards the
/// element kind required by the consuming Array.
pub fn v2_keys(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_keys — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.values() -> Array<value>
///
/// SURFACE: same gap as `keys()`, with `Arc<HeapValue>` values needing per-arm
/// `HeapValue::*` → `NativeKind::Ptr(HeapKind::*)` dispatch.
pub fn v2_values(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_values — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.entries() -> Array<[key, value]>
///
/// SURFACE: each entry tuple is `(Arc<String>, Arc<HeapValue>)`; both halves
/// need kinded pushes that the V2 ABI cannot emit.
pub fn v2_entries(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_entries — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.len() -> int
///
/// SURFACE: even though len() returns a scalar `Int64` whose kind is statically
/// known, the receiver classification at `args[0]` would lean on the
/// dispatcher's HashMap-kind invariant (the V2 ABI carries no kind track to
/// confirm). The handler surfaces uniformly with the rest of this file
/// rather than introduce a single dispatcher-invariant trust path that other
/// handlers cannot follow without forbidden patterns. Phase-2c: with a
/// kinded MethodFnV2 ABI, `len()` becomes a one-line `*const HashMapData`
/// borrow + `Int64` push.
pub fn v2_len(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_len — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.isEmpty() -> bool
///
/// SURFACE: same shape as `len()` — receiver classification trusts the
/// dispatcher invariant; uniform surface keeps the file a single
/// pattern.
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_is_empty — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.merge(other) -> HashMap
///
/// SURFACE: same as `set` / `delete` — no mutation API on the post-Stage-C
/// `HashMapData`, plus the kind-blind classification of `args[1]` (the other
/// HashMap receiver).
pub fn v2_merge(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_merge — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.getOrDefault(key, default) -> value
///
/// SURFACE: same as `get` — polymorphic `Arc<HeapValue>` return needs per-arm
/// `NativeKind::Ptr(HeapKind::*)` dispatch.
pub fn v2_get_or_default(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_get_or_default — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.toArray() -> Array<[key, value]>
///
/// SURFACE: alias for `entries()`, same kinded-push gap.
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_to_array — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Closure-based handlers — D-array-joins surface family
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.forEach(fn(key, value)) -> unit
pub fn v2_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_for_each — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.filter(fn(key, value) -> bool) -> HashMap
pub fn v2_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_filter — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.map(fn(key, value) -> new_value) -> HashMap
pub fn v2_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_map — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.reduce(fn(acc, key, value) -> acc, initial) -> value
pub fn v2_reduce(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_reduce — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// HashMap.groupBy(fn(key, value) -> group_key) -> HashMap<group_key, HashMap>
pub fn v2_group_by(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_group_by — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Iterator handler — IteratorState deleted, phase-2c
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.iter() -> Iterator
///
/// SURFACE: `IteratorState` is deleted with the legacy lazy-iterator shape.
/// The kinded redesign of HashMap iteration (per-element kind dispatch over
/// `Arc<String>` keys + `Arc<HeapValue>` values) is a phase-2c follow-up
/// tracked under ADR-006 §2.7.4.
pub fn v2_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.iter — SURFACE: phase-2c (ADR-006 §2.7.4). `IteratorState` \
         deleted; the kinded HashMap-iteration shape (per-entry key + value \
         kind dispatch) is a follow-up workstream."
            .into(),
    ))
}

// ─── Tests ───────────────────────────────────────────────────────────
//
// The pre-Wave-β unit tests exercised `HashMapData::find_key`,
// `HashMapData::rebuild_index`, `vw_hash` / `vw_equals` collision-bucket
// chaining, `as_hashmap` / `from_hashmap_pairs` / `empty_hashmap`, and the
// per-handler `v2_*` bodies. Every dependency is deleted (the legacy
// `Vec<ValueWord>` HashMapData shape, the `ValueWord` constructors, the
// `find_key` API, the iterator-state struct). Re-instated under cluster
// E-tests / phase-2c once the kinded MethodFnV2 ABI lands and the
// post-Stage-C HashMapData mutation API is wired.
