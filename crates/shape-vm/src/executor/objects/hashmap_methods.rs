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
use shape_value::heap_value::{
    HashMapData, HashMapKindedRef, HashMapValueElem, HeapKind, HeapValue,
    TraitObjectPtr, TypedObjectPtr, TypedObjectStorage,
};
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder (TypedArrayData-result methods)
// ═══════════════════════════════════════════════════════════════════════════

#[cold]
#[inline(never)]
fn ckpt5_hashmap_array_surface(op: &'static str) -> VMError {
    VMError::NotImplemented(format!(
        "HashMap.{op}: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 \
         surface. `Arc<TypedArrayData>` result carrier DELETED at V3-S5 \
         ckpt-1..ckpt-4 per W12-typed-array-data-deletion audit §3.5 + \
         §3.6 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. Rebuild lands at \
         ckpt-6 STRICT close per the per-T v2-raw `TypedArray<T>` carrier \
         shape. REFUSED ON SIGHT: TypedArrayData resurrection under any \
         rename (Refusal #1).",
        op = op,
    ))
}

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Read all keys from a `*mut TypedArray<*const StringObj>` keys buffer as
/// owned `Vec<Arc<String>>`. Each StringObj's content is deep-copied (no
/// share-handoff of the v2-raw `*const StringObj`).
///
/// Used by `v2_keys`, `v2_entries`, closure-based methods, and the iterator
/// `hashmap_elem_at` projection — every path that needs `Arc<String>`-keyed
/// representation rather than the v2-raw `*const StringObj`.
///
/// Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14). ADR-006 §2.7.24 Q25.B
/// SUPERSEDED + audit §C.4.
///
/// # Safety
/// `keys` must point to a live `TypedArray<*const StringObj>` whose elements
/// are live StringObjs (the `HashMapData<V>` contract).
unsafe fn read_keys_owned(
    keys: *const shape_value::v2::typed_array::TypedArray<
        *const shape_value::v2::string_obj::StringObj,
    >,
) -> Vec<Arc<String>> {
    let n = unsafe { shape_value::v2::typed_array::TypedArray::len(keys) as usize };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let ptr = unsafe {
            shape_value::v2::typed_array::TypedArray::get_unchecked(keys, i as u32)
        };
        let s = unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned() };
        out.push(Arc::new(s));
    }
    out
}

/// Per-V values count for a `HashMapKindedRef`. Cheap len() walk.
#[inline]
fn kref_len(kref: &HashMapKindedRef) -> usize {
    kref.len()
}

/// Schema id for the canonical `{key, value}` Entry TypedObject. Lazily
/// registered on first use via `register_predeclared_any_schema`.
///
/// W17-typed-carrier-bundle-A checkpoint-2 amendment (per phase-2d-playbook
/// §3): HashMap entries are TypedObjects with named fields rather than
/// 2-element arrays. User code accesses `entry.key` / `entry.value`.
fn ensure_entry_schema() -> u32 {
    let fields = [String::from("key"), String::from("value")];
    shape_runtime::type_schema::register_predeclared_any_schema(&fields)
}

/// Build a single `{key, value}` Entry TypedObject. The value slot's kind
/// is supplied by the caller (per the V-arm dispatch). Caller transfers one
/// share on `key_arc` and one share on the value-slot bits to this method;
/// returned `TypedObjectPtr` owns both shares (refcount=1).
fn build_entry_object(
    key_arc: Arc<String>,
    value_bits: u64,
    value_kind: NativeKind,
) -> TypedObjectPtr {
    let schema_id = ensure_entry_schema();
    let key_bits = Arc::into_raw(key_arc) as u64;
    let slots: Box<[ValueSlot]> = Box::new([
        ValueSlot::from_raw(key_bits),
        ValueSlot::from_raw(value_bits),
    ]);
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::String, value_kind].into_boxed_slice());
    // Both fields are heap-resident (String + value either heap or scalar;
    // the heap_mask is per-field for Drop dispatch — for scalar value kinds
    // the matching `drop_with_kind` arm is a no-op).
    let heap_mask: u64 = 0b11;
    let ptr = TypedObjectStorage::_new(
        schema_id as u64,
        slots,
        heap_mask,
        field_kinds,
    );
    TypedObjectPtr::new(ptr)
}

