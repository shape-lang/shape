//! HashMap method handlers for the PHF method registry.
//!
//! ## Wave-δ MR-hashmap-readonly migration (2026-05-09)
//!
//! Per ADR-006 §2.7.10 / Q11 (Wave-γ G-method-fn-v2-abi close commit
//! `5091cba`) the `MethodFnV2` ABI is kinded: handlers take
//! `args: &[KindedSlot]` and return `Result<KindedSlot, VMError>`. This
//! Wave-δ pass migrates the 9 read-only handlers (`get`, `has`, `keys`,
//! `values`, `entries`, `len`, `isEmpty`, `getOrDefault`, `toArray`) to
//! real bodies on top of the post-§2.7.4 `HashMapData` shape
//! (`Arc<TypedBuffer<Arc<String>>>` keys + `Arc<TypedBuffer<Arc<HeapValue>>>`
//! values + eager bucket-index for O(1) lookup; see
//! `shape_value::heap_value::HashMapData`).
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::HashMap)`, then `args[0].slot.as_heap_value()`
//! pattern-matched against `HeapValue::HashMap(arc)` (single-discriminator
//! per ADR-005 §1 — no per-heap-variant `KindedSlot` accessor).
//!
//! Per-arg key kind classification follows the same shape: `args[1].kind`
//! against `NativeKind::String | NativeKind::Ptr(HeapKind::String)`, then
//! `as_str()` for the borrow.
//!
//! Result construction follows playbook §3:
//! - `len` / `isEmpty` → inline-scalar `KindedSlot::from_int` / `from_bool`.
//! - `get` / `getOrDefault` value-side → re-wrap the `Arc<HeapValue>` value
//!   into a per-FieldType `KindedSlot` arm (`from_string_arc`,
//!   `from_typed_array`, etc.) — same `heap_value_to_slot` pattern as
//!   `executor/builtins/array_ops.rs:156`.
//! - `keys` → `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)`.
//! - `values` → `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)`.
//! - `entries` / `toArray` → outer `TypedArrayData::HeapValue` with each
//!   element a 2-element inner array (`[key, value]`) wrapped as
//!   `Arc::new(HeapValue::TypedArray(Arc::new(TypedArrayData::HeapValue(
//!   ..))))` — same shape as `array_ops::builtin_zip` at line 376.
//!
//! ## Still-surfaced handlers (3 mutation + 1 iter + 5 closure-callback)
//!
//! - **`set` / `delete` / `merge`** remain `NotImplemented(SURFACE: phase-2c
//!   — HashMapData typed-buffer mutation API rebuild)`. The post-§2.7.4
//!   `HashMapData` deliberately drops the legacy mutation API:
//!   `Arc::make_mut`-driven `keys.push` / `values.push` / shape-id transition
//!   was deleted alongside the `Vec<ValueWord>` storage shape. Buffer-aware
//!   insert / remove / merge over `Arc<TypedBuffer<…>>` is a phase-2c
//!   workstream tracked under ADR-006 §2.7.4 + the homogeneous-typed
//!   HashMap workstream (see also `executor/objects/typed_access.rs:202`'s
//!   `MapSetStrI64` SURFACE referencing the same gap).
//!
//! - **`iter`** remains `NotImplemented(SURFACE: phase-2c)` — the legacy
//!   `IteratorState` struct is deleted; the kinded HashMap-iteration shape
//!   (per-entry key + value kind dispatch over the typed buffers) is a
//!   phase-2c follow-up under ADR-006 §2.7.4.
//!
//! - **`forEach` / `filter` / `map` / `reduce` / `groupBy`** remain
//!   `NotImplemented(SURFACE: phase-2c — closure-call kinded dispatch
//!   rebuild)`. Same architectural blocker as the M-datatable Wave-β
//!   `joins.rs` handlers (commit `eb78699`): the closure-callback path
//!   would need `vm.call_value_immediate_*` to thread `&[KindedSlot]` /
//!   return `KindedSlot`, but every entry point in `call_convention.rs`
//!   (`call_value_immediate_nb`, `call_value_immediate_raw`,
//!   `call_function_with_raw_args`, `call_closure_with_raw_args`,
//!   `call_function_from_stack`, `call_closure_with_nb_args`,
//!   `call_closure_with_nb_args_keepalive`, `jit_trampoline_call_closure`)
//!   is itself a `todo!("phase-2c — ADR-006 §2.7.8 cluster B-round-2: …
//!   kinded-arg + kinded-cell rebuild pending")`. Surface and stop per
//!   playbook §8 cross-cluster cascade — do not paper over with a
//!   forbidden-pattern call path.
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / Q8 / Q11 + playbook §3 / §7 / §10.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HashMapData, HeapKind, HeapValue, TypedArrayData};
use shape_value::typed_buffer::TypedBuffer;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to the inner `Arc<HashMapData>` via
/// the §2.7.6 / Q8 single-discriminator path: kind gate on
/// `Ptr(HeapKind::HashMap)`, then `slot.as_heap_value()` matched against
/// `HeapValue::HashMap(arc)`. The receiver retains its share — the caller
/// borrows through the `&Arc<HashMapData>` and never decrements.
#[inline]
fn as_hashmap(slot: &KindedSlot) -> Result<&Arc<HashMapData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::HashMap)) {
        return Err(type_error(format!(
            "HashMap method receiver must be a HashMap (got kind {:?})",
            slot.kind
        )));
    }
    match slot.slot.as_heap_value() {
        HeapValue::HashMap(arc) => Ok(arc),
        other => Err(type_error(format!(
            "HashMap method receiver kind says HashMap but heap arm is {:?}",
            other.kind()
        ))),
    }
}

