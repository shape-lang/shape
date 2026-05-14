//! HashMap method handlers for the PHF method registry.
//!
//! ## W9-hashmap-methods migration (2026-05-10)
//!
//! Per ADR-006 §2.7.10 / Q11 (Wave-γ G-method-fn-v2-abi close commit
//! `5091cba`) the `MethodFnV2` ABI is kinded: handlers take
//! `args: &[KindedSlot]` and return `Result<KindedSlot, VMError>`. The
//! preceding Wave-δ pass migrated the 9 read-only handlers (`get`, `has`,
//! `keys`, `values`, `entries`, `len`, `isEmpty`, `getOrDefault`,
//! `toArray`) to real bodies on top of the post-§2.7.4 `HashMapData` shape
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
//! - `values` → `the-deleted-heterogeneous-element-carrier(Arc<TypedBuffer<Arc<HeapValue>>>)`.
//! - `entries` / `toArray` → outer `the-deleted-heterogeneous-element-carrier` with each
//!   element a 2-element inner array (`[key, value]`) wrapped as
//!   `Arc::new(HeapValue::TypedArray(Arc::new(the-deleted-heterogeneous-element-carrier(
//!   ..))))` — same shape as `array_ops::builtin_zip` at line 376.
//!
//! ## Wave-9 closure-callback migration
//!
//! `forEach`, `map`, `filter`, `reduce`, and `groupBy` route the per-entry
//! callback through `vm.call_value_immediate_nb(&closure, &[key, value, …],
//! ctx.as_deref_mut())` (the W7-cv-static §2.7.11 / Q12 dispatch shell at
//! `executor/call_convention.rs:767`). The receiver `Arc<HashMapData>` is
//! cloned once up-front so the iteration borrow is independent of the
//! `&mut VirtualMachine` reborrow on each call. Per-entry key carriers are
//! `KindedSlot::from_string_arc(Arc::clone(&map.keys.data[i]))`; value
//! carriers go through `heap_value_arc_to_slot` (per-FieldType constructors
//! per ADR-005 §3 / ADR-006 single-discriminator). The closure result
//! `KindedSlot` is consumed by the body — `forEach` drops it, `map` /
//! `reduce` thread its `Arc<HeapValue>`-projected payload as the new
//! value / accumulator, `filter` reads its `as_bool()`, and `groupBy`
//! requires its kind to be `String` (the new `HashMapData` invariant
//! constrains keys to `Arc<String>`; non-string group keys surface as a
//! `RuntimeError`).
//!
//! ## Still-surfaced handlers (3 mutation + 1 iter)
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
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.11 / Q8 / Q11 / Q12 + playbook
//! §1 / §3.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HashMapKindedRef, HeapKind, HeapValue, TypedArrayData};
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
fn as_hashmap(slot: &KindedSlot) -> Result<Arc<HashMapKindedRef>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::HashMap)) {
        return Err(type_error(format!(
            "HashMap method receiver must be a HashMap (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("HashMap method receiver slot bits null"));
    }
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): slot bits are
    // `Arc::into_raw(Arc<HashMapKindedRef>) as u64` per ADR-006 §2.7.24
    // Q25.B SUPERSEDED. Recovery: bump strong count, clone the outer
    // Arc<HashMapKindedRef> share, drop the bumped share (the slot's
    // original share remains intact — 5-arm receiver-recovery shape).
    // SAFETY: construction-side contract on KindedSlot::from_hashmap.
    let arc = unsafe {
        Arc::<HashMapKindedRef>::from_raw(bits as *const HashMapKindedRef)
    };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
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
/// `the-deleted-heterogeneous-element-carrier` element re-wrapping path).
fn heap_value_arc_to_slot(hv: &Arc<HeapValue>) -> KindedSlot {
    match hv.as_ref() {
        HeapValue::String(s) => KindedSlot::from_string_arc(Arc::clone(s)),
        HeapValue::Decimal(d) => KindedSlot::from_decimal(Arc::clone(d)),
        HeapValue::BigInt(b) => KindedSlot::from_bigint(Arc::clone(b)),
        HeapValue::TypedArray(a) => KindedSlot::from_typed_array(Arc::clone(a)),
        // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): TypedObjectPtr.
        HeapValue::TypedObject(o) => KindedSlot::from_typed_object_raw(o.clone().into_raw()),
        // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): payload flipped
        // to `HashMapKindedRef`. The Arc-wrapped clone is built fresh
        // (Arc::new on a cloned kinded ref — Clone is per-V Arc bump per
        // HashMapKindedRef::clone). Mirrors the post-flip
        // `ValueSlot::from_hashmap(Arc<HashMapKindedRef>)` signature.
        HeapValue::HashMap(m) => KindedSlot::from_hashmap(Arc::new(m.clone())),
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
    let _map = as_hashmap(&args[0])?;
    let _key = as_string_key(&args[1])?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.get(key)
    // → KindedSlot dispatch is ckpt-3 territory (consumer cascade alongside
    // hashmap_methods.rs body rewrites). At ckpt-2 the receiver-recovery
    // helper `as_hashmap` returns `Arc<HashMapKindedRef>` (per-V enum
    // carrier); the per-V `match kref { I64(arc) => arc.get(...) ... }`
    // body is restored at ckpt-3. SURFACE-AND-STOP at ckpt-2.
    Err(VMError::RuntimeError(
        "HashMap.get(): per-V dispatch is ckpt-3 territory (per-V \
         HashMapKindedRef body rewrite not landed). ADR-006 §2.7.24 \
         Q25.B SUPERSEDED + audit §C.4 — Round 3b C2-joint cascade \
         pending."
            .to_string(),
    ))
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
    let _map = as_hashmap(&args[0])?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.keys()
    // → `TypedArrayData::String` projection is ckpt-3 territory (the new
    // keys buffer shape is `*mut TypedArray<*const StringObj>`; the
    // projection into `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)`
    // requires either a new TypedArrayData::StringV2 variant or a buffer
    // copy step). SURFACE-AND-STOP at ckpt-2. ADR-006 §2.7.24 Q25.B
    // SUPERSEDED + audit §C.4.
    Err(VMError::RuntimeError(
        "HashMap.keys(): per-V dispatch is ckpt-3 territory (keys-buffer \
         shape flipped to *mut TypedArray<*const StringObj>; projection \
         to TypedArrayData::String requires ckpt-3 cascade). Round 3b \
         C2-joint pending."
            .to_string(),
    ))
}

/// HashMap.values() -> Array<value>
///
/// Returns an `Array<heap>` reusing the receiver's values buffer
/// (`Arc<TypedBuffer<Arc<HeapValue>>>`) — single Arc bump on the buffer.
/// Wraps as `the-deleted-heterogeneous-element-carrier(Arc<TypedBuffer<Arc<HeapValue>>>)`.
pub fn v2_values(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("HashMap.values() takes no arguments"));
    }
    let _map = as_hashmap(&args[0])?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): the values projection
    // now lives at the `HashMapKindedRef` enum level — each variant's
    // inner `Arc<HashMapData<V>>` projects to its matching
    // `TypedArrayData::<V>` arm by reading `arc.values: *mut TypedArray<V>`
    // and (a) wrapping in `TypedBuffer::<V>::wrap_raw` (requires the
    // per-V wrap helper, not yet built) OR (b) deep-copying into a fresh
    // `TypedBuffer<V>`. Per-V cascade is ckpt-3 territory. SURFACE-AND-
    // STOP at ckpt-2. ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4.
    Err(VMError::RuntimeError(
        "HashMap.values(): per-V dispatch is ckpt-3 territory (values-buffer \
         shape flipped to *mut TypedArray<V>; per-V TypedArrayData::<V> \
         projection requires ckpt-3 cascade). Round 3b C2-joint pending."
            .to_string(),
    ))
}

