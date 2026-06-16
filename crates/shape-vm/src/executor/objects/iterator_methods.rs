//! Iterator method handlers — kinded `Arc<IteratorState>` carrier.
//!
//! ## Wave 1b SEAM B resurrection (2026-06-15)
//!
//! The lazy `IteratorState` design (ADR-006 §2.7.16 / Q17, W13) is RATIFIED.
//! This file RESURRECTS the handler bodies that were gutted when the
//! `TypedArrayData` enum was deleted at V3-S5 ckpt-1. The carrier
//! (`shape_value::iterator_state`) already exists; the Array source variant is
//! re-added over the per-T v2-raw `TypedArray<T>` flat-struct carrier
//! ([`shape_value::iterator_state::TypedArrayArc`]) — NOT `Arc<TypedArrayData>`
//! (Refusal #1, CLAUDE.md §Forbidden).
//!
//! Two tiers:
//!
//! 1. **Lazy adapters** (`map / filter / take / skip / enumerate / chain /
//!    flatMap`) clone the receiver state and push one [`IteratorTransform`]
//!    stage — no element consumption. Closures are stashed as
//!    `Arc<HeapValue>` per ADR-006 §2.7.11 / Q12.
//! 2. **Eager terminals** (`collect / reduce / count / any / all / find /
//!    forEach`) walk the `(source, transforms)` pipeline EAGERLY, invoking
//!    closures via `vm.call_value_immediate_nb` (§2.7.11 / Q12) and reusing the
//!    proven `v2_array_detect` element primitives (`read_element` /
//!    `push_element` / `allocate_empty_typed_array`). `collect()` materializes
//!    into a fresh `TypedArray<T>`.
//!
//! ## Source coverage
//!
//! - **Array** (`TypedArrayArc`): scalar + heap-element element types via
//!   `read_element` (kind-generic).
//! - **String**: per-codepoint `char` yields.
//! - **Range**: `i64` yields.
//! - **HashMap**: per-entry `[key, value]` inner-array yields couple to
//!   nested-array / heap-element construction (V3-S5 territory) and
//!   surface-and-stop cleanly at the driver's source-element read.

