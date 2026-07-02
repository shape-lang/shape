//! Method handlers for the Set collection type.
//!
//! ## W13-hashset-rebuild migration (2026-05-10)
//!
//! Per ADR-006 §2.7.15 / Q16 amendment (Wave 13 W13-hashset-rebuild),
//! the Set carrier is a typed-`Arc<HashSetData>`-backed `HeapValue`
//! arm — full HeapValue arm, not pure-discriminator like FilterExpr /
//! SharedCell. Set is a HashMap sibling: insertion-ordered keys + eager
//! FNV-1a bucket index, with the values buffer dropped. W74B extends the
//! original string arm with an explicit `i64` arm to match the public
//! `Set<T>` contract for integer elements.
//!
//! Most handlers (`add`, `has`, `delete`, `len`/`length`, `is_empty`,
//! `to_array`, `union`, `intersection`, `difference`, `for_each`,
//! `filter`) are live bodies on top of the post-§2.7.15 `HashSetData`
//! shape (`shape_value::heap_value::HashSetData`). `map` surfaces until
//! its result element kind is statically available at this method boundary.
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::HashSet)`, then reconstruct
//! `Arc<HashSetData>` directly from the slot bits because
//! `KindedSlot::from_hashset` stores `Arc::into_raw(Arc<HashSetData>)`.
//!
//! Per-key kind validation is exact: `Int64` is accepted only by the `i64`
//! arm and string carriers only by the string arm. Empty sets still carry a
//! static arm from construction; handlers never infer the arm from the first
//! runtime element. Mixed Set element kinds surface a `RuntimeError`; no key
//! coercion or stringification is performed.
//!
//! Result construction follows playbook §3:
//! - `len` / `is_empty` / `has` → inline-scalar `KindedSlot::from_int`
//!   / `from_bool`.
//! - `add` / `delete` → return the post-mutation `Arc<HashSetData>` via
//!   `KindedSlot::from_hashset` (clone-on-write per ADR-006 §2.7.4 /
//!   W13-hashmap-mutation precedent).
//! - `union` / `intersection` / `difference` → build a fresh
//!   `HashSetData` via the matching `from_*_keys` constructor (no
//!   Arc::make_mut on either input — both receivers borrowed read-only).
//! - `to_array` → materialize a v2 raw `TypedArray<T>` for the active
//!   element arm.
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
//! - `map` currently surfaces because its output element kind must be
//!   statically plumbed into result construction.
//! - `filter` reads the predicate's `as_bool()`; non-bool results
//!   surface as a `RuntimeError`.
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.11 / §2.7.15 + playbook
//! §1.W13-hashset-rebuild.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HashSetData, HashSetElementKind, HeapKind, HeapValue};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};
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

#[derive(Clone)]
enum SetKey {
    String(Arc<String>),
    I64(i64),
}

impl SetKey {
    fn kind_name(&self) -> &'static str {
        match self {
            SetKey::String(_) => "string",
            SetKey::I64(_) => "int",
        }
    }
}

/// Project a `KindedSlot` into a supported Set element key. This is a strict
/// discriminator dispatch, not a coercion: `Int64` stores in the int arm and
/// string carriers store in the string arm.
#[inline]
fn set_key_from_slot(slot: &KindedSlot) -> Result<SetKey, VMError> {
    match slot.kind {
        NativeKind::Int64 => Ok(SetKey::I64(slot.raw() as i64)),
        NativeKind::String | NativeKind::StringV2 => {
            let s = slot.as_str().ok_or_else(|| {
                type_error(format!(
                    "Set key kind {:?} could not be borrowed as string",
                    slot.kind()
                ))
            })?;
            Ok(SetKey::String(Arc::new(s.to_string())))
        }
        NativeKind::Ptr(HeapKind::String) => match slot.slot.as_heap_value() {
            HeapValue::String(s) => Ok(SetKey::String(Arc::new(s.as_str().to_string()))),
            _ => Err(type_error(
                "Set key kind=Ptr(String) but heap arm mismatched",
            )),
        },
        _ => Err(type_error(format!(
            "Set key must be an int or string (got kind {:?})",
            slot.kind()
        ))),
    }
}

#[inline]
fn ensure_accepts_key(set: &HashSetData, key: &SetKey, op: &str) -> Result<(), VMError> {
    let ok = matches!(
        (set.element_kind(), key),
        (HashSetElementKind::String, SetKey::String(_)) | (HashSetElementKind::I64, SetKey::I64(_))
    );
    if ok {
        Ok(())
    } else {
        Err(type_error(format!(
            "Set.{op}(): key kind {} does not match Set<{}>",
            key.kind_name(),
            hashset_kind_name(set.element_kind())
        )))
    }
}

#[inline]
fn hashset_kind_name(kind: HashSetElementKind) -> &'static str {
    kind.name()
}

#[inline]
fn ensure_compatible_sets(
    lhs: &HashSetData,
    rhs: &HashSetData,
    op: &str,
) -> Result<HashSetElementKind, VMError> {
    match (lhs.element_kind(), rhs.element_kind()) {
        (a, b) if a == b => Ok(a),
        (a, b) => Err(type_error(format!(
            "Set.{op}(): cannot combine Set<{}> with Set<{}>",
            hashset_kind_name(a),
            hashset_kind_name(b)
        ))),
    }
}

#[inline]
fn string_array_slot(keys: &[Arc<String>]) -> KindedSlot {
    use crate::executor::v2_handlers::v2_array_detect::{ELEM_TYPE_STRING, stamp_elem_type};
    let arr = TypedArray::<*const StringObj>::with_capacity(keys.len() as u32);
    unsafe {
        for key in keys {
            let ptr = StringObj::new(key.as_str()) as *const StringObj;
            TypedArray::<*const StringObj>::push(arr, ptr);
        }
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
    }
    KindedSlot::new(
        ValueSlot::from_raw(arr as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

#[inline]
fn i64_array_slot(keys: &[i64]) -> KindedSlot {
    use crate::executor::v2_handlers::v2_array_detect::{ELEM_TYPE_I64, stamp_elem_type};
    let arr = TypedArray::<i64>::from_slice(keys);
    unsafe {
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
    }
    KindedSlot::new(
        ValueSlot::from_raw(arr as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
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
        return Err(type_error("Set.has() requires exactly 1 argument (key)"));
    }
    let set = as_hashset(&args[0])?;
    let key = set_key_from_slot(&args[1])?;
    ensure_accepts_key(&set, &key, "has")?;
    let contains = match key {
        SetKey::String(key) => set.contains(key.as_str()),
        SetKey::I64(key) => set.contains_i64(key),
    };
    Ok(KindedSlot::from_bool(contains))
}

/// Set.len() / Set.length() -> int
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Set.len()/length() takes no arguments"));
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

/// Set.toArray() -> Array<T>
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Set.toArray() takes no arguments"));
    }
    let set = as_hashset(&args[0])?;
    match set.element_kind() {
        HashSetElementKind::String => Ok(string_array_slot(set.string_keys())),
        HashSetElementKind::I64 => Ok(i64_array_slot(set.i64_keys())),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Set.add(key) -> Set
///
/// Routes through `HashSetData::insert*` by exact element kind. The
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
        return Err(type_error("Set.add() requires exactly 1 argument (key)"));
    }
    let mut hs: Arc<HashSetData> = as_hashset(&args[0])?;
    let key = set_key_from_slot(&args[1])?;
    ensure_accepts_key(&hs, &key, "add")?;
    match key {
        SetKey::String(key) => {
            Arc::make_mut(&mut hs).insert(key).map_err(type_error)?;
        }
        SetKey::I64(key) => {
            Arc::make_mut(&mut hs).insert_i64(key).map_err(type_error)?;
        }
    }
    Ok(KindedSlot::from_hashset(hs))
}

/// Set.delete(key) -> Set
///
/// Routes through `HashSetData::remove*` by exact element kind. Returns the
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
        return Err(type_error("Set.delete() requires exactly 1 argument (key)"));
    }
    let mut hs: Arc<HashSetData> = as_hashset(&args[0])?;
    let key = set_key_from_slot(&args[1])?;
    ensure_accepts_key(&hs, &key, "delete")?;
    match key {
        SetKey::String(key) => {
            Arc::make_mut(&mut hs).remove(key.as_str());
        }
        SetKey::I64(key) => {
            Arc::make_mut(&mut hs).remove_i64(key);
        }
    }
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
    let result_kind = ensure_compatible_sets(&lhs, &rhs, "union")?;
    let result = match result_kind {
        HashSetElementKind::String => {
            let mut keys: Vec<Arc<String>> = Vec::with_capacity(lhs.len() + rhs.len());
            for k in lhs.string_keys().iter() {
                keys.push(Arc::clone(k));
            }
            for k in rhs.string_keys().iter() {
                keys.push(Arc::clone(k));
            }
            HashSetData::from_keys(keys)
        }
        HashSetElementKind::I64 => {
            let mut keys: Vec<i64> = Vec::with_capacity(lhs.len() + rhs.len());
            keys.extend(lhs.i64_keys().iter().copied());
            keys.extend(rhs.i64_keys().iter().copied());
            HashSetData::from_i64_keys(keys)
        }
    };
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
    let result_kind = ensure_compatible_sets(&lhs, &rhs, "intersection")?;
    let result = match result_kind {
        HashSetElementKind::String => {
            let (small, large) = if lhs.len() <= rhs.len() {
                (lhs.as_ref(), rhs.as_ref())
            } else {
                (rhs.as_ref(), lhs.as_ref())
            };
            let mut keys: Vec<Arc<String>> = Vec::new();
            for k in small.string_keys().iter() {
                if large.contains(k.as_str()) {
                    keys.push(Arc::clone(k));
                }
            }
            HashSetData::from_keys(keys)
        }
        HashSetElementKind::I64 => {
            let (small, large) = if lhs.len() <= rhs.len() {
                (lhs.as_ref(), rhs.as_ref())
            } else {
                (rhs.as_ref(), lhs.as_ref())
            };
            let mut keys: Vec<i64> = Vec::new();
            for &k in small.i64_keys().iter() {
                if large.contains_i64(k) {
                    keys.push(k);
                }
            }
            HashSetData::from_i64_keys(keys)
        }
    };
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
    let result_kind = ensure_compatible_sets(&lhs, &rhs, "difference")?;
    let result = match result_kind {
        HashSetElementKind::String => {
            let mut keys: Vec<Arc<String>> = Vec::new();
            for k in lhs.string_keys().iter() {
                if !rhs.contains(k.as_str()) {
                    keys.push(Arc::clone(k));
                }
            }
            HashSetData::from_keys(keys)
        }
        HashSetElementKind::I64 => {
            let mut keys: Vec<i64> = Vec::new();
            for &k in lhs.i64_keys().iter() {
                if !rhs.contains_i64(k) {
                    keys.push(k);
                }
            }
            HashSetData::from_i64_keys(keys)
        }
    };
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
    match set.element_kind() {
        HashSetElementKind::String => {
            for key in set.string_keys().iter() {
                let key_slot = KindedSlot::from_string_arc(Arc::clone(key));
                let _ = vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
            }
        }
        HashSetElementKind::I64 => {
            for &key in set.i64_keys().iter() {
                let key_slot = KindedSlot::from_int(key);
                let _ = vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
            }
        }
    }
    Ok(KindedSlot::none())
}

/// Set.map(fn(key) -> new_key) -> Set
///
/// The output element arm must come from static result typing. That plumbing
/// is not available at this method boundary yet, so this surfaces instead of
/// inferring the result arm from the first runtime callback result.
pub fn v2_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Set.map() requires exactly 1 argument (mapper)"));
    }
    let _set = as_hashset(&args[0])?;
    Err(VMError::NotImplemented(
        "Set.map(): SURFACE - result element kind must be statically plumbed into the HashSetData constructor; runtime first-result inference is forbidden".to_string(),
    ))
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
    let mut result_set = HashSetData::empty_for_element_kind(set.element_kind());
    match set.element_kind() {
        HashSetElementKind::String => {
            for key_arc in set.string_keys().iter() {
                let key_slot = KindedSlot::from_string_arc(Arc::clone(key_arc));
                let result =
                    vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
                let keep = result.as_bool().ok_or_else(|| {
                    type_error(format!(
                        "Set.filter(): predicate must return bool (got kind {:?})",
                        result.kind()
                    ))
                })?;
                if keep {
                    result_set.insert(Arc::clone(key_arc)).map_err(type_error)?;
                }
            }
        }
        HashSetElementKind::I64 => {
            for &key in set.i64_keys().iter() {
                let key_slot = KindedSlot::from_int(key);
                let result =
                    vm.call_value_immediate_nb(closure, &[key_slot], ctx.as_deref_mut())?;
                let keep = result.as_bool().ok_or_else(|| {
                    type_error(format!(
                        "Set.filter(): predicate must return bool (got kind {:?})",
                        result.kind()
                    ))
                })?;
                if keep {
                    result_set.insert_i64(key).map_err(type_error)?;
                }
            }
        }
    }
    Ok(KindedSlot::from_hashset(Arc::new(result_set)))
}
