//! Iterator method handlers — kinded `Arc<IteratorState>` carrier.
//!
//! ## W13-iterator-state migration (2026-05-10)
//!
//! Per ADR-006 §2.7.16 / Q17 (W13-iterator-state) the lazy-iterator
//! pipeline is rebuilt on the `HeapValue::Iterator(Arc<IteratorState>)`
//! carrier. Handlers take `args: &[KindedSlot]` per ADR-006 §2.7.10 / Q11
//! and return `Result<KindedSlot, VMError>`.
//!
//! Source-side factories (`Array.iter` / `String.iter` / `HashMap.iter` /
//! `Range.iter`) construct a fresh `IteratorState { source, transforms:
//! Vec::new(), cursor: 0 }` and wrap it into a `KindedSlot` via
//! `KindedSlot::from_iterator(Arc::new(state))`.
//!
//! Lazy transforms (`map` / `filter` / `take` / `skip` / `flatMap` /
//! `enumerate` / `chain`) recover the receiver `Arc<IteratorState>`,
//! call `IteratorState::with_transform` to append a new stage, and
//! return a fresh `KindedSlot` carrying the new state.
//!
//! Eager terminals (`collect` / `forEach` / `reduce` / `count` / `any` /
//! `all` / `find`) walk the (source, transforms, cursor) triple via the
//! local `iterate_to_vec` helper, applying each transform per element
//! and short-circuiting on early-exit semantics. Closure-callback
//! transforms invoke `vm.call_value_immediate_nb(&closure, &[elem],
//! ctx.as_deref_mut())` per ADR-006 §2.7.11 / Q12.
//!
//! See `docs/adr/006-value-and-memory-model.md` §2.7.16, the W9
//! playbook §1 recipe, and the `W13-hashmap-mutation` precedent for the
//! `Arc<T>`-receiver clone-up-front pattern that keeps the iteration
//! borrow independent of the `&mut VirtualMachine` reborrow on each
//! `call_value_immediate_nb` re-entry.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HashMapData, HeapKind, HeapValue, TypedArrayData};
use shape_value::iterator_state::{IteratorSource, IteratorState, IteratorTransform};
use shape_value::typed_buffer::TypedBuffer;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Receiver projection helpers ───────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to the inner `Arc<IteratorState>` via
/// the §2.7.6 / Q8 single-discriminator path: kind gate on
/// `Ptr(HeapKind::Iterator)`, then `slot.as_heap_value()` matched against
/// `HeapValue::Iterator(arc)`. The receiver retains its share — the caller
/// borrows through the `&Arc<IteratorState>` and never decrements.
#[inline]
fn as_iterator(slot: &KindedSlot) -> Result<&Arc<IteratorState>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Iterator)) {
        return Err(type_error(format!(
            "Iterator method receiver must be an Iterator (got kind {:?})",
            slot.kind
        )));
    }
    match slot.slot.as_heap_value() {
        HeapValue::Iterator(arc) => Ok(arc),
        other => Err(type_error(format!(
            "Iterator receiver kind says Iterator but heap arm is {:?}",
            other.kind()
        ))),
    }
}

/// Reconstruct + clone share + restore — yields an owning clone whose
/// lifetime is independent of the slot's borrow, so it can outlive a
/// `&mut VirtualMachine` re-entry. Mirrors the pattern in
/// `array_transform::handle_map_v2` for typed-array receivers.
#[inline]
fn clone_typed_array_arc(slot: &KindedSlot) -> Result<Arc<TypedArrayData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::TypedArray)) {
        return Err(type_error(format!(
            "iter: expected Array receiver, got kind {:?}",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("iter: Array receiver slot bits null"));
    }
    // SAFETY: per the construction-side contract,
    // `NativeKind::Ptr(HeapKind::TypedArray)` slot bits are
    // `Arc::into_raw(Arc<TypedArrayData>)` and the slot owns one
    // strong-count share. Reconstruct, clone (bumping the share), then
    // restore the slot's original share via `Arc::into_raw`.
    let arc = unsafe { Arc::<TypedArrayData>::from_raw(bits as *const TypedArrayData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

#[inline]
fn clone_string_arc(slot: &KindedSlot) -> Result<Arc<String>, VMError> {
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("iter: String receiver slot bits null"));
    }
    match slot.kind {
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            // SAFETY: per the construction-side contract, both string
            // kinds store `Arc::into_raw::<String>` and the carrier
            // owns one strong-count share. Reconstruct, clone, restore.
            let arc = unsafe { Arc::<String>::from_raw(bits as *const String) };
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            Ok(cloned)
        }
        other => Err(type_error(format!(
            "iter: expected String receiver, got kind {:?}",
            other
        ))),
    }
}

#[inline]
fn clone_hashmap_arc(slot: &KindedSlot) -> Result<Arc<HashMapData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::HashMap)) {
        return Err(type_error(format!(
            "iter: expected HashMap receiver, got kind {:?}",
            slot.kind
        )));
    }
    match slot.slot.as_heap_value() {
        HeapValue::HashMap(arc) => Ok(Arc::clone(arc)),
        _ => Err(type_error("iter: HashMap kind says HashMap but heap arm mismatched")),
    }
}