use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    allocate_empty_typed_array, as_v2_typed_array, push_element, read_element,
};
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, HeapValue};
use shape_value::iterator_state::{
    IteratorSource, IteratorState, IteratorTransform, TypedArrayArc,
};
use shape_value::v2::typed_array::release_v2_typed_array;
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Receiver recovery (canonical 5-arm typed-Arc pattern)
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Reconstruct + clone share + restore — yields an owning `Arc<IteratorState>`
/// clone whose lifetime is independent of the slot's borrow. The slot bits are
/// `Arc::into_raw(Arc<IteratorState>)` directly per ADR-006 §2.7.16; recovery
/// reconstructs the typed Arc in place, clones (bumping the share), and
/// restores the slot's original share via `Arc::into_raw`.
#[inline]
fn clone_iterator_arc(slot: &KindedSlot) -> Result<Arc<IteratorState>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Iterator)) {
        return Err(type_error(format!(
            "Iterator method receiver must be an Iterator (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Iterator method receiver slot bits null"));
    }
    // SAFETY: per `KindedSlot::from_iterator`, `Ptr(HeapKind::Iterator)` slot
    // bits are `Arc::into_raw(Arc<IteratorState>)` and the slot owns one
    // strong-count share. Reconstruct, clone (bumping the share), restore so
    // the slot's original share is preserved.
    let arc = unsafe { Arc::<IteratorState>::from_raw(bits as *const IteratorState) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Recover a closure carrier `Arc<HeapValue>` (one fresh share) from a
/// `Ptr(HeapKind::Closure)` slot, to stash inside an `IteratorTransform`. The
/// slot bits are `Arc::into_raw(Arc<HeapValue>)` per W7 closure-slot contract.
#[inline]
fn clone_closure_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    if slot.kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(type_error(format!(
            "Iterator transform argument must be a closure, got kind {:?}",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Iterator closure slot bits null"));
    }
    // SAFETY: closure slot bits are `Arc::into_raw(Arc<HeapValue>)` and the
    // slot owns one share. Reconstruct, clone (bump), restore.
    let arc = unsafe { Arc::<HeapValue>::from_raw(bits as *const HeapValue) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Build a fresh `Ptr(HeapKind::Closure)` slot bumping a share off a stored
/// `Arc<HeapValue>` closure carrier, ready to hand to
/// `vm.call_value_immediate_nb` per ADR-006 §2.7.11 / Q12.
#[inline]
fn closure_slot_from_arc(arc: &Arc<HeapValue>) -> KindedSlot {
    let bumped = Arc::clone(arc);
    let bits = Arc::into_raw(bumped) as u64;
    KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::Closure),
    )
}

/// Read an integer `n` argument (for `take` / `skip`), clamped to `>= 0`.
#[inline]
fn read_count_arg(op: &str, slot: &KindedSlot) -> Result<usize, VMError> {
    let n = slot.as_i64().ok_or_else(|| {
        type_error(format!(
            "Iterator.{}: argument must be an integer, got kind {:?}",
            op, slot.kind
        ))
    })?;
    Ok(if n < 0 { 0 } else { n as usize })
}

/// Wrap a freshly-built `IteratorState` into a `Ptr(HeapKind::Iterator)` slot.
#[inline]
fn wrap_iterator(state: IteratorState) -> KindedSlot {
    KindedSlot::from_iterator(Arc::new(state))
}

/// Append a transform to the receiver iterator, returning the new iterator
/// slot. Shared adapter body for the closure-free transforms.
#[inline]
fn append_transform(
    args: &[KindedSlot],
    op: &str,
    t: IteratorTransform,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error(format!("Iterator.{}: missing receiver", op)))?,
    )?;
    Ok(wrap_iterator(state.with_transform(t)))
}

// ═══════════════════════════════════════════════════════════════════════════
// Eager terminal driver
// ═══════════════════════════════════════════════════════════════════════════

/// Read element `index` from a `IteratorSource`, producing a fresh
/// `KindedSlot` (heap-element carriers carry a fresh share). Returns `None`
/// past the end. HashMap sources surface (per-entry `[key,value]`
/// inner-array materialization is V3-S5 nested-array territory).
fn source_elem_at(source: &IteratorSource, index: usize) -> Result<Option<KindedSlot>, VMError> {
    match source {
        IteratorSource::Array(arr) => {
            let view = match as_v2_typed_array(
                arr.ptr() as usize as u64,
                NativeKind::Ptr(HeapKind::TypedArray),
            ) {
                Some(v) => v,
                None => {
                    return Err(type_error(
                        "Iterator(Array): source carrier failed v2 TypedArray detection",
                    ));
                }
            };
            if index >= view.len as usize {
                return Ok(None);
            }
            match read_element(&view, index as u32) {
                Some((bits, kind)) => Ok(Some(KindedSlot::new(ValueSlot::from_raw(bits), kind))),
                None => Ok(None),
            }
        }
        IteratorSource::String(s) => match s.chars().nth(index) {
            Some(c) => Ok(Some(KindedSlot::from_char(c))),
            None => Ok(None),
        },
        IteratorSource::Range { start, end, step } => {
            if *step <= 0 {
                return Ok(None);
            }
            let v = *start + (index as i64) * *step;
            if v >= *end {
                Ok(None)
            } else {
                Ok(Some(KindedSlot::from_int(v)))
            }
        }
        IteratorSource::HashMap(_) => Err(VMError::NotImplemented(
            "Iterator(HashMap): SURFACE — per-entry `[key, value]` inner-array \
             materialization is V3-S5 nested-array / heap-element construction \
             territory (the entry yield is an `Array<Array>` whose element kind \
             couples to the heap-element TypedArray carrier). Walk the entries \
             via `HashMap.entries()` until the nested-array iterator-yield \
             lands. NO Bool-default (ADR-006 §2.7.14)."
                .to_string(),
        )),
    }
}

/// Walk the `(source, transforms)` pipeline eagerly, invoking `sink` on each
/// fully-transformed yield. `sink` returns `Ok(true)` to continue or
/// `Ok(false)` to short-circuit (used by `any` / `all` / `find`). Closures
/// stashed in transforms are invoked via `vm.call_value_immediate_nb`.
///
/// Each `KindedSlot` handed to `sink` is owned by the sink (heap-element
/// carriers carry a fresh share); the sink is responsible for consuming or
/// releasing it.
fn drive_pipeline<F>(
    vm: &mut VirtualMachine,
    source: &IteratorSource,
    transforms: &[IteratorTransform],
    mut ctx: Option<&mut ExecutionContext>,
    sink: &mut F,
) -> Result<(), VMError>
where
    F: FnMut(
        &mut VirtualMachine,
        KindedSlot,
        Option<&mut ExecutionContext>,
    ) -> Result<bool, VMError>,
{
    let len = source.len();
    let mut idx = 0usize;
    while idx < len {
        let elem = match source_elem_at(source, idx)? {
            Some(e) => e,
            None => break,
        };
        idx += 1;
        // Apply the transform chain to this single element, feeding the sink.
        if !apply_transforms(vm, transforms, elem, ctx.as_deref_mut(), sink)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Apply `transforms[0..]` to a single upstream `elem`, feeding fully-
/// transformed yields into `sink`. Returns `Ok(false)` to short-circuit the
/// whole drive. Recursive over the transform list so `take` / `skip` /
/// `enumerate` / closures compose per-element.
fn apply_transforms<F>(
    vm: &mut VirtualMachine,
    transforms: &[IteratorTransform],
    elem: KindedSlot,
    mut ctx: Option<&mut ExecutionContext>,
    sink: &mut F,
) -> Result<bool, VMError>
where
    F: FnMut(
        &mut VirtualMachine,
        KindedSlot,
        Option<&mut ExecutionContext>,
    ) -> Result<bool, VMError>,
{
    // Note: take/skip/enumerate carry per-pipeline counters; those are handled
    // by the stateful driver below (`StageState`). This per-element recursion
    // handles only the stateless stages (map/filter/flatMap/chain); take/skip/
    // enumerate are pre-bound into the stage walk via `drive_stateful`.
    if transforms.is_empty() {
        return sink(vm, elem, ctx.as_deref_mut());
    }
    let (head, rest) = transforms.split_first().unwrap();
    match head {
        IteratorTransform::Map(closure_arc) => {
            let closure = closure_slot_from_arc(closure_arc);
            let mapped = vm.call_value_immediate_nb(&closure, &[elem], ctx.as_deref_mut())?;
            apply_transforms(vm, rest, mapped, ctx, sink)
        }
        IteratorTransform::Filter(closure_arc) => {
            let closure = closure_slot_from_arc(closure_arc);
            let elem_for_pred = elem.clone();
            let pred =
                vm.call_value_immediate_nb(&closure, &[elem_for_pred], ctx.as_deref_mut())?;
            if slot_truthy(&pred) {
                apply_transforms(vm, rest, elem, ctx, sink)
            } else {
                Ok(true)
            }
        }
        IteratorTransform::FlatMap(closure_arc) => {
            let closure = closure_slot_from_arc(closure_arc);
            let inner = vm.call_value_immediate_nb(&closure, &[elem], ctx.as_deref_mut())?;
            if inner.kind != NativeKind::Ptr(HeapKind::TypedArray) {
                return Err(type_error(format!(
                    "Iterator.flatMap: transform must return an array, got kind {:?}",
                    inner.kind
                )));
            }
            let inner_view = as_v2_typed_array(inner.slot.raw(), inner.kind).ok_or_else(|| {
                type_error("Iterator.flatMap: inner array failed v2 TypedArray detection")
            })?;
            let mut cont = true;
            for j in 0..inner_view.len {
                if let Some((ib, ik)) = read_element(&inner_view, j) {
                    let inner_elem = KindedSlot::new(ValueSlot::from_raw(ib), ik);
                    if !apply_transforms(vm, rest, inner_elem, ctx.as_deref_mut(), sink)? {
                        cont = false;
                        break;
                    }
                }
            }
            unsafe { release_v2_typed_array(inner.slot.raw() as *mut u8) };
            std::mem::forget(inner);
            Ok(cont)
        }
        IteratorTransform::Flatten => {
            // Closure-free sibling of FlatMap: the element IS the inner array.
            // Mechanical clone of the FlatMap arm above MINUS the closure call.
            let inner = elem;
            if inner.kind != NativeKind::Ptr(HeapKind::TypedArray) {
                return Err(type_error(format!(
                    "Iterator.flatten: every element must be an array, got kind {:?}. \
                     flatten flattens one level.",
                    inner.kind
                )));
            }
            let inner_view = as_v2_typed_array(inner.slot.raw(), inner.kind).ok_or_else(|| {
                type_error("Iterator.flatten: inner array failed v2 TypedArray detection")
            })?;
            let mut cont = true;
            for j in 0..inner_view.len {
                if let Some((ib, ik)) = read_element(&inner_view, j) {
                    let inner_elem = KindedSlot::new(ValueSlot::from_raw(ib), ik);
                    if !apply_transforms(vm, rest, inner_elem, ctx.as_deref_mut(), sink)? {
                        cont = false;
                        break;
                    }
                }
            }
            unsafe { release_v2_typed_array(inner.slot.raw() as *mut u8) };
            std::mem::forget(inner);
            Ok(cont)
        }
        // Chain/Take/Skip/Enumerate are stateful across elements; the stateful
        // driver pre-handles them. Reaching them here is a driver-routing bug.
        IteratorTransform::Take(_)
        | IteratorTransform::Skip(_)
        | IteratorTransform::Enumerate
        | IteratorTransform::Chain(_) => Err(type_error(
            "Iterator: stateful transform reached per-element recursion (driver bug)",
        )),
    }
}

/// Test a `KindedSlot` for truthiness — mirrors `array_query::slot_truthy`.
#[inline]
fn slot_truthy(slot: &KindedSlot) -> bool {
    let bits = slot.slot.raw();
    match slot.kind {
        NativeKind::Bool => bits != 0,
        NativeKind::Float64 => f64::from_bits(bits) != 0.0,
        NativeKind::Float32 => f32::from_bits(bits as u32) != 0.0,
        NativeKind::Null => false,
        _ => bits != 0,
    }
}

/// The full eager driver: handles the stateful stages (`take` / `skip` /
/// `enumerate` / `chain`) by partitioning the transform list at the first
/// stateful stage. Everything before it is stateless (map/filter/flatMap) and
/// runs per-element; the stateful stage applies its cross-element counter;
/// the remainder recurses through `drive_stateful` again.
///
/// Implementation: we materialize the pipeline's yields into a `Vec` of owned
/// `KindedSlot`s by feeding a collecting sink, applying stateful stages by
/// re-running the driver over the intermediate vec. This keeps each stage
/// simple and correct at the cost of intermediate buffering (acceptable —
/// terminals are eager by definition per ADR-006 §2.7.16).
fn materialize_yields(
    vm: &mut VirtualMachine,
    source: &IteratorSource,
    transforms: &[IteratorTransform],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<Vec<KindedSlot>, VMError> {
    // Split at the first stateful stage.
    let split = transforms.iter().position(|t| {
        matches!(
            t,
            IteratorTransform::Take(_)
                | IteratorTransform::Skip(_)
                | IteratorTransform::Enumerate
                | IteratorTransform::Chain(_)
        )
    });

    match split {
        None => {
            // All stateless: drive once into a vec.
            let mut out: Vec<KindedSlot> = Vec::new();
            {
                let mut sink = |_vm: &mut VirtualMachine,
                                slot: KindedSlot,
                                _ctx: Option<&mut ExecutionContext>|
                 -> Result<bool, VMError> {
                    out.push(slot);
                    Ok(true)
                };
                drive_pipeline(vm, source, transforms, ctx.as_deref_mut(), &mut sink)?;
            }
            Ok(out)
        }
        Some(pos) => {
            // Stateless prefix → vec.
            let prefix = &transforms[..pos];
            let upstream = materialize_yields(vm, source, prefix, ctx.as_deref_mut())?;
            // Apply the one stateful stage to the prefix's yields.
            let stage = &transforms[pos];
            let staged = apply_stateful_stage(vm, stage, upstream, ctx.as_deref_mut())?;
            // Remaining transforms run over the staged yields via a synthetic
            // source. We re-drive by recursing with a `Materialized` source.
            let rest = &transforms[pos + 1..];
            if rest.is_empty() {
                Ok(staged)
            } else {
                drive_over_slots(vm, staged, rest, ctx.as_deref_mut())
            }
        }
    }
}

/// Apply a single stateful stage (`take` / `skip` / `enumerate` / `chain`) to
/// an already-materialized vec of upstream yields.
fn apply_stateful_stage(
    vm: &mut VirtualMachine,
    stage: &IteratorTransform,
    upstream: Vec<KindedSlot>,
    ctx: Option<&mut ExecutionContext>,
) -> Result<Vec<KindedSlot>, VMError> {
    match stage {
        IteratorTransform::Take(n) => {
            let mut up = upstream;
            if up.len() > *n {
                // Drop the tail (releases heap-element shares on drop).
                up.truncate(*n);
            }
            Ok(up)
        }
        IteratorTransform::Skip(n) => {
            let mut up = upstream;
            let drop_n = (*n).min(up.len());
            // `drain` drops the skipped prefix, releasing shares.
            up.drain(0..drop_n);
            Ok(up)
        }
        IteratorTransform::Enumerate => {
            // Each upstream element `e` at position `i` becomes a `(i, e)`
            // tuple. Per ADR-006 the tuple carrier is a `TypedObject`
            // (`closure_layout.rs:964`: `Tuple = Ptr(HeapKind::TypedObject)`),
            // NOT a heterogeneous inner array — Shape has no `Array<Any>`.
            // The `_0` / `_1` field convention + `typed_object_from_pairs`
            // schema-registration path mirror `handle_zip_v2`
            // (`array_basic.rs::handle_zip_v2`), the proven `Array<Pair<A,B>>`
            // producer. The index slot is a literal `Int64`; the element slot
            // carries its own kind from the upstream parallel-kind track.
            //
            // Refcount: `typed_object_from_pairs` takes its `(&str, KindedSlot)`
            // pairs by reference, CLONES each input slot (bumping any heap
            // refcount) and `mem::forget`s the clone into the new object — it
            // does NOT consume the slots passed in. So `index_slot` (an inline
            // `Int64` scalar — no share) and `elem` (its own upstream share)
            // both drop at the end of each iteration, releasing the share
            // `typed_object_from_pairs` cloned. The constructed pair owns one
            // fresh `TypedObject` share, transferred into `out`.
            let _ = (vm, ctx);
            let mut out: Vec<KindedSlot> = Vec::with_capacity(upstream.len());
            for (i, elem) in upstream.into_iter().enumerate() {
                let index_slot = KindedSlot::new(ValueSlot::from_int(i as i64), NativeKind::Int64);
                let pair = shape_runtime::type_schema::typed_object_from_pairs(&[
                    ("_0", index_slot),
                    ("_1", elem),
                ]);
                if pair.kind != NativeKind::Ptr(HeapKind::TypedObject) {
                    return Err(type_error(format!(
                        "Iterator.enumerate: typed_object_from_pairs returned \
                         unexpected kind {:?}",
                        pair.kind
                    )));
                }
                out.push(pair);
            }
            Ok(out)
        }
        IteratorTransform::Chain(other) => {
            // Materialize the other iterator's yields and append.
            let mut up = upstream;
            let other_yields = materialize_yields(vm, &other.source, &other.transforms, ctx)?;
            up.extend(other_yields);
            Ok(up)
        }
        // Stateless stages are never routed here (the caller splits at the
        // first stateful stage). Reaching this is a driver-routing bug.
        IteratorTransform::Map(_)
        | IteratorTransform::Filter(_)
        | IteratorTransform::FlatMap(_)
        | IteratorTransform::Flatten => {
            drop(upstream);
            Err(type_error(
                "Iterator: stateless transform routed to apply_stateful_stage (driver bug)",
            ))
        }
    }
}

/// Drive a remaining transform list over an already-materialized vec of slots,
/// treating the vec as the upstream source. Recurses through the same
/// stateful-split logic.
fn drive_over_slots(
    vm: &mut VirtualMachine,
    slots: Vec<KindedSlot>,
    transforms: &[IteratorTransform],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<Vec<KindedSlot>, VMError> {
    let split = transforms.iter().position(|t| {
        matches!(
            t,
            IteratorTransform::Take(_)
                | IteratorTransform::Skip(_)
                | IteratorTransform::Enumerate
                | IteratorTransform::Chain(_)
        )
    });
    match split {
        None => {
            let mut out: Vec<KindedSlot> = Vec::new();
            for elem in slots {
                let mut sink = |_vm: &mut VirtualMachine,
                                slot: KindedSlot,
                                _ctx: Option<&mut ExecutionContext>|
                 -> Result<bool, VMError> {
                    out.push(slot);
                    Ok(true)
                };
                apply_transforms(
                    vm,
                    transforms,
                    elem,
                    ctx.as_deref_mut(),
                    &mut out_sink_adapter(&mut sink),
                )?;
            }
            Ok(out)
        }
        Some(pos) => {
            let prefix = &transforms[..pos];
            let upstream = drive_over_slots(vm, slots, prefix, ctx.as_deref_mut())?;
            let stage = &transforms[pos];
            let staged = apply_stateful_stage(vm, stage, upstream, ctx.as_deref_mut())?;
            let rest = &transforms[pos + 1..];
            if rest.is_empty() {
                Ok(staged)
            } else {
                drive_over_slots(vm, staged, rest, ctx.as_deref_mut())
            }
        }
    }
}

/// Identity adapter so the closure type unifies in `drive_over_slots`.
#[inline]
fn out_sink_adapter<'a, F>(
    f: &'a mut F,
) -> impl FnMut(&mut VirtualMachine, KindedSlot, Option<&mut ExecutionContext>) -> Result<bool, VMError>
+ 'a
where
    F: FnMut(
        &mut VirtualMachine,
        KindedSlot,
        Option<&mut ExecutionContext>,
    ) -> Result<bool, VMError>,
{
    move |vm, slot, ctx| f(vm, slot, ctx)
}

// ═══════════════════════════════════════════════════════════════════════════
// Receiver-bound iter() factories
// ═══════════════════════════════════════════════════════════════════════════

/// `Range.iter()` — forwarder to `range_methods::range_iter`. Live registry
/// entry is `range_methods::range_iter`; this binding exists for build
/// stability and delegates.
pub fn v2_range_iter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::range_methods::range_iter(vm, args, ctx)
}

/// `Array.iter()` — wrap a v2-raw `TypedArray<T>` receiver into
/// `IteratorSource::Array`. Bumps a share off the receiver carrier so the
/// iterator keeps the source alive.
pub(crate) fn handle_array_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let recv = args
        .first()
        .ok_or_else(|| type_error("Array.iter(): missing receiver"))?;
    if recv.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(type_error(format!(
            "Array.iter(): receiver must be an array, got kind {:?}",
            recv.kind
        )));
    }
    let ptr = recv.slot.raw() as usize as *mut u8;
    if ptr.is_null() || as_v2_typed_array(recv.slot.raw(), recv.kind).is_none() {
        return Err(type_error(
            "Array.iter(): receiver failed v2 TypedArray detection",
        ));
    }
    // SAFETY: `ptr` is a live v2 carrier (detection above succeeded); retain a
    // share for the iterator's lifetime.
    let handle = unsafe { TypedArrayArc::retain_from(ptr) };
    let state = IteratorState::new(IteratorSource::Array(handle));
    Ok(wrap_iterator(state))
}

/// `String.iter()` — wrap an `Arc<String>` receiver into
/// `IteratorSource::String`.
pub(crate) fn handle_string_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let recv = args
        .first()
        .ok_or_else(|| type_error("String.iter(): missing receiver"))?;
    let s = recv
        .as_str()
        .ok_or_else(|| type_error("String.iter(): receiver must be a string"))?;
    let state = IteratorState::new(IteratorSource::String(Arc::new(s.to_string())));
    Ok(wrap_iterator(state))
}