/// Build an Entry TypedObject for a single (key, value) pair from a
/// `HashMapKindedRef` at index `i`. Per-V dispatch reads the value at index
/// and bumps refcount appropriately (HeapElement V: `v2_retain` on the
/// HeapHeader; Ptr-newtype V: wrapper's Clone bumps refcount; POD V: byte
/// copy).
///
/// Returns a `TypedObjectPtr` with refcount=1 (owned share for caller).
fn entry_object_at(kref: &HashMapKindedRef, i: usize, key_arc: Arc<String>) -> TypedObjectPtr {
    match kref {
        HashMapKindedRef::I64(arc) => {
            let v: i64 = unsafe { *(*arc.values).data.add(i) };
            build_entry_object(key_arc, v as u64, NativeKind::Int64)
        }
        HashMapKindedRef::F64(arc) => {
            let v: f64 = unsafe { *(*arc.values).data.add(i) };
            build_entry_object(key_arc, v.to_bits(), NativeKind::Float64)
        }
        HashMapKindedRef::Bool(arc) => {
            let v: u8 = unsafe { *(*arc.values).data.add(i) };
            build_entry_object(key_arc, v as u64, NativeKind::Bool)
        }
        HashMapKindedRef::Char(arc) => {
            let v: char = unsafe { *(*arc.values).data.add(i) };
            build_entry_object(key_arc, v as u64, NativeKind::Char)
        }
        HashMapKindedRef::String(arc) => {
            // V = *const StringObj. Deep-copy to Arc<String> for the slot
            // (matches v2_get's V=String projection). Caller-owned Arc<String>
            // share goes into the slot.
            let ptr: *const shape_value::v2::string_obj::StringObj =
                unsafe { *(*arc.values).data.add(i) };
            let s = unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned() };
            let arc_s = Arc::new(s);
            let value_bits = Arc::into_raw(arc_s) as u64;
            build_entry_object(key_arc, value_bits, NativeKind::String)
        }
        HashMapKindedRef::Decimal(arc) => {
            // V = *const DecimalObj. Bump refcount; pointer becomes slot bits.
            let ptr: *const shape_value::v2::decimal_obj::DecimalObj =
                unsafe { *(*arc.values).data.add(i) };
            unsafe {
                shape_value::v2::refcount::v2_retain(&(*ptr).header);
            }
            build_entry_object(key_arc, ptr as u64, NativeKind::DecimalV2)
        }
        HashMapKindedRef::TypedObject(arc) => {
            // V = TypedObjectPtr. Clone bumps refcount via v2_retain on
            // the inner *const TypedObjectStorage. Convert to slot bits.
            let elem: &TypedObjectPtr = unsafe { &*(*arc.values).data.add(i) };
            let bumped: TypedObjectPtr = elem.clone();
            let ptr = bumped.into_raw();
            build_entry_object(key_arc, ptr as u64, NativeKind::Ptr(HeapKind::TypedObject))
        }
        HashMapKindedRef::TraitObject(arc) => {
            // V = TraitObjectPtr. Mirror of TypedObject.
            let elem: &TraitObjectPtr = unsafe { &*(*arc.values).data.add(i) };
            let bumped: TraitObjectPtr = elem.clone();
            let ptr = bumped.into_raw();
            build_entry_object(key_arc, ptr as u64, NativeKind::Ptr(HeapKind::TraitObject))
        }
    }
}