/// Project a callback `KindedSlot` into the canonical
/// `Arc<HeapValue>` carrier the iterator-state stash uses for
/// transforms (`Map` / `Filter` / `FlatMap`).
///
/// Per §2.7.11 / Q12 closure-bearing slots carry
/// `Arc::into_raw(Arc<HeapValue>) as u64` directly (the `HeapKind::Closure`
/// arm of `clone_with_kind` / `drop_with_kind`). Recovery re-clones one
/// share so the carrier owns the stash's reference.
#[inline]
fn closure_to_heap_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Closure)) {
        return Err(type_error(format!(
            "iter: closure argument required, got kind {:?}",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("iter: closure slot bits null"));
    }
    // SAFETY: closure slot bits are `Arc::into_raw(Arc<HeapValue>)` and
    // the carrier owns one strong-count share. Bump, reconstruct.
    unsafe {
        Arc::increment_strong_count(bits as *const HeapValue);
        Ok(Arc::from_raw(bits as *const HeapValue))
    }
}

/// Materialise a stored closure `Arc<HeapValue>` back into a
/// `KindedSlot { kind: Ptr(HeapKind::Closure) }` carrier suitable for
/// `vm.call_value_immediate_nb`. Bumps the share so the resulting carrier
/// owns one independent strong-count.
#[inline]
fn closure_arc_to_kinded_slot(closure: &Arc<HeapValue>) -> KindedSlot {
    let bits = Arc::into_raw(Arc::clone(closure)) as u64;
    KindedSlot::new(
        shape_value::ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::Closure),
    )
}

// ── Element extraction (per IteratorSource variant) ───────────────────────

/// Per-variant element count for `TypedArrayData`. Local copy to avoid a
/// cross-module dependency on `array_transform::typed_array_len`.
#[inline]
fn typed_array_len(arr: &TypedArrayData) -> usize {
    match arr {
        TypedArrayData::I64(b) => b.data.len(),
        TypedArrayData::F64(b) => b.data.len(),
        TypedArrayData::Bool(b) => b.data.len(),
        TypedArrayData::I8(b) => b.data.len(),
        TypedArrayData::I16(b) => b.data.len(),
        TypedArrayData::I32(b) => b.data.len(),
        TypedArrayData::U8(b) => b.data.len(),
        TypedArrayData::U16(b) => b.data.len(),
        TypedArrayData::U32(b) => b.data.len(),
        TypedArrayData::U64(b) => b.data.len(),
        TypedArrayData::F32(b) => b.data.len(),
        TypedArrayData::String(b) => b.data.len(),
        TypedArrayData::HeapValue(b) => b.data.len(),
        TypedArrayData::Matrix(m) => m.data.len(),
        TypedArrayData::FloatSlice { len, .. } => *len as usize,
        // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A specialized arms.
        // No construction sites on this branch — surface-and-stop until commit 3.
        TypedArrayData::Decimal(_)
        | TypedArrayData::BigInt(_)
        | TypedArrayData::DateTime(_)
        | TypedArrayData::Timespan(_)
        | TypedArrayData::Duration(_)
        | TypedArrayData::Instant(_)
        | TypedArrayData::Char(_)
        | TypedArrayData::TypedObject(_)
        | TypedArrayData::TraitObject(_) => unreachable!(
            "TypedArrayData specialized variant reached in W17-typed-carrier-bundle-A commit 1/4: no construction sites yet (ADR-006 §2.7.24 Q25.A)"
        ),
    }
}

