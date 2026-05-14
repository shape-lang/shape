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
///
/// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): per-V dispatch on
/// `HashMapKindedRef::V(arc).get_share(key)`. Returns a KindedSlot
/// constructed via the matching per-V `KindedSlot::from_*` constructor;
/// the slot owns one fresh share (POD V: trivial copy; HeapElement /
/// Ptr-newtype V: v2_retain bump via `HashMapValueElem::share_clone`).
/// On miss returns `KindedSlot::none()`.
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
    Ok(get_kinded(&map, key).unwrap_or_else(KindedSlot::none))
}

/// Per-V get → `KindedSlot` dispatcher. Reads `arc.get_share(key)` for the
/// matching variant; wraps each per-V value type in the matching
/// `KindedSlot::from_*` constructor. ADR-006 §2.7.24 Q25.B SUPERSEDED.
fn get_kinded(map: &HashMapKindedRef, key: &str) -> Option<KindedSlot> {
    match map {
        HashMapKindedRef::I64(arc) => arc.get_share(key).map(KindedSlot::from_int),
        HashMapKindedRef::F64(arc) => arc.get_share(key).map(KindedSlot::from_number),
        HashMapKindedRef::Bool(arc) => arc.get_share(key).map(|v| KindedSlot::from_bool(v != 0)),
        HashMapKindedRef::Char(arc) => arc.get_share(key).map(KindedSlot::from_char),
        HashMapKindedRef::String(arc) => {
            // V = *const StringObj. get_share v2_retains the share; deep-
            // copy to Arc<String> + retire the bumped share for user-
            // visible compatibility with NativeKind::String paths.
            arc.get_share(key).map(|ptr| {
                let s = unsafe {
                    shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
                };
                unsafe {
                    use shape_value::v2::heap_element::HeapElement;
                    shape_value::v2::string_obj::StringObj::release_elem(ptr);
                }
                KindedSlot::from_string_arc(Arc::new(s))
            })
        }
        HashMapKindedRef::Decimal(arc) => {
            // V = *const DecimalObj.
            arc.get_share(key).map(KindedSlot::from_decimal_v2_ptr)
        }
        HashMapKindedRef::TypedObject(arc) => {
            // V = TypedObjectPtr. get_share returns a new wrapper with
            // v2_retain bumped refcount; convert to slot via into_raw.
            arc.get_share(key)
                .map(|ptr| KindedSlot::from_typed_object_raw(ptr.into_raw()))
        }
        HashMapKindedRef::TraitObject(arc) => {
            // V = TraitObjectPtr. Same shape as TypedObject.
            arc.get_share(key)
                .map(|ptr| KindedSlot::from_trait_object_raw(ptr.into_raw()))
        }
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
/// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): per-V dispatch via
/// `get_kinded`; on miss the kinded `default` slot is returned by
/// clone (refcount bump for heap arms via `KindedSlot::clone`).
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
    Ok(get_kinded(&map, key).unwrap_or_else(|| args[2].clone()))
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
/// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): per-V dispatch.
/// Requires the value arg's kind to match the receiver's existing V
/// variant (no V-flip / promotion at this layer — that would require
/// a full HashMap-clone-into-new-V transition not yet supported). The
/// new HashMap with the (possibly newly-cloned) inner data is returned
/// so chained `m.set(...).set(...)` flows through the post-mutation
/// share.
///
/// Mismatched value kind surfaces as a RuntimeError per playbook §6
/// (no fallback coercion; kind soundness preserved).
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
    let map_arc = as_hashmap(&args[0])?;
    let key = as_string_key(&args[1])?.to_owned();
    let value_slot = &args[2];
    // Build the new outer Arc<HashMapKindedRef> via per-V clone-on-write.
    let new_kref = set_kinded(&map_arc, &key, value_slot)?;
    Ok(KindedSlot::from_hashmap(std::sync::Arc::new(new_kref)))
}

/// Per-V set: clone the inner Arc<HashMapData<V>>, project the value
/// arg's slot into a V (Arc::make_mut clone-on-write), insert. Returns
/// the new `HashMapKindedRef` carrier.
fn set_kinded(
    map: &HashMapKindedRef,
    key: &str,
    value_slot: &KindedSlot,
) -> Result<HashMapKindedRef, VMError> {
    use shape_value::heap_value::HashMapData;
    // Empty-HashMap V-promotion: if the receiver has zero entries + the
    // current V doesn't match the incoming value's kind, rebuild as the
    // matching V. Sound because an empty HashMap has no live values to
    // re-cast. Resolves the `let m = HashMap(); m.set("k", 1)` pattern
    // where HashMapCtor defaults to V=*const StringObj but the first
    // insert is V=i64. ADR-006 §2.7.24 Q25.B SUPERSEDED.
    if map.is_empty() && map.values_kind() != value_kind_hint(value_slot) {
        if let Some(promoted) = empty_set_with_promotion(key, value_slot)? {
            return Ok(promoted);
        }
    }
    match map {
        HashMapKindedRef::I64(arc) => {
            let v = match value_slot.kind {
                NativeKind::Int64 => value_slot.as_i64().ok_or_else(|| {
                    type_error("HashMap.set(): Int64 slot bits not a valid integer")
                })?,
                other => {
                    return Err(type_error(format!(
                        "HashMap.set(): value kind {:?} incompatible with HashMap<string, int>",
                        other
                    )))
                }
            };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v) };
            Ok(HashMapKindedRef::I64(new_arc))
        }
        HashMapKindedRef::F64(arc) => {
            let v = match value_slot.kind {
                NativeKind::Float64 => value_slot.as_f64().ok_or_else(|| {
                    type_error("HashMap.set(): Float64 slot bits not a valid f64")
                })?,
                other => {
                    return Err(type_error(format!(
                        "HashMap.set(): value kind {:?} incompatible with HashMap<string, number>",
                        other
                    )))
                }
            };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v) };
            Ok(HashMapKindedRef::F64(new_arc))
        }
        HashMapKindedRef::Bool(arc) => {
            let v: u8 = match value_slot.kind {
                NativeKind::Bool => {
                    let b = value_slot.as_bool().ok_or_else(|| {
                        type_error("HashMap.set(): Bool slot bits not a valid bool")
                    })?;
                    if b {
                        1
                    } else {
                        0
                    }
                }
                other => {
                    return Err(type_error(format!(
                        "HashMap.set(): value kind {:?} incompatible with HashMap<string, bool>",
                        other
                    )))
                }
            };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v) };
            Ok(HashMapKindedRef::Bool(new_arc))
        }
        HashMapKindedRef::Char(arc) => {
            let v: char = match value_slot.kind {
                NativeKind::Char => value_slot.as_char().ok_or_else(|| {
                    type_error("HashMap.set(): Char slot bits not a valid char")
                })?,
                other => {
                    return Err(type_error(format!(
                        "HashMap.set(): value kind {:?} incompatible with HashMap<string, char>",
                        other
                    )))
                }
            };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v) };
            Ok(HashMapKindedRef::Char(new_arc))
        }
        HashMapKindedRef::String(arc) => {
            // V = *const StringObj. Project value slot to a fresh-share
            // *const StringObj via the §2.7.6 / Q8 dispatch.
            let v_ptr = string_slot_to_v2_ptr(value_slot).ok_or_else(|| {
                type_error(format!(
                    "HashMap.set(): value kind {:?} incompatible with HashMap<string, string>",
                    value_slot.kind
                ))
            })?;
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v_ptr) };
            Ok(HashMapKindedRef::String(new_arc))
        }
        HashMapKindedRef::Decimal(_)
        | HashMapKindedRef::TypedObject(_)
        | HashMapKindedRef::TraitObject(_) => {
            // Decimal / TypedObject / TraitObject value-arg projection
            // requires §2.7.6 / Q8 slot → pointer recovery with proper
            // share semantics. SURFACE-AND-STOP at ckpt-3: the projection
            // path requires careful refcount handling that's bounded by
            // ckpt-final scope (each variant's recovery shape mirrors
            // the §3ac2f11 5-arm receiver-recovery pattern). ADR-006
            // §2.7.24 Q25.B SUPERSEDED + audit §C.4.
            Err(VMError::RuntimeError(format!(
                "HashMap.set(): value-side projection for V={:?} is ckpt-final \
                 territory (5-arm receiver-recovery shape pending). \
                 ADR-006 §2.7.24 Q25.B SUPERSEDED.",
                map.values_kind()
            )))
        }
    }
}