/// Borrow a `&str` key from a `KindedSlot` whose kind is `String` /
/// `Ptr(HeapKind::String)`. Returns a `TypeError` for non-string kinds.
/// The slot retains its share — the borrow is bounded by the carrier's
/// lifetime per `KindedSlot::as_str` / the §2.7.6 / Q8 dispatch path.
#[inline]
fn as_string_key(slot: &KindedSlot) -> Result<&str, VMError> {
    match slot.kind {
        NativeKind::String => slot
            .as_str()
            .ok_or_else(|| type_error("HashMap key kind=String but slot bits null")),
        NativeKind::Ptr(HeapKind::String) => match slot.slot.as_heap_value() {
            HeapValue::String(s) => Ok(s.as_str()),
            _ => Err(type_error(
                "HashMap key kind=Ptr(String) but heap arm mismatched",
            )),
        },
        _ => Err(type_error(format!(
            "HashMap key must be a string (got kind {:?})",
            slot.kind
        ))),
    }
}

/// Convert an `Arc<HeapValue>` value (as stored in `HashMapData::values`)
/// to a `KindedSlot` via the matching per-FieldType constructor. Mirrors
/// `executor/builtins/array_ops.rs::heap_value_to_slot` (the canonical
/// `TypedArrayData::HeapValue` element re-wrapping path).
fn heap_value_arc_to_slot(hv: &Arc<HeapValue>) -> KindedSlot {
    match hv.as_ref() {
        HeapValue::String(s) => KindedSlot::from_string_arc(Arc::clone(s)),
        HeapValue::Decimal(d) => KindedSlot::from_decimal(Arc::clone(d)),
        HeapValue::BigInt(b) => KindedSlot::from_bigint(Arc::clone(b)),
        HeapValue::TypedArray(a) => KindedSlot::from_typed_array(Arc::clone(a)),
        HeapValue::TypedObject(o) => KindedSlot::from_typed_object(Arc::clone(o)),
        HeapValue::HashMap(m) => KindedSlot::from_hashmap(Arc::clone(m)),
        HeapValue::Char(c) => KindedSlot::from_char(*c),
        // Other heap arms — fall back to `none()` (Bool-kind null sentinel).
        // Same coverage gap as `array_ops::heap_value_to_slot`; widening is
        // tracked alongside the `KindedSlot::from_*` constructor build-out.
        _ => KindedSlot::none(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Read-only handlers — Wave-δ MR-hashmap-readonly migration
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.get(key) -> value | none
pub fn v2_get(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.get() requires exactly 1 argument (key)",
        ));
    }
    let map = as_hashmap(&args[0])?;
    let key = as_string_key(&args[1])?;
    match map.get(key) {
        Some(value_arc) => Ok(heap_value_arc_to_slot(value_arc)),
        None => Ok(KindedSlot::none()),
    }
}

/// HashMap.has(key) -> bool
pub fn v2_has(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.has() requires exactly 1 argument (key)",
        ));
    }
    let map = as_hashmap(&args[0])?;
    let key = as_string_key(&args[1])?;
    Ok(KindedSlot::from_bool(map.contains_key(key)))
}

/// HashMap.keys() -> Array<string>
///
/// Returns an `Array<string>` reusing the receiver's keys buffer
/// (`Arc<TypedBuffer<Arc<String>>>`) — single Arc bump on the buffer, no
/// per-element clone. Wraps the buffer as
/// `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)`.
pub fn v2_keys(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.keys() takes no arguments"));
    }
    let map = as_hashmap(&args[0])?;
    let arr = TypedArrayData::String(Arc::clone(&map.keys));
    Ok(KindedSlot::from_typed_array(Arc::new(arr)))
}