/// Build a fresh `HashMapKindedRef` of the same V variant as `src`,
/// containing only the entries at `kept_indices` (in input order). Each
/// kept value is share-cloned (refcount bump for HeapElement / Ptr-newtype
/// V; byte copy for POD V) via `HashMapValueElem::share_clone`.
///
/// Used by `v2_filter` — the surviving entries form a subset of the source
/// keyed on the same V variant (filter doesn't introduce new value kinds).
fn build_filtered_kref(
    src: &HashMapKindedRef,
    kept_indices: &[usize],
    kept_keys: &[Arc<String>],
) -> Result<HashMapKindedRef, VMError> {
    debug_assert_eq!(kept_indices.len(), kept_keys.len());
    Ok(match src {
        HashMapKindedRef::I64(arc) => {
            let mut data: HashMapData<i64> = HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let v: i64 = unsafe { *(*arc.values).data.add(*slot) };
                unsafe { data.insert(key.as_str(), v) };
            }
            HashMapKindedRef::I64(Arc::new(data))
        }
        HashMapKindedRef::F64(arc) => {
            let mut data: HashMapData<f64> = HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let v: f64 = unsafe { *(*arc.values).data.add(*slot) };
                unsafe { data.insert(key.as_str(), v) };
            }
            HashMapKindedRef::F64(Arc::new(data))
        }
        HashMapKindedRef::Bool(arc) => {
            let mut data: HashMapData<u8> = HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let v: u8 = unsafe { *(*arc.values).data.add(*slot) };
                unsafe { data.insert(key.as_str(), v) };
            }
            HashMapKindedRef::Bool(Arc::new(data))
        }
        HashMapKindedRef::Char(arc) => {
            let mut data: HashMapData<char> = HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let v: char = unsafe { *(*arc.values).data.add(*slot) };
                unsafe { data.insert(key.as_str(), v) };
            }
            HashMapKindedRef::Char(Arc::new(data))
        }
        HashMapKindedRef::String(arc) => {
            let mut data: HashMapData<*const shape_value::v2::string_obj::StringObj> =
                HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let elem_ref: &*const shape_value::v2::string_obj::StringObj =
                    unsafe { &*(*arc.values).data.add(*slot) };
                // share_clone v2_retains the inner StringObj; the cloned
                // share is transferred into the new HashMapData via insert.
                let cloned = unsafe {
                    <*const shape_value::v2::string_obj::StringObj
                        as HashMapValueElem>::share_clone(elem_ref)
                };
                unsafe { data.insert(key.as_str(), cloned) };
            }
            HashMapKindedRef::String(Arc::new(data))
        }
        HashMapKindedRef::Decimal(arc) => {
            let mut data: HashMapData<*const shape_value::v2::decimal_obj::DecimalObj> =
                HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let elem_ref: &*const shape_value::v2::decimal_obj::DecimalObj =
                    unsafe { &*(*arc.values).data.add(*slot) };
                let cloned = unsafe {
                    <*const shape_value::v2::decimal_obj::DecimalObj
                        as HashMapValueElem>::share_clone(elem_ref)
                };
                unsafe { data.insert(key.as_str(), cloned) };
            }
            HashMapKindedRef::Decimal(Arc::new(data))
        }
        HashMapKindedRef::TypedObject(arc) => {
            let mut data: HashMapData<TypedObjectPtr> = HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let elem_ref: &TypedObjectPtr =
                    unsafe { &*(*arc.values).data.add(*slot) };
                let cloned = unsafe {
                    <TypedObjectPtr as HashMapValueElem>::share_clone(elem_ref)
                };
                unsafe { data.insert(key.as_str(), cloned) };
            }
            HashMapKindedRef::TypedObject(Arc::new(data))
        }
        HashMapKindedRef::TraitObject(arc) => {
            let mut data: HashMapData<TraitObjectPtr> = HashMapData::new();
            for (slot, key) in kept_indices.iter().zip(kept_keys.iter()) {
                let elem_ref: &TraitObjectPtr =
                    unsafe { &*(*arc.values).data.add(*slot) };
                let cloned = unsafe {
                    <TraitObjectPtr as HashMapValueElem>::share_clone(elem_ref)
                };
                unsafe { data.insert(key.as_str(), cloned) };
            }
            HashMapKindedRef::TraitObject(Arc::new(data))
        }
    })
}

/// Read a single (key_slot, value_slot) pair from a `HashMapKindedRef` at
/// index `i`. The key slot is `NativeKind::String` (Arc<String>). The value
/// slot uses the matching per-V `KindedSlot::from_*` constructor with one
/// owned share. Used by closure-based methods (forEach, map, filter, reduce,
/// groupBy) to feed each per-entry callback.
fn read_entry_kinded(kref: &HashMapKindedRef, i: usize, key_arc: Arc<String>) -> (KindedSlot, KindedSlot) {
    let key_slot = KindedSlot::from_string_arc(key_arc);
    let value_slot = value_slot_at(kref, i);
    (key_slot, value_slot)
}