/// HashMap.entries() -> Array<[key, value]>
///
/// Each entry is a 2-element inner array `[key, value]` stored as
/// `HeapValue::TypedArray(Arc<the-deleted-heterogeneous-element-carrier>)`. The outer
/// array is a `the-deleted-heterogeneous-element-carrier` of those `Arc<HeapValue>` entries.
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
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.get(key)
    // → KindedSlot dispatch is ckpt-3 territory. For the simple
    // `contains_key`-checked default-or-surface pattern: on miss return
    // the default (which works at ckpt-2 since default is `KindedSlot`
    // already); on hit SURFACE since per-V projection of the value
    // pointer/scalar into a `KindedSlot` requires ckpt-3.
    if map.contains_key(key) {
        Err(VMError::RuntimeError(
            "HashMap.getOrDefault(): per-V hit-path dispatch is ckpt-3 \
             territory. ADR-006 §2.7.24 Q25.B SUPERSEDED."
                .to_string(),
        ))
    } else {
        Ok(args[2].clone())
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

/// Shared body for `entries()` / `toArray()`. Builds per-entry
/// `Entry<K,V>` TypedObjects + the outer `TypedArrayData::TypedObject` wrapper.
///
/// W17-typed-carrier-bundle-A checkpoint 2/4: per the C+ resolution
/// recorded in `phase-2d-playbook.md` §3 (Bundle-A checkpoint-2 amendment),
/// each entry is constructed as a TypedObject with fields `{key, value}`.
/// User code reads `entry.key` / `entry.value` rather than `entry[0]` /
/// `entry[1]` — breaking change for stdlib + tests.
fn build_entries_array(receiver: &KindedSlot) -> Result<KindedSlot, VMError> {
    let _map = as_hashmap(receiver)?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap entries
    // walk is ckpt-3 territory. The new HashMapData<V> shape requires
    // (a) walking `*mut TypedArray<*const StringObj>` keys → `Arc<String>`
    // per element (via StringObj-to-Arc<String> projection), and
    // (b) walking `*mut TypedArray<V>` values per-V → `KindedSlot::from_V`
    // dispatch. Both walks are ckpt-3 cascade. SURFACE-AND-STOP at ckpt-2.
    // ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4.
    let _ = TypedBuffer::<i64>::from_vec; // sanity binding
    Err(VMError::RuntimeError(
        "HashMap.entries() / toArray(): per-V dispatch is ckpt-3 territory \
         (per-V HashMapKindedRef walk into entry TypedObjects not landed). \
         ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4 — Round 3b C2-joint \
         cascade pending."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation handlers — phase-2c HashMapData typed-buffer mutation API rebuild
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.set(key, value) -> HashMap
///
/// W13-hashmap-mutation (2026-05-10) close: routes through
/// `HashMapData::insert(Arc<String>, Arc<HeapValue>)`. The receiver
/// `Arc<HashMapData>` is cloned up-front so `Arc::make_mut` clones the
/// underlying data only when other shares exist (clone-on-write per
/// ADR-006 §2.7.4 / playbook). Returns the (possibly newly-cloned)
/// `Arc<HashMapData>` as the result so chained `m.set(...).set(...)`
/// continues to flow through the post-mutation share.
pub fn v2_set(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 3 {
        return Err(type_error(
            "HashMap.set() requires exactly 2 arguments (key, value)",
        ));
    }
    let _ = &args[0]; // suppress unused
    let _ = &args[1];
    let _ = &args[2];
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.set is
    // ckpt-3 territory. The new shape requires (a) classifying the value
    // arg's kind to pick the right HashMapKindedRef::V arm, (b) per-V
    // Arc::make_mut on the inner Arc<HashMapData<V>>, (c) per-V
    // `*mut TypedArray<V>::push` mutation. ADR-006 §2.7.24 Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.set(): per-V dispatch is ckpt-3 territory. ADR-006 \
         §2.7.24 Q25.B SUPERSEDED."
            .to_string(),
    ))
}

/// HashMap.delete(key) -> HashMap
///
/// W13-hashmap-mutation close: routes through `HashMapData::remove(&str)`.
/// Returns the (possibly newly-cloned) `Arc<HashMapData>` post-removal —
/// missing-key removals are a no-op at the `HashMapData` layer (the
/// `bool` return is ignored at this surface; the result still carries the
/// receiver share for chaining).
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.delete() requires exactly 1 argument (key)",
        ));
    }
    let _ = as_string_key(&args[1])?;
    let _ = as_hashmap(&args[0])?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.delete
    // is ckpt-3 territory (per-V Arc::make_mut + per-V remove). ADR-006
    // §2.7.24 Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.delete(): per-V dispatch is ckpt-3 territory."
            .to_string(),
    ))
}