/// `Range.iter()` — alternate binding for build stability; delegates.
pub(crate) fn handle_range_iter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::range_methods::range_iter(vm, args, ctx)
}

/// `HashMap.iter()` — construct an `IteratorSource::HashMap`. The terminal
/// driver surfaces on the per-entry `[key, value]` yield (V3-S5 nested-array
/// territory), but the lazy factory itself is sound.
pub(crate) fn handle_hashmap_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let recv = args
        .first()
        .ok_or_else(|| type_error("HashMap.iter(): missing receiver"))?;
    if recv.kind != NativeKind::Ptr(HeapKind::HashMap) {
        return Err(type_error(format!(
            "HashMap.iter(): receiver must be a HashMap, got kind {:?}",
            recv.kind
        )));
    }
    // Recover the HashMapKindedRef via the canonical single-discriminator
    // path (`as_heap_value()` → `HeapValue::HashMap`).
    let kref = match recv.slot.as_heap_value() {
        HeapValue::HashMap(kref) => kref.clone(),
        other => {
            return Err(type_error(format!(
                "HashMap.iter(): receiver HeapValue is not a HashMap ({:?})",
                other.kind()
            )));
        }
    };
    let state = IteratorState::new(IteratorSource::HashMap(kref));
    Ok(wrap_iterator(state))
}