/// Read the value at index `i` as a `KindedSlot` (one owned share). Per-V
/// dispatch parallels `entry_object_at`'s value-projection arms.
fn value_slot_at(kref: &HashMapKindedRef, i: usize) -> KindedSlot {
    match kref {
        HashMapKindedRef::I64(arc) => {
            let v: i64 = unsafe { *(*arc.values).data.add(i) };
            KindedSlot::from_int(v)
        }
        HashMapKindedRef::F64(arc) => {
            let v: f64 = unsafe { *(*arc.values).data.add(i) };
            KindedSlot::from_number(v)
        }
        HashMapKindedRef::Bool(arc) => {
            let v: u8 = unsafe { *(*arc.values).data.add(i) };
            KindedSlot::from_bool(v != 0)
        }
        HashMapKindedRef::Char(arc) => {
            let v: char = unsafe { *(*arc.values).data.add(i) };
            KindedSlot::from_char(v)
        }
        HashMapKindedRef::String(arc) => {
            let ptr: *const shape_value::v2::string_obj::StringObj =
                unsafe { *(*arc.values).data.add(i) };
            let s = unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned() };
            KindedSlot::from_string_arc(Arc::new(s))
        }
        HashMapKindedRef::Decimal(arc) => {
            let ptr: *const shape_value::v2::decimal_obj::DecimalObj =
                unsafe { *(*arc.values).data.add(i) };
            unsafe {
                shape_value::v2::refcount::v2_retain(&(*ptr).header);
            }
            KindedSlot::from_decimal_v2_ptr(ptr)
        }
        HashMapKindedRef::TypedObject(arc) => {
            let elem: &TypedObjectPtr = unsafe { &*(*arc.values).data.add(i) };
            let bumped: TypedObjectPtr = elem.clone();
            KindedSlot::from_typed_object_raw(bumped.into_raw())
        }
        HashMapKindedRef::TraitObject(arc) => {
            let elem: &TraitObjectPtr = unsafe { &*(*arc.values).data.add(i) };
            let bumped: TraitObjectPtr = elem.clone();
            KindedSlot::from_trait_object_raw(bumped.into_raw())
        }
    }
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
        // V3-S5 ckpt-6 STRICT close (2026-05-15): `HeapValue::TypedArray(a)
        // => KindedSlot::from_typed_array(Arc::clone(a))` arm DELETED in
        // lockstep with the outer `HeapValue::TypedArray` variant + the
        // `KindedSlot::from_typed_array(Arc<...>)` convenience constructor
        // (ADR-006 §2.7.24 Q25.A SUPERSEDED). Per-element-T v2-raw
        // `*mut TypedArray<T>` re-wrapping is downstream-wave territory.
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
    let _ = as_hashmap(&args[0])?;
    // Suppress unused-fn warning for the helper used by the deleted
    // body; the helper itself has no TypedArrayData dependency.
    let _ = read_keys_owned;
    Err(ckpt5_hashmap_array_surface("keys"))
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
    let _ = as_hashmap(&args[0])?;
    let _ = kref_len;
    Err(ckpt5_hashmap_array_surface("values"))
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
    let _ = as_hashmap(receiver)?;
    // Suppress unused-fn warnings for helpers used by the deleted body.
    let _ = entry_object_at;
    Err(ckpt5_hashmap_array_surface("entries/toArray"))
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
        HashMapKindedRef::Decimal(arc) => {
            // V = *const DecimalObj. Project from value_slot's kind ==
            // DecimalV2 (raw pointer carrier) or Ptr(HeapKind::Decimal)
            // (Arc<rust_decimal::Decimal> carrier — deep-copy needed).
            //
            // 5-arm receiver-recovery (phase-2d-handover.md §0): the
            // recovery clones-the-share, never moves the slot's original.
            let v_ptr: *const shape_value::v2::decimal_obj::DecimalObj =
                match value_slot.kind {
                    NativeKind::DecimalV2 => {
                        let bits = value_slot.slot.raw();
                        if bits == 0 {
                            return Err(type_error(
                                "HashMap.set(): DecimalV2 slot bits null",
                            ));
                        }
                        // Bump v2_retain so the inserted share is fresh and
                        // independent of the slot's own share.
                        let ptr = bits as *const shape_value::v2::decimal_obj::DecimalObj;
                        unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
                        ptr
                    }
                    NativeKind::Ptr(HeapKind::Decimal) => {
                        // Slot carries Arc<rust_decimal::Decimal> via
                        // HeapValue::Decimal. Deep-copy to a v2-raw
                        // DecimalObj with refcount=1.
                        match value_slot.slot.as_heap_value() {
                            HeapValue::Decimal(d) => {
                                shape_value::v2::decimal_obj::DecimalObj::new(**d)
                                    as *const _
                            }
                            _ => {
                                return Err(type_error(
                                    "HashMap.set(): Ptr(Decimal) slot heap arm mismatched",
                                ))
                            }
                        }
                    }
                    other => {
                        return Err(type_error(format!(
                            "HashMap.set(): value kind {:?} incompatible with HashMap<string, decimal>",
                            other
                        )))
                    }
                };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v_ptr) };
            Ok(HashMapKindedRef::Decimal(new_arc))
        }
        HashMapKindedRef::TypedObject(arc) => {
            // V = TypedObjectPtr. 5-arm receiver-recovery: kind ==
            // Ptr(HeapKind::TypedObject) slot bits are
            // `*const TypedObjectStorage` (v2-raw raw-pointer carrier per
            // ADR-006 §2.3). Bump v2_retain via the storage's header to
            // build a fresh wrapper-share, independent of the slot's
            // own share. Casting via `as_heap_value()` here would be
            // unsound (TypedObject slot bits are NOT Arc::into_raw of an
            // outer Arc<HeapValue>; they are the raw storage pointer).
            let v_ptr: TypedObjectPtr = match value_slot.kind {
                NativeKind::Ptr(HeapKind::TypedObject) => {
                    let bits = value_slot.slot.raw();
                    if bits == 0 {
                        return Err(type_error(
                            "HashMap.set(): TypedObject slot bits null",
                        ));
                    }
                    let ptr = bits as *const TypedObjectStorage;
                    unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
                    TypedObjectPtr::new(ptr)
                }
                other => {
                    return Err(type_error(format!(
                        "HashMap.set(): value kind {:?} incompatible with HashMap<string, TypedObject>",
                        other
                    )))
                }
            };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v_ptr) };
            Ok(HashMapKindedRef::TypedObject(new_arc))
        }
        HashMapKindedRef::TraitObject(arc) => {
            // V = TraitObjectPtr. Mirror of TypedObject — 5-arm
            // receiver-recovery via v2_retain on the inner storage's
            // HeapHeader.
            let v_ptr: TraitObjectPtr = match value_slot.kind {
                NativeKind::Ptr(HeapKind::TraitObject) => {
                    let bits = value_slot.slot.raw();
                    if bits == 0 {
                        return Err(type_error(
                            "HashMap.set(): TraitObject slot bits null",
                        ));
                    }
                    let ptr = bits as *const shape_value::heap_value::TraitObjectStorage;
                    unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
                    TraitObjectPtr::new(ptr)
                }
                other => {
                    return Err(type_error(format!(
                        "HashMap.set(): value kind {:?} incompatible with HashMap<string, TraitObject>",
                        other
                    )))
                }
            };
            let mut new_arc = Arc::clone(arc);
            unsafe { Arc::make_mut(&mut new_arc).insert(key, v_ptr) };
            Ok(HashMapKindedRef::TraitObject(new_arc))
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
        NativeKind::DecimalV2 => {
            let bits = value_slot.slot.raw();
            if bits == 0 {
                return Err(type_error("HashMap.set(): DecimalV2 slot bits null"));
            }
            let ptr = bits as *const shape_value::v2::decimal_obj::DecimalObj;
            unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
            let mut data: HashMapData<*const shape_value::v2::decimal_obj::DecimalObj> =
                HashMapData::new();
            unsafe { data.insert(key, ptr) };
            Ok(Some(HashMapKindedRef::Decimal(Arc::new(data))))
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            match value_slot.slot.as_heap_value() {
                HeapValue::Decimal(d) => {
                    let new_obj = shape_value::v2::decimal_obj::DecimalObj::new(**d);
                    let mut data: HashMapData<
                        *const shape_value::v2::decimal_obj::DecimalObj,
                    > = HashMapData::new();
                    unsafe { data.insert(key, new_obj as *const _) };
                    Ok(Some(HashMapKindedRef::Decimal(Arc::new(data))))
                }
                _ => Ok(None),
            }
        }
        NativeKind::Ptr(HeapKind::TypedObject) => {
            let bits = value_slot.slot.raw();
            if bits == 0 {
                return Err(type_error("HashMap.set(): TypedObject slot bits null"));
            }
            let ptr = bits as *const TypedObjectStorage;
            unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
            let to_ptr = TypedObjectPtr::new(ptr);
            let mut data: HashMapData<TypedObjectPtr> = HashMapData::new();
            unsafe { data.insert(key, to_ptr) };
            Ok(Some(HashMapKindedRef::TypedObject(Arc::new(data))))
        }
        NativeKind::Ptr(HeapKind::TraitObject) => {
            let bits = value_slot.slot.raw();
            if bits == 0 {
                return Err(type_error("HashMap.set(): TraitObject slot bits null"));
            }
            let ptr = bits as *const shape_value::heap_value::TraitObjectStorage;
            unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
            let tr_ptr = TraitObjectPtr::new(ptr);
            let mut data: HashMapData<TraitObjectPtr> = HashMapData::new();
            unsafe { data.insert(key, tr_ptr) };
            Ok(Some(HashMapKindedRef::TraitObject(Arc::new(data))))
        }
        // Other value kinds fall through to the regular kind-mismatch path.
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
    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): clone the receiver
    // Arc up-front so the iteration borrow is independent of the
    // &mut VirtualMachine reborrow on each call_value_immediate_nb call.
    let map = as_hashmap(&args[0])?;
    let closure = &args[1];
    let keys_ptr = match &*map {
        HashMapKindedRef::I64(arc) => arc.keys,
        HashMapKindedRef::F64(arc) => arc.keys,
        HashMapKindedRef::Bool(arc) => arc.keys,
        HashMapKindedRef::Char(arc) => arc.keys,
        HashMapKindedRef::String(arc) => arc.keys,
        HashMapKindedRef::Decimal(arc) => arc.keys,
        HashMapKindedRef::TypedObject(arc) => arc.keys,
        HashMapKindedRef::TraitObject(arc) => arc.keys,
    };
    let keys_vec: Vec<Arc<String>> = unsafe { read_keys_owned(keys_ptr) };
    for (i, key_arc) in keys_vec.into_iter().enumerate() {
        let (key_slot, value_slot) = read_entry_kinded(&map, i, key_arc);
        let _result = vm.call_value_immediate_nb(
            closure,
            &[key_slot, value_slot],
            ctx.as_deref_mut(),
        )?;
        // Result is discarded — drops automatically when KindedSlot drops.
    }
    Ok(KindedSlot::none())
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
    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V iteration. The
    // surviving entries get cloned (share-bump) into a fresh
    // HashMapData<V> of the same V variant. Same-V invariant is structural
    // (filter doesn't introduce new kinds).
    let map = as_hashmap(&args[0])?;
    let closure = &args[1];
    let keys_ptr = match &*map {
        HashMapKindedRef::I64(arc) => arc.keys,
        HashMapKindedRef::F64(arc) => arc.keys,
        HashMapKindedRef::Bool(arc) => arc.keys,
        HashMapKindedRef::Char(arc) => arc.keys,
        HashMapKindedRef::String(arc) => arc.keys,
        HashMapKindedRef::Decimal(arc) => arc.keys,
        HashMapKindedRef::TypedObject(arc) => arc.keys,
        HashMapKindedRef::TraitObject(arc) => arc.keys,
    };
    let keys_vec: Vec<Arc<String>> = unsafe { read_keys_owned(keys_ptr) };
    // Walk once invoking the predicate; record kept indices.
    let mut kept_indices: Vec<usize> = Vec::with_capacity(keys_vec.len());
    let mut kept_keys: Vec<Arc<String>> = Vec::new();
    for (i, key_arc) in keys_vec.into_iter().enumerate() {
        let (key_slot, value_slot) =
            read_entry_kinded(&map, i, Arc::clone(&key_arc));
        let result = vm.call_value_immediate_nb(
            closure,
            &[key_slot, value_slot],
            ctx.as_deref_mut(),
        )?;
        match result.kind {
            NativeKind::Bool => {
                if result.slot.raw() != 0 {
                    kept_indices.push(i);
                    kept_keys.push(key_arc);
                }
            }
            other => {
                return Err(type_error(format!(
                    "HashMap.filter(): predicate must return bool, got kind {:?}",
                    other
                )))
            }
        }
    }
    let kept = build_filtered_kref(&map, &kept_indices, &kept_keys)?;
    Ok(KindedSlot::from_hashmap(Arc::new(kept)))
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
    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V iteration into a
    // closure-driven value rewrite. Result V is dispatched on the first
    // result kind; subsequent results must match (homogeneous-result
    // invariant — same kind soundness as Iterator.collect at playbook §6).
    let map = as_hashmap(&args[0])?;
    let closure = &args[1];
    let keys_ptr = match &*map {
        HashMapKindedRef::I64(arc) => arc.keys,
        HashMapKindedRef::F64(arc) => arc.keys,
        HashMapKindedRef::Bool(arc) => arc.keys,
        HashMapKindedRef::Char(arc) => arc.keys,
        HashMapKindedRef::String(arc) => arc.keys,
        HashMapKindedRef::Decimal(arc) => arc.keys,
        HashMapKindedRef::TypedObject(arc) => arc.keys,
        HashMapKindedRef::TraitObject(arc) => arc.keys,
    };
    let keys_vec: Vec<Arc<String>> = unsafe { read_keys_owned(keys_ptr) };
    let mut results: Vec<(Arc<String>, KindedSlot)> = Vec::with_capacity(keys_vec.len());
    for (i, key_arc) in keys_vec.into_iter().enumerate() {
        let (key_slot, value_slot) =
            read_entry_kinded(&map, i, Arc::clone(&key_arc));
        let result = vm.call_value_immediate_nb(
            closure,
            &[key_slot, value_slot],
            ctx.as_deref_mut(),
        )?;
        results.push((key_arc, result));
    }
    let kref = build_kref_from_kinded_results(results)?;
    Ok(KindedSlot::from_hashmap(Arc::new(kref)))
}