/// HashMap.remove(key) -> Option<value>
///
/// Tuple-return ABI variant (ADR-006 §2.7.27 amendment, W17-pop-mutation,
/// 2026-05-12). Conceptual dispatch signature is
/// `(&mut self) -> (Option<value>, Self)`; reads the value-at-key
/// (returning `Option<value>` for missing keys), mutates the map via
/// `Arc::make_mut + remove`, side-channel-publishes the new
/// `Arc<HashMapData>` to the VM stack for compiler-emitted write-back,
/// then returns the popped value.
///
/// Distinct from the existing `delete(key)` self-returning method:
/// `delete` returns the (new) map for chaining and is consumed by
/// `stdlib-src/core/set.shape::remove` which wraps `s.delete(item)` to
/// preserve set-style semantics. `remove` follows the canonical
/// pop-shape ABI; both methods can coexist on HashMap with distinct
/// return contracts (decision-call #3 per W17-pop-mutation dispatch).
pub fn v2_remove(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.remove() requires exactly 1 argument (key)",
        ));
    }
    let _ = as_string_key(&args[1])?;
    let _ = as_hashmap(&args[0])?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.remove
    // is ckpt-3 territory. The tuple-return ABI + side-channel publish
    // both depend on per-V Arc::make_mut + per-V value-at-key projection.
    Err(VMError::RuntimeError(
        "HashMap.remove(): per-V dispatch is ckpt-3 territory."
            .to_string(),
    ))
}