/// Read element `idx` from a typed array as a `KindedSlot`. Mirrors
/// `array_transform::element_kinded` (kept module-local to avoid a
/// cross-module helper dependency).
fn typed_array_elem_at(arr: &TypedArrayData, idx: usize) -> Result<KindedSlot, VMError> {
    match arr {
        TypedArrayData::I64(buf) => Ok(KindedSlot::from_int(buf.data[idx])),
        TypedArrayData::F64(buf) => Ok(KindedSlot::from_number(buf.data.as_slice()[idx])),
        TypedArrayData::Bool(buf) => Ok(KindedSlot::from_bool(buf.data[idx] != 0)),
        TypedArrayData::I8(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::I16(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::I32(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U8(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U16(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U32(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U64(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::F32(buf) => Ok(KindedSlot::from_number(buf.data[idx] as f64)),
        TypedArrayData::String(buf) => {
            Ok(KindedSlot::from_string_arc(Arc::clone(&buf.data[idx])))
        }
        TypedArrayData::FloatSlice { parent, offset, len: _ } => {
            let off = *offset as usize;
            Ok(KindedSlot::from_number(parent.data.as_slice()[off + idx]))
        }
        TypedArrayData::Matrix(m) => Ok(KindedSlot::from_number(m.data.as_slice()[idx])),
        TypedArrayData::HeapValue(_) => Err(VMError::NotImplemented(
            "iter over Array<heap> — SURFACE: per-element NativeKind needs \
             to be sourced from a per-element parallel-kind track that \
             does not yet exist on `TypedArrayData::HeapValue` (ADR-006 \
             §2.7.4)."
                .into(),
        )),
        // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A specialized arms.
        // No construction sites on this branch — surface-and-stop until commit 3.
        TypedArrayData::Decimal(_)
        | TypedArrayData::BigInt(_)
        | TypedArrayData::DateTime(_)
        | TypedArrayData::Timespan(_)
        | TypedArrayData::Duration(_)
        | TypedArrayData::Instant(_)
        | TypedArrayData::Char(_)
        | TypedArrayData::TypedObject(_)
        | TypedArrayData::TraitObject(_) => unreachable!(
            "TypedArrayData specialized variant reached in W17-typed-carrier-bundle-A commit 1/4: no construction sites yet (ADR-006 §2.7.24 Q25.A)"
        ),
    }
}

/// Read codepoint `idx` from a string source as a single-character
/// `String` `KindedSlot` (matches the user-visible `for c in s` shape
/// where each yielded value is itself a one-character string).
fn string_elem_at(s: &str, idx: usize) -> KindedSlot {
    let ch = s.chars().nth(idx).expect("string_elem_at: idx within len()");
    let mut buf = String::with_capacity(ch.len_utf8());
    buf.push(ch);
    KindedSlot::from_string_arc(Arc::new(buf))
}

/// Read range-element `idx` (0-based) from a (start, end, step) triple as
/// an Int64-kinded slot. Caller has already checked `idx < len`.
#[inline]
fn range_elem_at(start: i64, _end: i64, step: i64, idx: usize) -> KindedSlot {
    KindedSlot::from_int(start + (idx as i64) * step)
}

/// Read entry `idx` from a HashMap source as a 2-element
/// `[key, value]` inner array (mirrors `HashMap.entries()`).
fn hashmap_elem_at(m: &HashMapData, idx: usize) -> KindedSlot {
    let key_arc = Arc::clone(&m.keys.data[idx]);
    // W17-typed-carrier-bundle-A commit 1/4: HashMapData::values is now
    // a HashMapValueBuf enum per Q25.B; use the materialising accessor.
    let value_arc = m.values.value_at(idx);
    let inner = TypedArrayData::HeapValue(Arc::new(TypedBuffer::from_vec(vec![
        Arc::new(HeapValue::String(key_arc)),
        value_arc,
    ])));
    KindedSlot::from_typed_array(Arc::new(inner))
}

/// Yield element `idx` from an `IteratorSource`. Returns the kinded
/// payload — collection variants build the per-element kinded slot from
/// their typed buffers; range yields an `Int64` carrier.
fn source_elem_at(src: &IteratorSource, idx: usize) -> Result<KindedSlot, VMError> {
    match src {
        IteratorSource::Array(arr) => typed_array_elem_at(arr, idx),
        IteratorSource::String(s) => Ok(string_elem_at(s.as_str(), idx)),
        IteratorSource::Range { start, end, step } => {
            Ok(range_elem_at(*start, *end, *step, idx))
        }
        IteratorSource::HashMap(m) => Ok(hashmap_elem_at(m, idx)),
    }
}

// ── Pipeline driver — applies transforms per element ──────────────────────

/// Result of applying the transform chain to a single source element.
enum StageResult {
    /// Element survives — keep it, advance source cursor.
    Keep(KindedSlot),
    /// Element is dropped (filter rejected, or skip-stage absorbed it).
    Drop,
    /// Pipeline-level halt: terminate before this element. Used by
    /// `take(0)` and similar hard limits.
    Stop,
    /// FlatMap expansion — the element became zero or more sub-elements.
    /// Walk these as if each were an independent yield.
    Flat(Vec<KindedSlot>),
}

/// Apply a single transform stage to a candidate element, given the
/// per-stage local state (skip-counter, take-counter, enumerate-counter).
///
/// `skip_remaining[i]` and `take_remaining[i]` track per-stage state for
/// `Skip(n)` / `Take(n)` transforms — the per-stage indices align with
/// `transforms[i]`.
#[allow(clippy::too_many_arguments)]
fn apply_stage(
    vm: &mut VirtualMachine,
    ctx: Option<&mut ExecutionContext>,
    transform: &IteratorTransform,
    elem: KindedSlot,
    skip_remaining: &mut usize,
    take_remaining: &mut Option<usize>,
    enumerate_index: &mut usize,
) -> Result<StageResult, VMError> {
    match transform {
        IteratorTransform::Map(closure_arc) => {
            let closure = closure_arc_to_kinded_slot(closure_arc);
            let result = vm.call_value_immediate_nb(&closure, &[elem], ctx)?;
            Ok(StageResult::Keep(result))
        }
        IteratorTransform::Filter(closure_arc) => {
            let closure = closure_arc_to_kinded_slot(closure_arc);
            // Need to clone the element since the predicate consumes its
            // arg's share but we want to re-yield the original on keep.
            let elem_for_pred = elem.clone();
            let result = vm.call_value_immediate_nb(&closure, &[elem_for_pred], ctx)?;
            match result.kind {
                NativeKind::Bool => {
                    if result.slot.as_bool() {
                        Ok(StageResult::Keep(elem))
                    } else {
                        Ok(StageResult::Drop)
                    }
                }
                other => Err(type_error(format!(
                    "filter: predicate must return Bool, got kind {:?}",
                    other
                ))),
            }
        }
        IteratorTransform::Take(_n) => {
            // `take_remaining` is set to `Some(n)` on the first
            // encounter — see `iterate_to_vec` initialisation.
            match take_remaining {
                Some(0) => Ok(StageResult::Stop),
                Some(remaining) => {
                    *remaining -= 1;
                    Ok(StageResult::Keep(elem))
                }
                None => unreachable!("take_remaining must be initialised before apply_stage"),
            }
        }
        IteratorTransform::Skip(_n) => {
            if *skip_remaining > 0 {
                *skip_remaining -= 1;
                Ok(StageResult::Drop)
            } else {
                Ok(StageResult::Keep(elem))
            }
        }
        IteratorTransform::FlatMap(closure_arc) => {
            let closure = closure_arc_to_kinded_slot(closure_arc);
            let result = vm.call_value_immediate_nb(&closure, &[elem], ctx)?;
            // Result must be an Array — expand to a Vec<KindedSlot>.
            match result.kind {
                NativeKind::Ptr(HeapKind::TypedArray) => {
                    let arr_arc = match result.slot.as_heap_value() {
                        HeapValue::TypedArray(a) => Arc::clone(a),
                        _ => return Err(type_error("flatMap: kind=Array but heap arm mismatched")),
                    };
                    let len = typed_array_len(&arr_arc);
                    let mut sub: Vec<KindedSlot> = Vec::with_capacity(len);
                    for i in 0..len {
                        sub.push(typed_array_elem_at(&arr_arc, i)?);
                    }
                    Ok(StageResult::Flat(sub))
                }
                other => Err(type_error(format!(
                    "flatMap: callback must return Array, got kind {:?}",
                    other
                ))),
            }
        }
        IteratorTransform::Enumerate => {
            let idx_slot = KindedSlot::from_int(*enumerate_index as i64);
            *enumerate_index += 1;
            // Wrap [idx, elem] as a 2-element TypedArrayData::HeapValue
            // (matches the HashMap.entries() shape).
            let pair = TypedArrayData::HeapValue(Arc::new(TypedBuffer::from_vec(vec![
                kinded_slot_to_heap_arc(&idx_slot)?,
                kinded_slot_to_heap_arc(&elem)?,
            ])));
            Ok(StageResult::Keep(KindedSlot::from_typed_array(Arc::new(pair))))
        }
        IteratorTransform::Chain(_) => {
            // `Chain` is handled at the iterate_to_vec driver level
            // (after self's elements are exhausted, the chained
            // iterator is walked end-to-end). The stage-level
            // application of a `Chain` transform is a no-op pass-through.
            Ok(StageResult::Keep(elem))
        }
    }
}

/// Project an element `KindedSlot` to an `Arc<HeapValue>` for
/// heterogeneous-element array storage (used by `enumerate`'s inner
/// pair construction). Mirrors `hashmap_methods::result_slot_to_heap_value_arc`.
fn kinded_slot_to_heap_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    if slot.kind.is_integer_family() {
        return Ok(Arc::new(HeapValue::BigInt(Arc::new(slot.slot.raw() as i64))));
    }
    match slot.kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => {
            let v = slot.slot.as_f64();
            let dec = rust_decimal::Decimal::from_f64_retain(v).ok_or_else(|| {
                type_error("enumerate: Float64 element cannot round-trip to Decimal")
            })?;
            Ok(Arc::new(HeapValue::Decimal(Arc::new(dec))))
        }
        NativeKind::Bool => Ok(Arc::new(HeapValue::BigInt(Arc::new(
            if slot.slot.as_bool() { 1 } else { 0 },
        )))),
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(type_error("enumerate: String slot bits null"));
            }
            // SAFETY: per the construction-side contract, both string
            // kinds store `Arc::into_raw::<String>` and the slot owns
            // one strong-count share. Bump and reconstruct.
            let arc = unsafe {
                Arc::increment_strong_count(bits as *const String);
                Arc::from_raw(bits as *const String)
            };
            Ok(Arc::new(HeapValue::String(arc)))
        }
        NativeKind::Ptr(_) => {
            let hv: &HeapValue = slot.slot.as_heap_value();
            Ok(Arc::new(hv.clone()))
        }
        // Catch-all (covers any future NativeKind additions like new
        // nullable variants). Surface rather than fabricate.
        other => Err(type_error(format!(
            "enumerate/collect: cannot wrap kind {:?} into a HeapValue arm",
            other
        ))),
    }
}

/// Walk a `(source, transforms, cursor)` triple to completion, applying
/// every transform to each source element and pushing the surviving
/// outputs into `out`. Closure-bearing transforms (Map / Filter /
/// FlatMap) invoke `vm.call_value_immediate_nb`, which is why this
/// function takes `&mut VirtualMachine` directly.
///
/// Short-circuit terminals (`any` / `all` / `find`) iterate `out` and
/// invoke their predicate after the walk — semantically equivalent to a
/// streaming early-exit and simpler under the borrow checker (the W9
/// playbook's "collect first, terminate after" pattern, mirror of
/// `array_*` v2 handlers which materialise into `Vec<KindedSlot>` and
/// then short-circuit on the materialised vec).
fn iterate_to_vec(
    vm: &mut VirtualMachine,
    mut ctx: Option<&mut ExecutionContext>,
    state: &IteratorState,
) -> Result<Vec<KindedSlot>, VMError> {
    // Per-stage scratch state. `take_remaining[i]` = `Some(n)` on first
    // encounter of `Take(n)`; `skip_remaining[i]` = `n` on first
    // encounter of `Skip(n)`. `enumerate_index[i]` = running counter for
    // `Enumerate`. Vectors are sized to the transform chain length so
    // per-stage indices align directly.
    let nstages = state.transforms.len();
    let mut skip_remaining: Vec<usize> = Vec::with_capacity(nstages);
    let mut take_remaining: Vec<Option<usize>> = Vec::with_capacity(nstages);
    let mut enumerate_index: Vec<usize> = Vec::with_capacity(nstages);
    for t in &state.transforms {
        skip_remaining.push(match t {
            IteratorTransform::Skip(n) => *n,
            _ => 0,
        });
        take_remaining.push(match t {
            IteratorTransform::Take(n) => Some(*n),
            _ => None,
        });
        enumerate_index.push(0);
    }

    let mut out: Vec<KindedSlot> = Vec::new();

    // Drive: walk source elements, then if a `Chain` transform exists,
    // walk the chained iterator's elements after self's are exhausted.
    let nelems = state.source.len();
    let start = state.cursor;
    let mut early_stop = false;
    'outer: for i in start..nelems {
        let elem = source_elem_at(&state.source, i)?;
        // `Option<KindedSlot>` so each apply_stage take-and-replace
        // step is a `take()`/assign pair the borrow checker can verify
        // (otherwise the post-loop `out.push(current)` triggers a
        // "value moved into apply_stage on prior iteration" diagnostic
        // even though Drop/Stop/Flat all break).
        let mut current: Option<KindedSlot> = Some(elem);
        let mut dropped = false;
        for (sidx, transform) in state.transforms.iter().enumerate() {
            let elem = current.take().expect("current must be Some at stage entry");
            let res = apply_stage(
                vm,
                ctx.as_deref_mut(),
                transform,
                elem,
                &mut skip_remaining[sidx],
                &mut take_remaining[sidx],
                &mut enumerate_index[sidx],
            )?;
            match res {
                StageResult::Keep(next) => current = Some(next),
                StageResult::Drop => {
                    dropped = true;
                    break;
                }
                StageResult::Stop => {
                    early_stop = true;
                    break 'outer;
                }
                StageResult::Flat(sub) => {
                    // Each sub-element runs through stages `[sidx + 1..]`
                    // independently. Recurse via a per-sub helper that
                    // shares the per-stage scratch state.
                    for s in sub {
                        apply_remaining_stages(
                            vm,
                            ctx.as_deref_mut(),
                            state,
                            s,
                            sidx + 1,
                            &mut skip_remaining,
                            &mut take_remaining,
                            &mut enumerate_index,
                            &mut out,
                            &mut early_stop,
                        )?;
                        if early_stop {
                            break 'outer;
                        }
                    }
                    dropped = true; // current path consumed by Flat expansion
                    break;
                }
            }
        }
        if !dropped {
            if let Some(c) = current {
                out.push(c);
            }
        }
    }

    // Walk chained iterators after self's source is exhausted (or
    // skipped early by `early_stop`).
    if !early_stop {
        for transform in &state.transforms {
            if let IteratorTransform::Chain(other_state) = transform {
                let child_yields = iterate_to_vec(vm, ctx.as_deref_mut(), other_state)?;
                out.extend(child_yields);
            }
        }
    }
    Ok(out)
}

/// Apply stages `[start_stage..]` to a single element and push the
/// surviving result onto `out`. Used by the `FlatMap` expansion path so
/// each sub-element runs through the remaining post-flat stages.
#[allow(clippy::too_many_arguments)]
fn apply_remaining_stages(
    vm: &mut VirtualMachine,
    mut ctx: Option<&mut ExecutionContext>,
    state: &IteratorState,
    elem: KindedSlot,
    start_stage: usize,
    skip_remaining: &mut [usize],
    take_remaining: &mut [Option<usize>],
    enumerate_index: &mut [usize],
    out: &mut Vec<KindedSlot>,
    early_stop: &mut bool,
) -> Result<(), VMError> {
    let mut current: Option<KindedSlot> = Some(elem);
    for sidx in start_stage..state.transforms.len() {
        let elem = current.take().expect("apply_remaining_stages: current must be Some");
        let res = apply_stage(
            vm,
            ctx.as_deref_mut(),
            &state.transforms[sidx],
            elem,
            &mut skip_remaining[sidx],
            &mut take_remaining[sidx],
            &mut enumerate_index[sidx],
        )?;
        match res {
            StageResult::Keep(next) => current = Some(next),
            StageResult::Drop => return Ok(()),
            StageResult::Stop => {
                *early_stop = true;
                return Ok(());
            }
            StageResult::Flat(sub) => {
                for s in sub {
                    apply_remaining_stages(
                        vm,
                        ctx.as_deref_mut(),
                        state,
                        s,
                        sidx + 1,
                        skip_remaining,
                        take_remaining,
                        enumerate_index,
                        out,
                        early_stop,
                    )?;
                    if *early_stop {
                        return Ok(());
                    }
                }
                return Ok(());
            }
        }
    }
    if let Some(c) = current {
        out.push(c);
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Receiver-bound iter() factories — construct fresh IteratorState
// ═══════════════════════════════════════════════════════════════════════════

/// `Range.iter()` — historical W13-iterator-state surface that pointed
/// at the upstream `MakeRange` carrier gap. The W15-range cluster
/// (ADR-006 §2.7.23 / Q24, 2026-05-10) lands the kinded `RangeData`
/// carrier and the live `Range.iter()` body lives at
/// `range_methods::range_iter`. This entry forwards there for any
/// callers that still resolve the W13-era symbol; new code should
/// reference `range_methods::range_iter` directly.
pub fn v2_range_iter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::range_methods::range_iter(vm, args, ctx)
}

/// `Array.iter()` — wraps the receiver `Arc<TypedArrayData>` into a fresh
/// `IteratorState` over `IteratorSource::Array`.
pub(crate) fn handle_array_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("Array.iter(): missing receiver"));
    }
    let arr = clone_typed_array_arc(&args[0])?;
    let state = IteratorState::new(IteratorSource::Array(arr));
    Ok(KindedSlot::from_iterator(Arc::new(state)))
}