// ═══════════════════════════════════════════════════════════════════════════
// Lazy transforms
// ═══════════════════════════════════════════════════════════════════════════

/// `Iterator.map(closure)` — append a Map transform.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.map: missing closure argument"))?,
    )?;
    append_transform(args, "map", IteratorTransform::Map(closure))
}

/// `Iterator.filter(closure)` — append a Filter transform.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.filter: missing closure argument"))?,
    )?;
    append_transform(args, "filter", IteratorTransform::Filter(closure))
}

/// `Iterator.take(n)` — append a Take transform.
pub(crate) fn handle_take(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let n = read_count_arg(
        "take",
        args.get(1)
            .ok_or_else(|| type_error("Iterator.take: missing count argument"))?,
    )?;
    append_transform(args, "take", IteratorTransform::Take(n))
}

/// `Iterator.skip(n)` — append a Skip transform.
pub(crate) fn handle_skip(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let n = read_count_arg(
        "skip",
        args.get(1)
            .ok_or_else(|| type_error("Iterator.skip: missing count argument"))?,
    )?;
    append_transform(args, "skip", IteratorTransform::Skip(n))
}

/// `Iterator.flatMap(closure)` — append a FlatMap transform.
pub(crate) fn handle_flat_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.flatMap: missing closure argument"))?,
    )?;
    append_transform(args, "flatMap", IteratorTransform::FlatMap(closure))
}