/// HashMap.merge(other) -> HashMap
///
/// W13-hashmap-mutation close: routes through `HashMapData::merge(&other)`.
/// Last-write-wins on key collision (matches `Object.assign` /
/// `dict.update` semantics). The `other` receiver is borrowed via
/// `as_hashmap` and never has its share decremented at this surface.
pub fn v2_merge(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.merge() requires exactly 1 argument (other)",
        ));
    }
    let _ = as_hashmap(&args[1])?;
    let _ = as_hashmap(&args[0])?;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.merge is
    // ckpt-3 territory (per-V Arc::make_mut + per-V buffer extend).
    Err(VMError::RuntimeError(
        "HashMap.merge(): per-V dispatch is ckpt-3 territory."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Closure-based handlers — Wave-9 W9-hashmap-methods migration
// ═══════════════════════════════════════════════════════════════════════════
//
// Each handler clones the receiver `Arc<HashMapData>` up-front so the
// iteration borrow is independent of the `&mut VirtualMachine` reborrow on
// each `call_value_immediate_nb` call. Per-entry key carriers are
// `KindedSlot::from_string_arc(Arc::clone(&map.keys.data[i]))`; value
// carriers go through `heap_value_arc_to_slot`. The closure carrier
// (`args[1]`) is borrowed positionally — `call_value_immediate_nb`
// dispatches on `callee.kind` (Closure / UInt64), no kind fabrication.
//
// Result construction for `map` / `filter` / `groupBy` uses
// `HashMapData::from_pairs(Vec<Arc<String>>, Vec<Arc<HeapValue>>)` —
// the construction-only path that the §2.7.4 buffer rebuild deliberately
// preserved (the mutation gap is at `set` / `delete` / `merge`, where an
// existing buffer must be appended to / removed from in place; building a
// fresh `HashMapData` is free of that constraint).

/// HashMap.forEach(fn(key, value)) -> unit
///
/// Iterates entries in insertion order, invoking the closure with
/// `(key, value)` per entry. The callback's return is dropped (its share
/// is released as the `KindedSlot` carrier goes out of scope).
pub fn v2_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.forEach() requires exactly 1 argument (callback)",
        ));
    }
    let _ = as_hashmap(&args[0])?;
    let _closure = &args[1];
    let _ = vm;
    let _ = ctx;
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.forEach
    // iteration is ckpt-3 territory (per-V keys/values walk + per-V
    // kinded-slot construction for each closure-call). ADR-006 §2.7.24
    // Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.forEach(): per-V iteration is ckpt-3 territory."
            .to_string(),
    ))
}

