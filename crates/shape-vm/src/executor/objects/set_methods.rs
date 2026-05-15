//! Method handlers for the Set collection type.
//!
//! ## W13-hashset-rebuild migration (2026-05-10)
//!
//! Per ADR-006 §2.7.15 / Q16 amendment (Wave 13 W13-hashset-rebuild),
//! the Set carrier is a typed-`Arc<HashSetData>`-backed `HeapValue`
//! arm — full HeapValue arm, not pure-discriminator like FilterExpr /
//! SharedCell. Set is a HashMap sibling: same `Arc<TypedBuffer<Arc<
//! String>>>` keys + eager FNV-1a bucket index, with the values buffer
//! dropped.
//!
//! All 12 handlers (`add`, `has`, `delete`, `size`, `is_empty`,
//! `to_array`, `union`, `intersection`, `difference`, `for_each`,
//! `map`, `filter`) are real bodies on top of the post-§2.7.15
//! `HashSetData` shape (`shape_value::heap_value::HashSetData`).
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::HashSet)`, then `args[0].slot.as_heap_value()`
//! pattern-matched against `HeapValue::HashSet(arc)` (single-discriminator
//! per ADR-005 §1 — no per-heap-variant `KindedSlot` accessor).
//!
//! Per-key kind classification follows the same shape: `args[1].kind`
//! against `NativeKind::String | NativeKind::Ptr(HeapKind::String)`,
//! then `as_str()` for the borrow.
//!
//! Result construction follows playbook §3:
//! - `size` / `is_empty` / `has` → inline-scalar `KindedSlot::from_int`
//!   / `from_bool`.
//! - `add` / `delete` → return the post-mutation `Arc<HashSetData>` via
//!   `KindedSlot::from_hashset` (clone-on-write per ADR-006 §2.7.4 /
//!   W13-hashmap-mutation precedent).
//! - `union` / `intersection` / `difference` → build a fresh
//!   `HashSetData` via `from_keys` (no Arc::make_mut on either input —
//!   both receivers borrowed read-only).
//! - `to_array` → reuse the receiver's keys buffer as
//!   `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)` — single
//!   Arc bump, no per-element clone.
//!
//! ## Wave-9 closure-callback migration
//!
//! `for_each`, `map`, `filter` route the per-element callback through
//! `vm.call_value_immediate_nb(&closure, &[key], ctx.as_deref_mut())`
//! (the W7-cv-static §2.7.11 / Q12 dispatch shell at
//! `executor/call_convention.rs:767`). The receiver `Arc<HashSetData>`
//! is cloned once up-front so the iteration borrow is independent of
//! the `&mut VirtualMachine` reborrow on each call.
//!
//! - `for_each` drops the closure's return value (its share is released
//!   as the `KindedSlot` carrier goes out of scope).
//! - `map` requires the closure result kind to be `String` (the
//!   §2.7.15 invariant constrains keys to `Arc<String>`); non-string
//!   results surface as a `RuntimeError`.
//! - `filter` reads the predicate's `as_bool()`; non-bool results
//!   surface as a `RuntimeError`.
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.11 / §2.7.15 + playbook
//! §1.W13-hashset-rebuild.

// V3-S5 ckpt-5-prime²a (2026-05-15): `TypedArrayData` + `TypedBuffer` imports
// DELETED — `TypedBuffer<T>` retired at ckpt-4 (wrapper layer wholesale
// deletion); `TypedArrayData` enum retired across the ckpt-2/3/5 consumer-
// cascade. Migration shape (a) per supervisor 2026-05-15 ratification:
// `HashSetData.keys` now stores `Arc<Vec<Arc<String>>>` directly (smallest
// delta preserving `Arc::make_mut` clone-on-write at the keys-field layer).
// The `v2_to_array` handler is SURFACE-AND-STOP pending the cluster-2 v2-raw
// `*mut TypedArray<Arc<String>>` rebuild that owns the `Array<string>`
// result-construction path (mirrors `array_basic.rs::ckpt5_surface` shape).
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HashSetData, HeapKind, HeapValue};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to an `Arc<HashSetData>` clone via
/// the `iterator_methods::clone_typed_array_arc` sound-pattern
/// (`Arc::from_raw` + `Arc::clone` + `Arc::into_raw`): kind gate on
/// `Ptr(HeapKind::HashSet)`, reconstruct the typed Arc directly from
/// slot bits, clone the share, restore the receiver's slot.
///
/// The W13 version went through `slot.as_heap_value()` matched against
/// `HeapValue::HashSet(arc)` — but `KindedSlot::from_hashset` stores
/// `Arc::into_raw(Arc<HashSetData>) as u64` directly per §2.7.15, so
/// casting those bits to `*const HeapValue` is wrong-type recovery (the
/// underlying allocation is `HashSetData`, not a `HeapValue` enum) and
/// segfaults at the first field read. The sound recovery uses
/// `Arc::from_raw::<HashSetData>` to reconstruct the typed Arc, matching
/// the construction-side contract verbatim.
#[inline]
fn as_hashset(slot: &KindedSlot) -> Result<Arc<HashSetData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::HashSet)) {
        return Err(type_error(format!(
            "Set method receiver must be a Set (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Set method receiver slot bits null"));
    }
    // SAFETY: per the construction-side contract on
    // `KindedSlot::from_hashset`, `Ptr(HeapKind::HashSet)` slot bits are
    // `Arc::into_raw(Arc<HashSetData>)` and the slot owns one
    // strong-count share. Reconstruct, clone (bumping the share), then
    // restore the slot's original share via `Arc::into_raw`.
    let arc = unsafe { Arc::<HashSetData>::from_raw(bits as *const HashSetData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Borrow a `&str` key from a `KindedSlot` whose kind is `String` /
/// `Ptr(HeapKind::String)`. Mirror of `as_string_key` in
/// `hashmap_methods.rs`. Returns a `RuntimeError` for non-string kinds.
#[inline]
fn as_string_key(slot: &KindedSlot) -> Result<&str, VMError> {
    match slot.kind {
        NativeKind::String => slot
            .as_str()
            .ok_or_else(|| type_error("Set key kind=String but slot bits null")),
        NativeKind::Ptr(HeapKind::String) => match slot.slot.as_heap_value() {
            HeapValue::String(s) => Ok(s.as_str()),
            _ => Err(type_error(
                "Set key kind=Ptr(String) but heap arm mismatched",
            )),
        },
        _ => Err(type_error(format!(
            "Set key must be a string (got kind {:?})",
            slot.kind
        ))),
    }
}

/// Project a callback-return `KindedSlot` to an `Arc<String>` for use
/// as a `HashSetData` key. Mirror of `result_slot_to_string_arc` in
/// `hashmap_methods.rs`. Returns `None` for non-string kinds — caller
/// surfaces the kind mismatch as a `RuntimeError`.
fn result_slot_to_string_arc(result: &KindedSlot) -> Option<Arc<String>> {
    match result.kind {
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            let bits = result.slot.raw();
            if bits == 0 {
                return None;
            }
            // SAFETY: per the construction-side contract for both
            // string kinds, the slot bits are `Arc::into_raw::<String>`
            // and the result `KindedSlot` carrier owns one strong-count
            // share. Bump the count once via `increment_strong_count`
            // and reconstruct an owned `Arc<String>` via `Arc::from_raw`;
            // the carrier's drop releases the original share.
            unsafe {
                Arc::increment_strong_count(bits as *const String);
                Some(Arc::from_raw(bits as *const String))
            }
        }
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Read-only handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Set.has(key) -> bool
pub fn v2_has(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.has() requires exactly 1 argument (key)",
        ));
    }
    let set = as_hashset(&args[0])?;
    let key = as_string_key(&args[1])?;
    Ok(KindedSlot::from_bool(set.contains(key)))
}

/// Set.size() -> int
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Set.size() takes no arguments"));
    }
    let set = as_hashset(&args[0])?;
    Ok(KindedSlot::from_int(set.len() as i64))
}

/// Set.isEmpty() -> bool
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Set.isEmpty() takes no arguments"));
    }
    let set = as_hashset(&args[0])?;
    Ok(KindedSlot::from_bool(set.is_empty()))
}

/// Set.toArray() -> Array<string>
///
/// V3-S5 ckpt-5-prime²a SURFACE-AND-STOP (2026-05-15). Pre-deletion shape
/// reused the receiver's keys buffer as `TypedArrayData::String(Arc<
/// TypedBuffer<Arc<String>>>)` for a single-Arc-bump, zero-element-copy
/// result. Post-deletion: `TypedArrayData` enum + `TypedBuffer<T>` /
/// `AlignedTypedBuffer` wrapper layer + `HeapValue::TypedArray(Arc<
/// TypedArrayData>)` outer arm + `HeapKind::TypedArray=8` ordinal DELETED
/// at V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion audit §3.5 +
/// §3.6 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. Rebuild target =
/// per-T v2-raw `*mut TypedArray<Arc<String>>` flat-struct construction
/// per audit §A.3 + §3.1 scalar recipe (lands cluster-2 / ckpt-6).
/// REFUSED ON SIGHT: `TypedArrayData::String` / `TypedBuffer<Arc<String>>`
/// resurrection under any rename (Refusal #1).
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Set.toArray() takes no arguments"));
    }
    let _set = as_hashset(&args[0])?;
    Err(VMError::NotImplemented(
        "Set.toArray: SURFACE — V3-S5 ckpt-5-prime²a consumer-cascade. \
         `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)` + \
         `KindedSlot::from_typed_array` DELETED at V3-S5 ckpt-1..ckpt-4. \
         Rebuild = per-T v2-raw `*mut TypedArray<Arc<String>>` flat-struct \
         construction (cluster-2 / ckpt-6 territory). REFUSED ON SIGHT: \
         resurrection under any rename (Refusal #1)."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Set.add(key) -> Set
///
/// Routes through `HashSetData::insert(Arc<String>) -> bool`. The
/// receiver `Arc<HashSetData>` is cloned up-front so `Arc::make_mut`
/// clones the underlying data only when other shares exist
/// (clone-on-write per ADR-006 §2.7.4 / W13-hashmap-mutation
/// precedent). Returns the (possibly newly-cloned) `Arc<HashSetData>`
/// as the result so chained `s.add(...).add(...)` continues to flow
/// through the post-mutation share.
pub fn v2_add(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.add() requires exactly 1 argument (key)",
        ));
    }
    let key_arc: Arc<String> = result_slot_to_string_arc(&args[1]).ok_or_else(|| {
        type_error(format!(
            "Set.add(): key must be a string (got kind {:?})",
            args[1].kind()
        ))
    })?;
    let mut hs: Arc<HashSetData> = as_hashset(&args[0])?;
    Arc::make_mut(&mut hs).insert(key_arc);
    Ok(KindedSlot::from_hashset(hs))
}