/// Classify a value-slot's `NativeKind` to the matching HashMapKindedRef
/// variant's `values_kind()`. Used by the empty-HashMap V-promotion path
/// to decide if the variant needs to be rebuilt.
fn value_kind_hint(slot: &KindedSlot) -> NativeKind {
    slot.kind
}

/// Empty-HashMap V-promotion helper. Builds a new HashMapKindedRef
/// variant matching the incoming value's kind + inserts the first
/// entry. Returns `None` if the value kind isn't supported (caller
/// falls through to the regular kind-mismatch error path).
fn empty_set_with_promotion(
    key: &str,
    value_slot: &KindedSlot,
) -> Result<Option<HashMapKindedRef>, VMError> {
    use shape_value::heap_value::HashMapData;
    match value_slot.kind {
        NativeKind::Int64 => {
            let v = value_slot
                .as_i64()
                .ok_or_else(|| type_error("HashMap.set(): Int64 slot bits not a valid integer"))?;
            let mut data: HashMapData<i64> = HashMapData::new();
            unsafe { data.insert(key, v) };
            Ok(Some(HashMapKindedRef::I64(Arc::new(data))))
        }
        NativeKind::Float64 => {
            let v = value_slot
                .as_f64()
                .ok_or_else(|| type_error("HashMap.set(): Float64 slot bits not a valid f64"))?;
            let mut data: HashMapData<f64> = HashMapData::new();
            unsafe { data.insert(key, v) };
            Ok(Some(HashMapKindedRef::F64(Arc::new(data))))
        }
        NativeKind::Bool => {
            let b = value_slot
                .as_bool()
                .ok_or_else(|| type_error("HashMap.set(): Bool slot bits not a valid bool"))?;
            let mut data: HashMapData<u8> = HashMapData::new();
            unsafe { data.insert(key, if b { 1 } else { 0 }) };
            Ok(Some(HashMapKindedRef::Bool(Arc::new(data))))
        }
        NativeKind::Char => {
            let c = value_slot
                .as_char()
                .ok_or_else(|| type_error("HashMap.set(): Char slot bits not a valid char"))?;
            let mut data: HashMapData<char> = HashMapData::new();
            unsafe { data.insert(key, c) };
            Ok(Some(HashMapKindedRef::Char(Arc::new(data))))
        }
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            let v_ptr = string_slot_to_v2_ptr(value_slot).ok_or_else(|| {
                type_error("HashMap.set(): string slot bits could not be projected to *const StringObj")
            })?;
            let mut data: HashMapData<*const shape_value::v2::string_obj::StringObj> =
                HashMapData::new();
            unsafe { data.insert(key, v_ptr) };
            Ok(Some(HashMapKindedRef::String(Arc::new(data))))
        }
        // Other value kinds (Decimal / TypedObject / TraitObject) fall
        // through to the regular kind-mismatch path which surfaces
        // structured ckpt-final territory error.
        _ => Ok(None),
    }
}