/// `String.iter()` — wraps the receiver `Arc<String>` into a fresh
/// `IteratorState` over `IteratorSource::String`.
pub(crate) fn handle_string_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("String.iter(): missing receiver"));
    }
    let s = clone_string_arc(&args[0])?;
    let state = IteratorState::new(IteratorSource::String(s));
    Ok(KindedSlot::from_iterator(Arc::new(state)))
}

/// Range.iter() — alternate handler binding (kept for build stability;
/// see `v2_range_iter` for the live registry entry).
pub(crate) fn handle_range_iter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    v2_range_iter(vm, args, ctx)
}

/// `HashMap.iter()` — wraps the receiver `Arc<HashMapData>` into a fresh
/// `IteratorState` over `IteratorSource::HashMap`.
pub(crate) fn handle_hashmap_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("HashMap.iter(): missing receiver"));
    }
    let m = clone_hashmap_arc(&args[0])?;
    let state = IteratorState::new(IteratorSource::HashMap(m));
    Ok(KindedSlot::from_iterator(Arc::new(state)))
}

// ═══════════════════════════════════════════════════════════════════════════
// Lazy transforms — append a stage and return a fresh IteratorState
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn append_transform(
    args: &[KindedSlot],
    op: &'static str,
    expected_arity: usize,
    transform: IteratorTransform,
) -> Result<KindedSlot, VMError> {
    if args.len() != expected_arity {
        return Err(type_error(format!(
            "Iterator.{}: expected {} arguments, got {}",
            op,
            expected_arity - 1,
            args.len() - 1
        )));
    }
    let state = as_iterator(&args[0])?;
    let new_state = state.with_transform(transform);
    Ok(KindedSlot::from_iterator(Arc::new(new_state)))
}