/// Set.delete(key) -> Set
///
/// Routes through `HashSetData::remove(&str) -> bool`. Returns the
/// (possibly newly-cloned) `Arc<HashSetData>` post-removal — missing-
/// key removals are a no-op at the `HashSetData` layer (the `bool`
/// return is ignored at this surface; the result still carries the
/// receiver share for chaining).
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.delete() requires exactly 1 argument (key)",
        ));
    }
    let key = as_string_key(&args[1])?;
    let mut hs: Arc<HashSetData> = as_hashset(&args[0])?;
    Arc::make_mut(&mut hs).remove(key);
    Ok(KindedSlot::from_hashset(hs))
}

// ═══════════════════════════════════════════════════════════════════════════
// Set-operation handlers (build a fresh HashSetData)
// ═══════════════════════════════════════════════════════════════════════════

/// Set.union(other) -> Set
///
/// Returns a new `Set` containing every element in either receiver.
/// Both inputs are borrowed read-only (no `Arc::make_mut` on either);
/// the result is a fresh `HashSetData` built via `from_keys`.
pub fn v2_union(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.union() requires exactly 1 argument (other)",
        ));
    }
    let lhs: Arc<HashSetData> = as_hashset(&args[0])?;
    let rhs: Arc<HashSetData> = as_hashset(&args[1])?;
    let mut keys: Vec<Arc<String>> = Vec::with_capacity(lhs.len() + rhs.len());
    for k in lhs.keys.iter() {
        keys.push(Arc::clone(k));
    }
    for k in rhs.keys.iter() {
        keys.push(Arc::clone(k));
    }
    // `from_keys` collapses duplicates — first occurrence wins.
    let result = HashSetData::from_keys(keys);
    Ok(KindedSlot::from_hashset(Arc::new(result)))
}