/// HashMap.filter(fn(key, value) -> bool) -> HashMap
///
/// Keeps entries for which the closure returns `true`. The result kind on
/// the predicate is required to be `Bool`; other kinds surface as a
/// `RuntimeError` (per playbook §6 — no Bool-default fallback).
pub fn v2_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.filter() requires exactly 1 argument (predicate)",
        ));
    }
    let _ = as_hashmap(&args[0])?;
    let _ = (vm, ctx, &args[1]);
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.filter
    // is ckpt-3 territory (per-V iteration + per-V new HashMap construction
    // for surviving entries). ADR-006 §2.7.24 Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.filter(): per-V iteration + construction is ckpt-3 \
         territory."
            .to_string(),
    ))
}

/// HashMap.map(fn(key, value) -> new_value) -> HashMap
///
/// Builds a new `HashMap` with the same keys, replacing each value with
/// the closure's return. Result-side kinds re-pack into
/// `Arc<HeapValue>` via `result_slot_to_heap_value_arc`.
pub fn v2_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.map() requires exactly 1 argument (mapper)",
        ));
    }
    let _ = as_hashmap(&args[0])?;
    let _ = (vm, ctx, &args[1]);
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.map is
    // ckpt-3 territory (per-V iteration + per-V new HashMap construction
    // with closure-result re-pack). ADR-006 §2.7.24 Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.map(): per-V iteration + construction is ckpt-3 territory."
            .to_string(),
    ))
}

/// HashMap.reduce(fn(acc, key, value) -> acc, initial) -> value
///
/// Threads the accumulator through the per-entry callback. Initial value
/// is `args[2]`; final accumulator is the result.
pub fn v2_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 3 {
        return Err(type_error(
            "HashMap.reduce() requires exactly 2 arguments (reducer, initial)",
        ));
    }
    let _ = as_hashmap(&args[0])?;
    let _ = (vm, ctx, &args[1], &args[2]);
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.reduce
    // iteration is ckpt-3 territory. ADR-006 §2.7.24 Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.reduce(): per-V iteration is ckpt-3 territory."
            .to_string(),
    ))
}

/// HashMap.groupBy(fn(key, value) -> group_key) -> HashMap<group_key, HashMap>
///
/// Buckets entries by the closure's returned `group_key`. The new
/// `HashMapData` invariant constrains keys to `Arc<String>`; a non-string
/// group_key kind surfaces as a `RuntimeError` (no fallback coercion per
/// playbook §6 — kind soundness is preserved at the construction site).
pub fn v2_group_by(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.groupBy() requires exactly 1 argument (key-extractor)",
        ));
    }
    let _ = as_hashmap(&args[0])?;
    let _ = (vm, ctx, &args[1]);
    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): per-V HashMap.groupBy
    // is ckpt-3 territory. The inner-/outer-HashMap construction depends
    // on per-V `HashMapData<V>::from_pairs` API and HashMap-of-HashMap
    // wiring through HashMapKindedRef. ADR-006 §2.7.24 Q25.B SUPERSEDED.
    Err(VMError::RuntimeError(
        "HashMap.groupBy(): per-V iteration + outer/inner HashMap \
         construction is ckpt-3 territory."
            .to_string(),
    ))
}