#[inline]
fn read_int_arg(slot: &KindedSlot, op: &'static str) -> Result<usize, VMError> {
    let bits = slot.slot.raw();
    let i = match slot.kind {
        NativeKind::Int8 | NativeKind::NullableInt8 => bits as u8 as i8 as i64,
        NativeKind::Int16 | NativeKind::NullableInt16 => bits as u16 as i16 as i64,
        NativeKind::Int32 | NativeKind::NullableInt32 => bits as u32 as i32 as i64,
        NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize => bits as i64,
        NativeKind::UInt8 | NativeKind::NullableUInt8 => (bits as u8) as i64,
        NativeKind::UInt16 | NativeKind::NullableUInt16 => (bits as u16) as i64,
        NativeKind::UInt32 | NativeKind::NullableUInt32 => (bits as u32) as i64,
        NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => bits as i64,
        other => {
            return Err(type_error(format!(
                "Iterator.{}: expected integer count, got kind {:?}",
                op, other
            )));
        }
    };
    if i < 0 {
        return Err(type_error(format!(
            "Iterator.{}: count must be non-negative, got {}",
            op, i
        )));
    }
    Ok(i as usize)
}

/// `Iterator.map(closure)` — append a Map transform.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.map: expected (iterator, closure)"));
    }
    let closure = closure_to_heap_arc(&args[1])?;
    append_transform(args, "map", 2, IteratorTransform::Map(closure))
}