/// HashMap.values() -> Array<value>
///
/// Returns an `Array<heap>` reusing the receiver's values buffer
/// (`Arc<TypedBuffer<Arc<HeapValue>>>`) — single Arc bump on the buffer.
/// Wraps as `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)`.
pub fn v2_values(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.values() takes no arguments"));
    }
    let map = as_hashmap(&args[0])?;
    let arr = TypedArrayData::HeapValue(Arc::clone(&map.values));
    Ok(KindedSlot::from_typed_array(Arc::new(arr)))
}

/// HashMap.entries() -> Array<[key, value]>
///
/// Each entry is a 2-element inner array `[key, value]` stored as
/// `HeapValue::TypedArray(Arc<TypedArrayData::HeapValue>)`. The outer
/// array is a `TypedArrayData::HeapValue` of those `Arc<HeapValue>` entries.
/// Same shape as `array_ops::builtin_zip` (line 376).
pub fn v2_entries(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.entries() takes no arguments"));
    }
    build_entries_array(&args[0])
}

/// HashMap.len() -> int
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.len() takes no arguments"));
    }
    let map = as_hashmap(&args[0])?;
    Ok(KindedSlot::from_int(map.len() as i64))
}

/// HashMap.isEmpty() -> bool
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.isEmpty() takes no arguments"));
    }
    let map = as_hashmap(&args[0])?;
    Ok(KindedSlot::from_bool(map.is_empty()))
}

/// HashMap.getOrDefault(key, default) -> value
///
/// Same value-side dispatch as `get`; on miss the kinded `default` slot is
/// returned by clone (refcount bump for heap arms via `KindedSlot::clone`).
pub fn v2_get_or_default(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 3 {
        return Err(type_error(
            "HashMap.getOrDefault() requires exactly 2 arguments (key, default)",
        ));
    }
    let map = as_hashmap(&args[0])?;
    let key = as_string_key(&args[1])?;
    match map.get(key) {
        Some(value_arc) => Ok(heap_value_arc_to_slot(value_arc)),
        None => Ok(args[2].clone()),
    }
}

/// HashMap.toArray() -> Array<[key, value]>
///
/// Alias for `entries()`.
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.toArray() takes no arguments"));
    }
    build_entries_array(&args[0])
}

/// Shared body for `entries()` / `toArray()`. Builds the per-entry
/// 2-element inner arrays + the outer `TypedArrayData::HeapValue` wrapper.
fn build_entries_array(receiver: &KindedSlot) -> Result<KindedSlot, VMError> {
    let map = as_hashmap(receiver)?;
    let n = map.len();
    let mut pairs: Vec<Arc<HeapValue>> = Vec::with_capacity(n);
    for i in 0..n {
        let key_arc: Arc<String> = Arc::clone(&map.keys.data[i]);
        let value_arc: Arc<HeapValue> = Arc::clone(&map.values.data[i]);
        let inner = TypedArrayData::HeapValue(Arc::new(TypedBuffer::from_vec(vec![
            Arc::new(HeapValue::String(key_arc)),
            value_arc,
        ])));
        pairs.push(Arc::new(HeapValue::TypedArray(Arc::new(inner))));
    }
    let outer = TypedArrayData::HeapValue(Arc::new(TypedBuffer::from_vec(pairs)));
    Ok(KindedSlot::from_typed_array(Arc::new(outer)))
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation handlers — phase-2c HashMapData typed-buffer mutation API rebuild
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.set(key, value) -> HashMap
pub fn v2_set(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.set — SURFACE: phase-2c — HashMapData typed-buffer mutation \
         API rebuild. The post-§2.7.4 HashMapData (Arc<TypedBuffer<Arc<String>>> \
         keys + Arc<TypedBuffer<Arc<HeapValue>>> values + eager bucket-index) \
         deliberately dropped the legacy `Arc::make_mut`-driven `keys.push` / \
         `values.push` / shape-id transition path. Buffer-aware insert is a \
         phase-2c workstream tracked under ADR-006 §2.7.4 alongside the \
         homogeneous-typed HashMap workstream — see \
         `executor/objects/typed_access.rs:202` (MapSetStrI64) for the \
         canonical SURFACE referencing the same gap."
            .into(),
    ))
}

/// HashMap.delete(key) -> HashMap
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.delete — SURFACE: phase-2c — HashMapData typed-buffer \
         mutation API rebuild. Same buffer-aware mutation gap as set(): the \
         post-§2.7.4 HashMapData has no remove path on Arc<TypedBuffer<…>>. \
         Tracked alongside the homogeneous-typed HashMap workstream under \
         ADR-006 §2.7.4."
            .into(),
    ))
}