/// `Iterator.flatten()` — append a Flatten transform (closure-free FlatMap).
pub(crate) fn handle_flatten(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    append_transform(args, "flatten", IteratorTransform::Flatten)
}

/// `Iterator.enumerate()` — append an Enumerate transform.
pub(crate) fn handle_enumerate(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    append_transform(args, "enumerate", IteratorTransform::Enumerate)
}

/// `Iterator.chain(other)` — append a Chain transform. `other` must be an
/// Iterator.
pub(crate) fn handle_chain(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let other = clone_iterator_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.chain: missing other-iterator argument"))?,
    )?;
    append_transform(args, "chain", IteratorTransform::Chain(other))
}

// ═══════════════════════════════════════════════════════════════════════════
// Eager terminals
// ═══════════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════════
// Wave 1b SEAM C — positional for-loop drive
// ═══════════════════════════════════════════════════════════════════════════

/// Materialize (once, memoized) the full post-transform yield vec for the
/// bytecode for-loop positional drive, returning a shared `Arc<Vec<KindedSlot>>`.
///
/// The for-loop protocol (`compiler/loops.rs:427`) re-`Dup`s the same
/// `Arc<IteratorState>` each iteration and indexes positionally via a 0,1,2…
/// `idx` local. Transform pipelines aren't positionally indexable on the
/// source, so the whole pipeline is driven ONCE through the SEAM B
/// `materialize_yields` terminal driver (invoking `map`/`filter` closures via
/// `vm.call_value_immediate_nb`, ADR-006 §2.7.11 / Q12) and cached on the
/// `IteratorState` memo so subsequent `IterDone`/`IterNext` reads are O(1) and
/// side-effecting closures fire exactly once per element. The memo OWNS the
/// yields' heap-element shares; callers read a positional element via a
/// share-bumped `KindedSlot::clone`.
pub(crate) fn drive_for_loop_yields(
    vm: &mut VirtualMachine,
    state: &Arc<IteratorState>,
    ctx: Option<&mut ExecutionContext>,
) -> Result<Arc<Vec<KindedSlot>>, VMError> {
    if let Some(cached) = state.materialized_yields() {
        return Ok(cached);
    }
    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx)?;
    Ok(state.set_materialized(yields))
}