/// `Iterator.filter(closure)` — append a Filter transform.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Iterator.filter: expected (iterator, predicate)",
        ));
    }
    let closure = closure_to_heap_arc(&args[1])?;
    append_transform(args, "filter", 2, IteratorTransform::Filter(closure))
}

/// `Iterator.take(n)` — append a Take transform.
pub(crate) fn handle_take(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.take: expected (iterator, count)"));
    }
    let n = read_int_arg(&args[1], "take")?;
    append_transform(args, "take", 2, IteratorTransform::Take(n))
}

/// `Iterator.skip(n)` — append a Skip transform.
pub(crate) fn handle_skip(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.skip: expected (iterator, count)"));
    }
    let n = read_int_arg(&args[1], "skip")?;
    append_transform(args, "skip", 2, IteratorTransform::Skip(n))
}

/// `Iterator.flatMap(closure)` — append a FlatMap transform.
pub(crate) fn handle_flat_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.flatMap: expected (iterator, fn)"));
    }
    let closure = closure_to_heap_arc(&args[1])?;
    append_transform(args, "flatMap", 2, IteratorTransform::FlatMap(closure))
}

/// `Iterator.enumerate()` — append an Enumerate transform.
pub(crate) fn handle_enumerate(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    append_transform(args, "enumerate", 1, IteratorTransform::Enumerate)
}