/// Project a callback-return `KindedSlot` to an `Arc<HeapValue>` suitable
/// for `HashMapData::values` storage. Mirrors
/// `executor/builtins/array_ops::slot_to_heap_arc` — Int64 round-trips
/// through `BigInt(Arc<i64>)`; Float64 / Bool reject (no matching heap
/// arm in the post-§2.3 `HeapValue`); String / heap-pointer kinds clone
/// through `as_heap_value()` per ADR-005 §1 single-discriminator.
fn result_slot_to_heap_value_arc(result: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    match result.kind {
        NativeKind::Int64 => {
            let i = result.as_i64().ok_or_else(|| {
                type_error("HashMap.map(): Int64 slot bits not a valid integer")
            })?;
            Ok(Arc::new(HeapValue::BigInt(Arc::new(i))))
        }
        NativeKind::Float64 => Err(type_error(
            "HashMap.map(): Float64 result cannot be heap-wrapped (no HeapValue::Number arm)",
        )),
        NativeKind::Bool => Err(type_error(
            "HashMap.map(): Bool result cannot be heap-wrapped (no HeapValue::Bool arm)",
        )),
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            // Both string kinds store `Arc::into_raw::<String>` (no
            // `Box<HeapValue>` wrapper); recover via raw-pointer Arc
            // resurrection then re-wrap in the canonical
            // `HeapValue::String(Arc<String>)` arm. Same shape as
            // `result_slot_to_string_arc` below.
            let bits = result.slot.raw();
            if bits == 0 {
                return Err(type_error("HashMap.map(): String slot bits null"));
            }
            // SAFETY: see `result_slot_to_string_arc` — bits are
            // `Arc::into_raw::<String>`, carrier owns one share.
            let arc = unsafe {
                Arc::increment_strong_count(bits as *const String);
                Arc::from_raw(bits as *const String)
            };
            Ok(Arc::new(HeapValue::String(arc)))
        }
        NativeKind::Ptr(_) => {
            // True heap pointer: bits point at a `Box<HeapValue>`-style
            // payload via `Arc<T>` per the per-FieldType slot constructors.
            // Clone the underlying HeapValue. The result slot already owns
            // one strong-count share; the clone bumps it.
            let hv: &HeapValue = result.slot.as_heap_value();
            Ok(Arc::new(hv.clone()))
        }
        other => Err(type_error(format!(
            "HashMap.map(): result kind {:?} cannot be stored in a HashMap value",
            other
        ))),
    }
}

/// Project a callback-return `KindedSlot` to an `Arc<String>` for use as a
/// `HashMapData` key. Returns `None` for non-string kinds — caller surfaces
/// the kind mismatch as a `RuntimeError`. Both `NativeKind::String` and
/// `NativeKind::Ptr(HeapKind::String)` slot encodings store an
/// `Arc::into_raw::<String>` payload (per the `ValueSlot::from_string_arc`
/// constructor + `KindedSlot::clone`'s string arm at
/// `kinded_slot.rs:474..480`), so the recovery path is the same for both.
fn result_slot_to_string_arc(result: &KindedSlot) -> Option<Arc<String>> {
    match result.kind {
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            let bits = result.slot.raw();
            if bits == 0 {
                return None;
            }
            // SAFETY: per the construction-side contract for both string
            // kinds, the slot bits are `Arc::into_raw::<String>` and the
            // result `KindedSlot` carrier owns one strong-count share.
            // We bump the count once via `increment_strong_count` and
            // reconstruct an owned `Arc<String>` via `Arc::from_raw`; the
            // result Arc is the new share and the carrier's drop will
            // release the original share.
            unsafe {
                Arc::increment_strong_count(bits as *const String);
                Some(Arc::from_raw(bits as *const String))
            }
        }
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Iterator handler — phase-2c IteratorState rebuild
// ═══════════════════════════════════════════════════════════════════════════

/// HashMap.iter() -> Iterator
///
/// W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10): forwards to
/// the iterator cluster's `handle_hashmap_iter` factory. Each yielded
/// element is a 2-element `[key, value]` inner array (mirrors
/// `HashMap.entries()`).
pub fn v2_iter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::iterator_methods::handle_hashmap_iter(vm, args, ctx)
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