/// `Iterator.collect()` / `Iterator.toArray()` — materialize into a fresh
/// `TypedArray<T>`. Output element kind = the first yield's kind; a subsequent
/// kind mismatch surfaces a structured error (no coercion, no `Array<Any>`).
pub(crate) fn handle_collect(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.collect: missing receiver"))?,
    )?;
    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx)?;
    collect_into_typed_array(yields)
}

/// Materialize a vec of yields into a `TypedArray<T>` carrier.
fn collect_into_typed_array(yields: Vec<KindedSlot>) -> Result<KindedSlot, VMError> {
    use crate::executor::v2_handlers::v2_array_detect::native_kind_to_v2_elem_type;

    if yields.is_empty() {
        // Empty pipeline → no kind to establish. Stamp an empty Int64 array
        // as a well-typed neutral fallback (matches `select`'s empty contract;
        // no Bool-default, no Any).
        let ptr = allocate_empty_typed_array(
            crate::executor::v2_handlers::v2_array_detect::V2ElemType::I64,
            0,
        );
        return Ok(KindedSlot::new(
            ValueSlot::from_raw(ptr as usize as u64),
            NativeKind::Ptr(HeapKind::TypedArray),
        ));
    }

    let elem_kind = yields[0].kind;
    let elem_type = native_kind_to_v2_elem_type(elem_kind).ok_or_else(|| {
        type_error(format!(
            "Iterator.collect: yield kind {:?} has no TypedArray<T> carrier \
             monomorphization (no element-type stamp).",
            elem_kind
        ))
    })?;
    let out_ptr = allocate_empty_typed_array(elem_type, yields.len() as u32);
    let out_view = as_v2_typed_array(
        out_ptr as usize as u64,
        NativeKind::Ptr(HeapKind::TypedArray),
    )
    .ok_or_else(|| {
        // Release the allocation before surfacing.
        unsafe { release_v2_typed_array(out_ptr) };
        type_error("Iterator.collect: freshly-allocated TypedArray failed re-detection")
    })?;

    for (i, slot) in yields.into_iter().enumerate() {
        if slot.kind != elem_kind {
            unsafe { release_v2_typed_array(out_ptr) };
            return Err(type_error(format!(
                "Iterator.collect: yield kind mismatch at index {i}: expected \
                 {elem_kind:?} (established by index 0), got {:?}. Iterators \
                 require a single output element kind (CLAUDE.md \"No `any` \
                 type\" rule, no coercion).",
                slot.kind
            )));
        }
        if let Err(msg) = push_element(&out_view, slot.slot.raw(), slot.kind) {
            unsafe { release_v2_typed_array(out_ptr) };
            return Err(type_error(format!(
                "Iterator.collect: push_element failed at index {i}: {msg}"
            )));
        }
        // push_element transfers the heap-element share; forget the local.
        std::mem::forget(slot);
    }

    Ok(KindedSlot::new(
        ValueSlot::from_raw(out_ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// `Iterator.forEach(closure)` — invoke `closure(element)` for each yield;
/// returns `null`.
pub(crate) fn handle_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.forEach: missing receiver"))?,
    )?;
    let closure_arc = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.forEach: missing closure argument"))?,
    )?;

    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx.as_deref_mut())?;
    for slot in yields {
        let closure = closure_slot_from_arc(&closure_arc);
        vm.call_value_immediate_nb(&closure, &[slot], ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}

/// `Iterator.reduce(reducer, initial)` — fold each yield into the accumulator
/// via `reducer(acc, element)`. Shape's reduce takes the callback FIRST per the
/// SEAM A signature `reduce(func(acc, t) -> acc, init) -> acc`, so the runtime
/// arg order is `args[1] = reducer closure`, `args[2] = initial`.
pub(crate) fn handle_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.reduce: missing receiver"))?,
    )?;
    let reducer_arc = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.reduce: missing reducer closure argument"))?,
    )?;
    let initial = args
        .get(2)
        .ok_or_else(|| type_error("Iterator.reduce: missing initial argument"))?
        .clone();

    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx.as_deref_mut())?;
    let mut acc = initial;
    for slot in yields {
        let closure = closure_slot_from_arc(&reducer_arc);
        acc = vm.call_value_immediate_nb(&closure, &[acc, slot], ctx.as_deref_mut())?;
    }
    Ok(acc)
}