/// Project a `KindedSlot` whose kind is `String` / `Ptr(HeapKind::String)`
/// to a fresh-share `*const StringObj`. Bumps v2_retain on the inner
/// allocation; returns the raw pointer (caller takes one share).
///
/// Returns `None` for non-string kinds. Used by `set_kinded`'s String arm.
fn string_slot_to_v2_ptr(slot: &KindedSlot) -> Option<*const shape_value::v2::string_obj::StringObj> {
    use shape_value::v2::string_obj::StringObj;
    // Two shapes carry strings:
    // 1. NativeKind::String — slot bits = Arc::into_raw::<String>, owned.
    // 2. NativeKind::Ptr(HeapKind::String) — slot bits = `Arc::into_raw(Arc<HeapValue>)`
    //    where HeapValue is HeapValue::String(Arc<String>).
    // In both cases we extract a `&str`, allocate a fresh StringObj with
    // v2_retain=1, and return that as the new owned share.
    let s_owned = match slot.kind {
        NativeKind::String => slot.as_str().map(|s| s.to_owned())?,
        NativeKind::Ptr(HeapKind::String) => match slot.slot.as_heap_value() {
            HeapValue::String(arc) => (**arc).clone(),
            _ => return None,
        },
        _ => return None,
    };
    Some(StringObj::new(&s_owned) as *const StringObj)
}