/// Build a `HashMapKindedRef` from a `Vec<(Arc<String>, KindedSlot)>` where
/// each `KindedSlot` is one owned share. The V variant is dispatched on the
/// first slot's kind; subsequent slots must match (homogeneous-result
/// invariant). Each slot's share is transferred into the new HashMapData
/// (POD V byte-copy; HeapElement / Ptr-newtype V via raw-pointer extraction
/// matching the per-V kind).
fn build_kref_from_kinded_results(
    results: Vec<(Arc<String>, KindedSlot)>,
) -> Result<HashMapKindedRef, VMError> {
    if results.is_empty() {
        // Empty pipeline — pick a default (V=String; matches HashMap()
        // ctor default). Caller can later promote via the empty-V-promotion
        // path in v2_set if they insert other kinds.
        return Ok(HashMapKindedRef::String(Arc::new(HashMapData::new())));
    }
    let first_kind = results[0].1.kind;
    // Validate homogeneity.
    for (i, (_, slot)) in results.iter().enumerate().skip(1) {
        if slot.kind != first_kind {
            return Err(type_error(format!(
                "HashMap.map(): heterogeneous-kind results not supported \
                 (element 0 kind={:?}, element {} kind={:?})",
                first_kind, i, slot.kind
            )));
        }
    }
    match first_kind {
        NativeKind::Int64 => {
            let mut data: HashMapData<i64> = HashMapData::new();
            for (key, slot) in results.iter() {
                let v = slot.as_i64().ok_or_else(|| {
                    type_error("HashMap.map(): Int64 slot bits invalid")
                })?;
                unsafe { data.insert(key.as_str(), v) };
            }
            // Drop the source slots (POD: byte-copy fall-out).
            drop(results);
            Ok(HashMapKindedRef::I64(Arc::new(data)))
        }
        NativeKind::Float64 => {
            let mut data: HashMapData<f64> = HashMapData::new();
            for (key, slot) in results.iter() {
                let v = slot.as_f64().ok_or_else(|| {
                    type_error("HashMap.map(): Float64 slot bits invalid")
                })?;
                unsafe { data.insert(key.as_str(), v) };
            }
            drop(results);
            Ok(HashMapKindedRef::F64(Arc::new(data)))
        }
        NativeKind::Bool => {
            let mut data: HashMapData<u8> = HashMapData::new();
            for (key, slot) in results.iter() {
                let b = slot.as_bool().ok_or_else(|| {
                    type_error("HashMap.map(): Bool slot bits invalid")
                })?;
                unsafe { data.insert(key.as_str(), if b { 1 } else { 0 }) };
            }
            drop(results);
            Ok(HashMapKindedRef::Bool(Arc::new(data)))
        }
        NativeKind::Char => {
            let mut data: HashMapData<char> = HashMapData::new();
            for (key, slot) in results.iter() {
                let c = slot.as_char().ok_or_else(|| {
                    type_error("HashMap.map(): Char slot bits invalid")
                })?;
                unsafe { data.insert(key.as_str(), c) };
            }
            drop(results);
            Ok(HashMapKindedRef::Char(Arc::new(data)))
        }
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            // V = *const StringObj. Each result slot carries an
            // Arc::into_raw::<String> share; deep-copy content into a
            // fresh StringObj (one v2_retain share), drop the source
            // Arc<String> share at end-of-iteration.
            let mut data: HashMapData<*const shape_value::v2::string_obj::StringObj> =
                HashMapData::new();
            for (key, slot) in results.iter() {
                let s_arc: Arc<String> = {
                    let bits = slot.slot.raw();
                    if bits == 0 {
                        return Err(type_error("HashMap.map(): String slot bits null"));
                    }
                    // Bump the source share so the local clone is independent.
                    unsafe {
                        Arc::increment_strong_count(bits as *const String);
                        Arc::from_raw(bits as *const String)
                    }
                };
                let new_obj = shape_value::v2::string_obj::StringObj::new(s_arc.as_str());
                unsafe { data.insert(key.as_str(), new_obj as *const _) };
                drop(s_arc); // releases our local bump; slot's own Drop retires the original
            }
            // results' slots Drop normally — each retires its share via §2.7.7/Q9 dispatch
            Ok(HashMapKindedRef::String(Arc::new(data)))
        }
        NativeKind::Ptr(HeapKind::TypedObject) => {
            let mut data: HashMapData<TypedObjectPtr> = HashMapData::new();
            for (key, slot) in results.iter() {
                let bits = slot.slot.raw();
                if bits == 0 {
                    return Err(type_error("HashMap.map(): TypedObject slot bits null"));
                }
                // Bump v2_retain on the inner storage; build a fresh
                // TypedObjectPtr owning the new share.
                let ptr = bits as *const TypedObjectStorage;
                unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
                unsafe { data.insert(key.as_str(), TypedObjectPtr::new(ptr)) };
            }
            // results' slots Drop normally and retire their original shares.
            Ok(HashMapKindedRef::TypedObject(Arc::new(data)))
        }
        other => Err(type_error(format!(
            "HashMap.map(): result kind {:?} not supported — only \
             int, number, bool, char, string, TypedObject result Vs land at \
             ckpt-4 (Decimal / TraitObject value V requires a separate \
             pull-from-slot helper, tracked alongside HashMap value-side V \
             cluster).",
            other
        ))),
    }
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
    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): thread accumulator
    // through per-entry callback. Closure signature is
    // `(acc, key, value) -> acc`. Returns the final accumulator.
    let map = as_hashmap(&args[0])?;
    let closure = &args[1];
    let mut acc = args[2].clone();
    let keys_ptr = match &*map {
        HashMapKindedRef::I64(arc) => arc.keys,
        HashMapKindedRef::F64(arc) => arc.keys,
        HashMapKindedRef::Bool(arc) => arc.keys,
        HashMapKindedRef::Char(arc) => arc.keys,
        HashMapKindedRef::String(arc) => arc.keys,
        HashMapKindedRef::Decimal(arc) => arc.keys,
        HashMapKindedRef::TypedObject(arc) => arc.keys,
        HashMapKindedRef::TraitObject(arc) => arc.keys,
    };
    let keys_vec: Vec<Arc<String>> = unsafe { read_keys_owned(keys_ptr) };
    for (i, key_arc) in keys_vec.into_iter().enumerate() {
        let (key_slot, value_slot) = read_entry_kinded(&map, i, key_arc);
        let result = vm.call_value_immediate_nb(
            closure,
            &[acc, key_slot, value_slot],
            ctx.as_deref_mut(),
        )?;
        acc = result;
    }
    Ok(acc)
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
    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): walk entries, call
    // closure to derive group_key, accumulate per-group entries. Each
    // bucket is a fresh HashMap with the same V variant as the source.
    // The outer HashMap is HashMap<string, HashMap<…>> — V is the inner
    // kinded ref (since HashMaps are heap pointers, V = TypedObject is
    // NOT the right shape; we use the kref/HashMap-of-HashMap shape via
    // V = TypedObject only if we wrap the inner HashMap as a TypedObject,
    // which is unidiomatic).
    //
    // The clean way: outer is `HashMap<string, HashMap>` but our
    // HashMapKindedRef enum doesn't have a `HashMap` value V arm. The
    // canonical workaround per audit §C.4 + ADR-006 §2.3 is to store
    // the inner HashMaps as v2 heap entries via TypedObject-wrapping.
    // That requires a non-trivial cluster (TypedObject schema for a
    // generic HashMap-of-HashMap surface). SURFACE-AND-STOP cleanly per
    // playbook §6 rather than introducing a degraded carrier shape.
    let _ = as_hashmap(&args[0])?;
    let _ = (vm, ctx, &args[1]);
    Err(VMError::NotImplemented(
        "HashMap.groupBy(): outer HashMap<string, HashMap> carrier requires \
         a HashMap-value V arm in HashMapKindedRef, which is not landed and \
         would expand cluster-0+1 scope. Surface-and-stop per playbook §6 \
         (no degraded HashMap-as-TypedObject wrapper). Tracked as \
         hashmap-value-v-arm in the follow-up cluster.".into(),
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