/// `Iterator.count()` — number of yields after all transforms.
pub(crate) fn handle_count(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.count: missing receiver"))?,
    )?;
    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx)?;
    Ok(KindedSlot::from_int(yields.len() as i64))
}

/// `Iterator.any(predicate)` — true if any yield satisfies the predicate.
pub(crate) fn handle_any(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.any: missing receiver"))?,
    )?;
    let pred_arc = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.any: missing predicate argument"))?,
    )?;
    let mut ctx = ctx;
    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx.as_deref_mut())?;
    let mut found = false;
    for slot in yields {
        let closure = closure_slot_from_arc(&pred_arc);
        let r = vm.call_value_immediate_nb(&closure, &[slot], ctx.as_deref_mut())?;
        if slot_truthy(&r) {
            found = true;
            break; // short-circuit
        }
    }
    Ok(KindedSlot::from_bool(found))
}

/// `Iterator.all(predicate)` — true if every yield satisfies the predicate.
pub(crate) fn handle_all(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.all: missing receiver"))?,
    )?;
    let pred_arc = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.all: missing predicate argument"))?,
    )?;
    let mut ctx = ctx;
    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx.as_deref_mut())?;
    let mut all_true = true;
    for slot in yields {
        let closure = closure_slot_from_arc(&pred_arc);
        let r = vm.call_value_immediate_nb(&closure, &[slot], ctx.as_deref_mut())?;
        if !slot_truthy(&r) {
            all_true = false;
            break; // short-circuit
        }
    }
    Ok(KindedSlot::from_bool(all_true))
}

/// `Iterator.find(predicate)` — first yield satisfying the predicate, or
/// `null` if none.
pub(crate) fn handle_find(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let state = clone_iterator_arc(
        args.first()
            .ok_or_else(|| type_error("Iterator.find: missing receiver"))?,
    )?;
    let pred_arc = clone_closure_arc(
        args.get(1)
            .ok_or_else(|| type_error("Iterator.find: missing predicate argument"))?,
    )?;
    let mut ctx = ctx;
    let yields = materialize_yields(vm, &state.source, &state.transforms, ctx.as_deref_mut())?;
    let mut found: Option<KindedSlot> = None;
    for slot in yields {
        if found.is_some() {
            // Already found — remaining slots drop here, releasing shares.
            continue;
        }
        let probe = slot.clone();
        let closure = closure_slot_from_arc(&pred_arc);
        let r = vm.call_value_immediate_nb(&closure, &[probe], ctx.as_deref_mut())?;
        if slot_truthy(&r) {
            found = Some(slot);
        }
        // else: `slot` drops here, releasing any heap-element share.
    }
    Ok(found.unwrap_or_else(KindedSlot::none))
}