/// Set.intersection(other) -> Set
///
/// Returns a new `Set` containing only elements present in both
/// receivers. Iteration walks the smaller receiver and probes the
/// larger for membership via the bucket index.
pub fn v2_intersection(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.intersection() requires exactly 1 argument (other)",
        ));
    }
    let lhs: Arc<HashSetData> = as_hashset(&args[0])?;
    let rhs: Arc<HashSetData> = as_hashset(&args[1])?;
    // Walk the smaller side; probe the larger for membership.
    let (small, large) = if lhs.len() <= rhs.len() {
        (lhs.as_ref(), rhs.as_ref())
    } else {
        (rhs.as_ref(), lhs.as_ref())
    };
    let mut keys: Vec<Arc<String>> = Vec::new();
    for k in small.keys.iter() {
        if large.contains(k.as_str()) {
            keys.push(Arc::clone(k));
        }
    }
    let result = HashSetData::from_keys(keys);
    Ok(KindedSlot::from_hashset(Arc::new(result)))
}

/// Set.difference(other) -> Set
///
/// Returns a new `Set` containing every element in the receiver that
/// is NOT present in `other` (left-biased asymmetric difference,
/// matching JS / Python `Set.difference`).
pub fn v2_difference(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.difference() requires exactly 1 argument (other)",
        ));
    }
    let lhs: Arc<HashSetData> = as_hashset(&args[0])?;
    let rhs: Arc<HashSetData> = as_hashset(&args[1])?;
    let mut keys: Vec<Arc<String>> = Vec::new();
    for k in lhs.keys.iter() {
        if !rhs.contains(k.as_str()) {
            keys.push(Arc::clone(k));
        }
    }
    let result = HashSetData::from_keys(keys);
    Ok(KindedSlot::from_hashset(Arc::new(result)))
}