/// HashMap.delete(key) -> HashMap
///
/// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): per-V dispatch. Routes
/// through `HashMapData<V>::remove(&str)` via Arc::make_mut clone-on-write.
/// Returns the (possibly newly-cloned) HashMap as a new outer Arc<
/// HashMapKindedRef> wrapper post-removal — missing-key removals leave
/// the map unchanged. The removed value's share is retired immediately
/// (we drop it; if the caller wants the removed value use `remove()`).
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
    let map_arc = as_hashmap(&args[0])?;
    let key = as_string_key(&args[1])?.to_owned();
    let new_kref = delete_kinded(&map_arc, &key);
    Ok(KindedSlot::from_hashmap(std::sync::Arc::new(new_kref)))
}

/// Per-V delete: clone the inner `Arc<HashMapData<V>>`, remove the key,
/// drop the returned value's share. Returns the new HashMapKindedRef.
fn delete_kinded(map: &HashMapKindedRef, key: &str) -> HashMapKindedRef {
    match map {
        HashMapKindedRef::I64(arc) => {
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).remove(key) }; // POD: removed value drops trivially
            HashMapKindedRef::I64(new_arc)
        }
        HashMapKindedRef::F64(arc) => {
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            HashMapKindedRef::F64(new_arc)
        }
        HashMapKindedRef::Bool(arc) => {
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            HashMapKindedRef::Bool(new_arc)
        }
        HashMapKindedRef::Char(arc) => {
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            HashMapKindedRef::Char(new_arc)
        }
        HashMapKindedRef::String(arc) => {
            let mut new_arc = Arc::clone(arc);
            let removed = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            // Retire the removed *const StringObj share (HeapElement V).
            if let Some(ptr) = removed {
                unsafe {
                    use shape_value::v2::heap_element::HeapElement;
                    shape_value::v2::string_obj::StringObj::release_elem(ptr);
                }
            }
            HashMapKindedRef::String(new_arc)
        }
        HashMapKindedRef::Decimal(arc) => {
            let mut new_arc = Arc::clone(arc);
            let removed = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            if let Some(ptr) = removed {
                unsafe {
                    use shape_value::v2::heap_element::HeapElement;
                    shape_value::v2::decimal_obj::DecimalObj::release_elem(ptr);
                }
            }
            HashMapKindedRef::Decimal(new_arc)
        }
        HashMapKindedRef::TypedObject(arc) => {
            let mut new_arc = Arc::clone(arc);
            // TypedObjectPtr has a Drop impl — letting `removed` go out of
            // scope retires the share automatically.
            let _ = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            HashMapKindedRef::TypedObject(new_arc)
        }
        HashMapKindedRef::TraitObject(arc) => {
            let mut new_arc = Arc::clone(arc);
            let _ = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            HashMapKindedRef::TraitObject(new_arc)
        }
    }
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
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "HashMap.remove() requires exactly 1 argument (key)",
        ));
    }
    let map_arc = as_hashmap(&args[0])?;
    let key = as_string_key(&args[1])?.to_owned();
    // Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): tuple-return ABI per
    // ADR-006 §2.7.27 + W17-pop-mutation. Per-V dispatch: clone the inner
    // Arc, remove the entry, side-channel-publish the new map, return the
    // popped value (or `none` on miss).
    let (new_kref, popped_slot) = remove_kinded(&map_arc, &key);
    // Side-channel-publish NewContainer for compiler write-back.
    let new_self_slot = KindedSlot::from_hashmap(std::sync::Arc::new(new_kref));
    vm.push_kinded(new_self_slot.raw(), new_self_slot.kind())?;
    std::mem::forget(new_self_slot);
    Ok(popped_slot)
}