/// `Iterator.chain(other)` — append a Chain transform whose payload is
/// the other `Arc<IteratorState>`.
pub(crate) fn handle_chain(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.chain: expected (iterator, other)"));
    }
    let other = as_iterator(&args[1])?;
    let chained = Arc::clone(other);
    append_transform(args, "chain", 2, IteratorTransform::Chain(chained))
}

// ═══════════════════════════════════════════════════════════════════════════
// Eager terminals — walk the pipeline and emit a result
// ═══════════════════════════════════════════════════════════════════════════

/// Re-pack a yielded `KindedSlot` as `Arc<HeapValue>` for storage in a
/// homogeneous-element output buffer (`TypedArrayData::HeapValue`). Used
/// by `collect` since iterator pipelines yield heterogeneous-kind
/// elements.
fn yield_to_heap_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    kinded_slot_to_heap_arc(slot)
}

/// `Iterator.collect()` / `Iterator.toArray()` — materialise the
/// pipeline into an `Array<heap>` (heterogeneous element storage via
/// `TypedArrayData::HeapValue`). Each yielded element is re-packed into
/// an `Arc<HeapValue>` per `kinded_slot_to_heap_arc`; the resulting
/// outer array is wrapped as `KindedSlot::from_typed_array`.
pub(crate) fn handle_collect(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Iterator.collect: takes no arguments"));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let yields = iterate_to_vec(vm, ctx, &state)?;
    let mut out: Vec<Arc<HeapValue>> = Vec::with_capacity(yields.len());
    for slot in yields.iter() {
        out.push(yield_to_heap_arc(slot)?);
    }
    let outer = TypedArrayData::HeapValue(Arc::new(TypedBuffer::from_vec(out)));
    Ok(KindedSlot::from_typed_array(Arc::new(outer)))
}