/// HashMap.merge(other) -> HashMap
pub fn v2_merge(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.merge — SURFACE: phase-2c — HashMapData typed-buffer \
         mutation API rebuild. Merge is a bulk insert; same buffer-aware \
         mutation gap as set() / delete(). Tracked alongside the \
         homogeneous-typed HashMap workstream under ADR-006 §2.7.4."
            .into(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Closure-based handlers — phase-2c closure-call kinded dispatch rebuild
// ═══════════════════════════════════════════════════════════════════════════
//
// The closure-callback path needs `vm.call_value_immediate_*` to thread
// `&[KindedSlot]` / return `KindedSlot`, but every entry point in
// `call_convention.rs` is itself a phase-2c `todo!()` (see
// `call_value_immediate_nb`, `call_value_immediate_raw`,
// `call_function_with_raw_args`, `call_closure_with_raw_args`,
// `call_function_from_stack`, `call_closure_with_nb_args`,
// `call_closure_with_nb_args_keepalive`, `jit_trampoline_call_closure`).
// Surface and stop per playbook §8 cross-cluster cascade — same precedent
// as the M-datatable Wave-β `joins.rs` close (commit `eb78699`).

/// HashMap.forEach(fn(key, value)) -> unit
pub fn v2_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.forEach — SURFACE: phase-2c — closure-call kinded dispatch \
         rebuild (ADR-006 §2.7.8 cluster B-round-2). The callback path needs \
         `vm.call_value_immediate_*` to thread `&[KindedSlot]` / return \
         `KindedSlot`, but every call_convention.rs entry point is itself a \
         phase-2c `todo!()`. Same architectural blocker as the M-datatable \
         Wave-β `joins.rs` close (commit `eb78699`)."
            .into(),
    ))
}

/// HashMap.filter(fn(key, value) -> bool) -> HashMap
pub fn v2_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.filter — SURFACE: phase-2c — closure-call kinded dispatch \
         rebuild (ADR-006 §2.7.8 cluster B-round-2). Same blocker as \
         forEach: the predicate-callback path depends on the kinded \
         call_value rebuild plus the buffer-aware HashMapData construction \
         path that the mutation handlers also need (the filter result is a \
         new HashMap built from a subset of the entries). Surface and stop \
         per playbook §8."
            .into(),
    ))
}

/// HashMap.map(fn(key, value) -> new_value) -> HashMap
pub fn v2_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.map — SURFACE: phase-2c — closure-call kinded dispatch \
         rebuild (ADR-006 §2.7.8 cluster B-round-2). Same blocker as \
         forEach/filter: closure-callback path + buffer-aware HashMapData \
         construction (transformed-values map) both pending. Surface and \
         stop per playbook §8."
            .into(),
    ))
}

/// HashMap.reduce(fn(acc, key, value) -> acc, initial) -> value
pub fn v2_reduce(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.reduce — SURFACE: phase-2c — closure-call kinded dispatch \
         rebuild (ADR-006 §2.7.8 cluster B-round-2). Same blocker as \
         forEach: the accumulator-callback path depends on the kinded \
         call_value rebuild — the result is a single accumulated value, no \
         HashMap construction needed, but the per-entry callback still \
         routes through the deleted `call_value_immediate_*` path. Surface \
         and stop per playbook §8."
            .into(),
    ))
}

/// HashMap.groupBy(fn(key, value) -> group_key) -> HashMap<group_key, HashMap>
pub fn v2_group_by(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.groupBy — SURFACE: phase-2c — closure-call kinded dispatch \
         rebuild (ADR-006 §2.7.8 cluster B-round-2). Compounds the \
         forEach/filter blocker: groupBy needs both the kinded call_value \
         rebuild AND the buffer-aware HashMapData construction path (it \
         produces nested HashMaps grouped by the key-extractor's return \
         value). Surface and stop per playbook §8."
            .into(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Iterator handler — phase-2c IteratorState rebuild
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.iter() -> Iterator
pub fn v2_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "HashMap.iter — SURFACE: phase-2c (ADR-006 §2.7.4). The legacy \
         IteratorState struct is deleted; the kinded HashMap-iteration \
         shape (per-entry key + value kind dispatch over the typed buffers) \
         is a phase-2c follow-up workstream."
            .into(),
    ))
}

// ─── Tests ───────────────────────────────────────────────────────────
//
// The pre-Wave-β unit tests exercised the legacy `Vec<ValueWord>`
// HashMapData shape (`HashMapData::find_key`, `rebuild_index`,
// `vw_hash` / `vw_equals` collision-bucket chaining,
// `as_hashmap` / `from_hashmap_pairs` / `empty_hashmap`, and the
// per-handler `v2_*` bodies). Every dependency is deleted with the
// strict-typing bulldozer. Wave-δ MR-hashmap-readonly tests are
// exercised via the bytecode-level integration suites once the dispatch
// shell (`op_call_method`) wires through to these handlers; the local
// unit-test harness re-instatement is a phase-2c follow-up tracked
// under cluster `E-builtins-backlog` alongside the sibling typed-array
// V2 handler test backfills.