/// Per-V remove: clone the inner Arc<HashMapData<V>>, remove the key
/// (transferring the share to a KindedSlot for the caller). Returns
/// `(new HashMapKindedRef, popped KindedSlot)`. Missing key yields
/// `KindedSlot::none()`.
fn remove_kinded(map: &HashMapKindedRef, key: &str) -> (HashMapKindedRef, KindedSlot) {
    match map {
        HashMapKindedRef::I64(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            let slot = popped.map(KindedSlot::from_int).unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::I64(new_arc), slot)
        }
        HashMapKindedRef::F64(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            let slot = popped.map(KindedSlot::from_number).unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::F64(new_arc), slot)
        }
        HashMapKindedRef::Bool(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            let slot = popped
                .map(|v| KindedSlot::from_bool(v != 0))
                .unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::Bool(new_arc), slot)
        }
        HashMapKindedRef::Char(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            let slot = popped.map(KindedSlot::from_char).unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::Char(new_arc), slot)
        }
        HashMapKindedRef::String(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            // V = *const StringObj. Convert to NativeKind::String slot
            // (Arc<String>) for user-visible compatibility with `as_str()`
            // + downstream string-handling paths. Deep-copies the StringObj
            // content; retires the popped share.
            let slot = popped
                .map(|ptr| {
                    let s = unsafe {
                        shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned()
                    };
                    // Retire the popped StringObj share.
                    unsafe {
                        use shape_value::v2::heap_element::HeapElement;
                        shape_value::v2::string_obj::StringObj::release_elem(ptr);
                    }
                    KindedSlot::from_string_arc(Arc::new(s))
                })
                .unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::String(new_arc), slot)
        }
        HashMapKindedRef::Decimal(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            let slot = popped
                .map(KindedSlot::from_decimal_v2_ptr)
                .unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::Decimal(new_arc), slot)
        }
        HashMapKindedRef::TypedObject(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            // TypedObjectPtr: into_raw to transfer share to the slot.
            let slot = popped
                .map(|p| KindedSlot::from_typed_object_raw(p.into_raw()))
                .unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::TypedObject(new_arc), slot)
        }
        HashMapKindedRef::TraitObject(arc) => {
            let mut new_arc = Arc::clone(arc);
            let popped = unsafe { Arc::make_mut(&mut new_arc).remove(key) };
            let slot = popped
                .map(|p| KindedSlot::from_trait_object_raw(p.into_raw()))
                .unwrap_or_else(KindedSlot::none);
            (HashMapKindedRef::TraitObject(new_arc), slot)
        }
    }
}

/// HashMap.merge(other) -> HashMap
///
/// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): per-V dispatch. Routes
/// through `HashMapData<V>::merge(&other)` via Arc::make_mut clone-on-
/// write. Last-write-wins on key collision (matches `Object.assign` /
/// `dict.update` semantics). The `other` receiver is borrowed and its
/// shares are bumped via `share_clone` for each merged element.
///
/// Both receivers MUST have the same V variant (no V-flip / unification
/// at this layer). Mismatched V surfaces as a RuntimeError per playbook
/// §6 (no fallback coercion).
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
    let a = as_hashmap(&args[0])?;
    let b = as_hashmap(&args[1])?;
    let new_kref = merge_kinded(&a, &b)?;
    Ok(KindedSlot::from_hashmap(std::sync::Arc::new(new_kref)))
}

/// Per-V merge: requires both receivers have matching V. Clone the
/// inner Arc, call HashMapData<V>::merge, return the new carrier.
fn merge_kinded(a: &HashMapKindedRef, b: &HashMapKindedRef) -> Result<HashMapKindedRef, VMError> {
    let mismatch = || {
        type_error(format!(
            "HashMap.merge(): value-kind mismatch ({:?} vs {:?}); merge requires \
             same-V receivers at this layer",
            a.values_kind(),
            b.values_kind()
        ))
    };
    match (a, b) {
        (HashMapKindedRef::I64(arc_a), HashMapKindedRef::I64(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::I64(new_arc))
        }
        (HashMapKindedRef::F64(arc_a), HashMapKindedRef::F64(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::F64(new_arc))
        }
        (HashMapKindedRef::Bool(arc_a), HashMapKindedRef::Bool(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::Bool(new_arc))
        }
        (HashMapKindedRef::Char(arc_a), HashMapKindedRef::Char(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::Char(new_arc))
        }
        (HashMapKindedRef::String(arc_a), HashMapKindedRef::String(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::String(new_arc))
        }
        (HashMapKindedRef::Decimal(arc_a), HashMapKindedRef::Decimal(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::Decimal(new_arc))
        }
        (HashMapKindedRef::TypedObject(arc_a), HashMapKindedRef::TypedObject(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::TypedObject(new_arc))
        }
        (HashMapKindedRef::TraitObject(arc_a), HashMapKindedRef::TraitObject(arc_b)) => {
            let mut new_arc = Arc::clone(arc_a);
            unsafe { Arc::make_mut(&mut new_arc).merge(arc_b) };
            Ok(HashMapKindedRef::TraitObject(new_arc))
        }
        _ => Err(mismatch()),
    }
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