/// `Iterator.forEach(closure)` — invokes the closure per yielded
/// element and returns null/none.
pub(crate) fn handle_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.forEach: expected (iterator, fn)"));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let closure_arc = closure_to_heap_arc(&args[1])?;
    let yields = iterate_to_vec(vm, ctx.as_deref_mut(), &state)?;
    for slot in yields {
        let closure = closure_arc_to_kinded_slot(&closure_arc);
        let _ = vm.call_value_immediate_nb(&closure, &[slot], ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}

/// `Iterator.reduce(reducer, initial)` — threads the accumulator
/// through the per-element callback. Initial value is `args[2]`.
pub(crate) fn handle_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 3 {
        return Err(type_error(
            "Iterator.reduce: expected (iterator, reducer, initial)",
        ));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let closure_arc = closure_to_heap_arc(&args[1])?;
    let mut acc: KindedSlot = args[2].clone();
    let yields = iterate_to_vec(vm, ctx.as_deref_mut(), &state)?;
    for slot in yields {
        let closure = closure_arc_to_kinded_slot(&closure_arc);
        let next = vm.call_value_immediate_nb(
            &closure,
            &[std::mem::replace(&mut acc, KindedSlot::none()), slot],
            ctx.as_deref_mut(),
        )?;
        acc = next;
    }
    Ok(acc)
}

/// `Iterator.count()` — count the elements yielded by the pipeline.
pub(crate) fn handle_count(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Iterator.count: takes no arguments"));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let yields = iterate_to_vec(vm, ctx, &state)?;
    Ok(KindedSlot::from_int(yields.len() as i64))
}

/// `Iterator.any(predicate)` — return `true` if the predicate returns
/// `true` for any yielded element.
pub(crate) fn handle_any(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.any: expected (iterator, predicate)"));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let closure_arc = closure_to_heap_arc(&args[1])?;
    let yields = iterate_to_vec(vm, ctx.as_deref_mut(), &state)?;
    for slot in yields {
        let closure = closure_arc_to_kinded_slot(&closure_arc);
        let result = vm.call_value_immediate_nb(&closure, &[slot], ctx.as_deref_mut())?;
        match result.kind {
            NativeKind::Bool => {
                if result.slot.as_bool() {
                    return Ok(KindedSlot::from_bool(true));
                }
            }
            other => {
                return Err(type_error(format!(
                    "any: predicate must return Bool, got kind {:?}",
                    other
                )));
            }
        }
    }
    Ok(KindedSlot::from_bool(false))
}

/// `Iterator.all(predicate)` — return `true` if the predicate returns
/// `true` for every yielded element (vacuously true on empty
/// iterator).
pub(crate) fn handle_all(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.all: expected (iterator, predicate)"));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let closure_arc = closure_to_heap_arc(&args[1])?;
    let yields = iterate_to_vec(vm, ctx.as_deref_mut(), &state)?;
    for slot in yields {
        let closure = closure_arc_to_kinded_slot(&closure_arc);
        let result = vm.call_value_immediate_nb(&closure, &[slot], ctx.as_deref_mut())?;
        match result.kind {
            NativeKind::Bool => {
                if !result.slot.as_bool() {
                    return Ok(KindedSlot::from_bool(false));
                }
            }
            other => {
                return Err(type_error(format!(
                    "all: predicate must return Bool, got kind {:?}",
                    other
                )));
            }
        }
    }
    Ok(KindedSlot::from_bool(true))
}

/// `Iterator.find(predicate)` — return the first yielded element where
/// the predicate returns `true`, or null/none if no element matches.
pub(crate) fn handle_find(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Iterator.find: expected (iterator, predicate)"));
    }
    let state = Arc::clone(as_iterator(&args[0])?);
    let closure_arc = closure_to_heap_arc(&args[1])?;
    let yields = iterate_to_vec(vm, ctx.as_deref_mut(), &state)?;
    for slot in yields {
        let closure = closure_arc_to_kinded_slot(&closure_arc);
        let elem_for_pred = slot.clone();
        let result = vm.call_value_immediate_nb(&closure, &[elem_for_pred], ctx.as_deref_mut())?;
        match result.kind {
            NativeKind::Bool => {
                if result.slot.as_bool() {
                    return Ok(slot);
                }
            }
            other => {
                return Err(type_error(format!(
                    "find: predicate must return Bool, got kind {:?}",
                    other
                )));
            }
        }
    }
    Ok(KindedSlot::none())
}