// ═══════════════════════════════════════════════════════════════════════════
// Closure-based handlers — Wave-9 W9-set-methods migration
// ═══════════════════════════════════════════════════════════════════════════

/// Set.forEach(fn(key)) -> unit
///
/// Iterates entries in insertion order, invoking the closure with the
/// per-element key. The callback's return is dropped.
pub fn v2_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.forEach() requires exactly 1 argument (callback)",
        ));
    }
    let set: Arc<HashSetData> = as_hashset(&args[0])?;
    let closure = &args[1];
    let n = set.len();
    for i in 0..n {
        let key_slot = KindedSlot::from_string_arc(Arc::clone(&set.keys[i]));
        let _ = vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}

/// Set.map(fn(key) -> new_key) -> Set
///
/// Builds a new `Set` whose elements are the closure's per-element
/// returns. The §2.7.15 invariant constrains keys to `Arc<String>`;
/// non-string closure results surface as a `RuntimeError`. Duplicate
/// outputs are collapsed by `HashSetData::from_keys` (first occurrence
/// wins).
pub fn v2_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.map() requires exactly 1 argument (mapper)",
        ));
    }
    let set: Arc<HashSetData> = as_hashset(&args[0])?;
    let closure = &args[1];
    let n = set.len();
    let mut out_keys: Vec<Arc<String>> = Vec::with_capacity(n);
    for i in 0..n {
        let key_slot = KindedSlot::from_string_arc(Arc::clone(&set.keys[i]));
        let result = vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
        let new_key: Arc<String> = result_slot_to_string_arc(&result).ok_or_else(|| {
            type_error(format!(
                "Set.map(): mapper must return a string (got kind {:?})",
                result.kind()
            ))
        })?;
        out_keys.push(new_key);
    }
    let result = HashSetData::from_keys(out_keys);
    Ok(KindedSlot::from_hashset(Arc::new(result)))
}

/// Set.filter(fn(key) -> bool) -> Set
///
/// Keeps elements for which the closure returns `true`. Non-bool
/// closure results surface as a `RuntimeError` per playbook §6 — no
/// Bool-default fallback.
pub fn v2_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Set.filter() requires exactly 1 argument (predicate)",
        ));
    }
    let set: Arc<HashSetData> = as_hashset(&args[0])?;
    let closure = &args[1];
    let n = set.len();
    let mut out_keys: Vec<Arc<String>> = Vec::new();
    for i in 0..n {
        let key_arc = Arc::clone(&set.keys[i]);
        let key_slot = KindedSlot::from_string_arc(Arc::clone(&key_arc));
        let result = vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
        let keep = result.as_bool().ok_or_else(|| {
            type_error(format!(
                "Set.filter(): predicate must return bool (got kind {:?})",
                result.kind()
            ))
        })?;
        if keep {
            out_keys.push(key_arc);
        }
    }
    // Filter preserves order with no duplicates (the receiver is
    // already deduplicated and we don't introduce new keys); construct
    // directly without going through `from_keys`'s dedup pass.
    //
    // V3-S5 ckpt-5-prime²a (2026-05-15): `TypedBuffer::from_vec` retired
    // (wrapper layer wholesale deletion at ckpt-4); Migration shape (a)
    // wraps the `Vec<Arc<String>>` directly in the `Arc<Vec<Arc<String>>>`
    // post-§2.7.15 keys-field layout.
    let mut index: std::collections::HashMap<u64, Vec<u32>> = std::collections::HashMap::new();
    for (i, k) in out_keys.iter().enumerate() {
        let h = fnv1a_hash_local(k.as_bytes());
        index.entry(h).or_default().push(i as u32);
    }
    let result = HashSetData {
        keys: Arc::new(out_keys),
        index,
    };
    Ok(KindedSlot::from_hashset(Arc::new(result)))
}

/// Local FNV-1a hash matching `shape_value::heap_value::fnv1a_hash`'s
/// shape (the `heap_value`-private function isn't `pub`; replicating
/// the algorithm here keeps the bucket-index hash semantics aligned
/// with `HashSetData::contains` / `insert`).
#[inline]
fn fnv1a_hash_local(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
