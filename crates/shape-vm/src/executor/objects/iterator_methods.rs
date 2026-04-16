//! Iterator method handlers for the PHF method registry.
//!
//! All methods follow the MethodFn signature:
//! fn(&mut VirtualMachine, Vec<ValueWord>, Option<&mut ExecutionContext>) -> Result<ValueWord, VMError>
//!
//! Iterator methods support lazy chaining: map/filter/take/skip append transforms
//! and return a new Iterator. Terminal operations (collect/forEach/reduce/count/any/all/find)
//! consume the iterator by advancing through the source + transform chain.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapValue, IteratorState, IteratorTransform};
use shape_value::{HeapKind, VMError, ValueWord, ValueWordExt};
use std::sync::Arc;
use std::mem::ManuallyDrop;
use super::raw_helpers;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Check that a ValueWord value is callable (function, closure, module function, or native closure)
#[inline]
fn is_callable(nb: &ValueWord) -> bool {
    nb.is_function() || nb.is_module_function() || (nb.is_heap() && matches!(
        nb.heap_kind(),
        Some(HeapKind::Closure | HeapKind::HostClosure)
    ))
}

/// Create a new Iterator ValueWord by appending a transform to the existing iterator state.
fn with_transform(state: &IteratorState, transform: IteratorTransform) -> ValueWord {
    let mut new_state = state.clone();
    new_state.transforms.push(transform);
    ValueWord::from_iterator(Box::new(new_state))
}

/// Get the length of the source collection backing an iterator.
/// Public so that the loop module can use it for IterDone.
pub fn iter_source_len(source: &ValueWord) -> usize {
    source_len(source).unwrap_or(0)
}

/// Fetch element at `position` from the source collection.
/// Public so that the loop module can use it for IterNext.
pub fn iter_source_element_at(source: &ValueWord, position: usize) -> Option<ValueWord> {
    source_element_at(source, position)
}

fn source_len(source: &ValueWord) -> Option<usize> {
    // Handle unified arrays.
    if shape_value::tags::is_unified_heap(source.raw_bits()) {
        let kind = unsafe { shape_value::tags::unified_heap_kind(source.raw_bits()) };
        if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
            let arr = unsafe {
                shape_value::unified_array::UnifiedArray::from_heap_bits(source.raw_bits())
            };
            return Some(arr.len());
        }
        return std::option::Option::None;
    }
    match source.as_heap_ref()? {
        HeapValue::Array(arr) => Some(arr.len()),
        HeapValue::String(s) => Some(s.chars().count()),
        HeapValue::Range {
            start,
            end,
            inclusive,
        } => {
            let s = start
                .as_ref()
                .and_then(|v| v.as_number_coerce())
                .unwrap_or(0.0) as i64;
            let e = end
                .as_ref()
                .and_then(|v| v.as_number_coerce())
                .unwrap_or(0.0) as i64;
            let e = if *inclusive { e + 1 } else { e };
            Some((e - s).max(0) as usize)
        }
        HeapValue::HashMap(hm) => Some(hm.keys.len()),
        _ => std::option::Option::None,
    }
}

/// Fetch element at `position` from the source collection.
fn source_element_at(source: &ValueWord, position: usize) -> Option<ValueWord> {
    // Handle unified arrays.
    if shape_value::tags::is_unified_heap(source.raw_bits()) {
        let kind = unsafe { shape_value::tags::unified_heap_kind(source.raw_bits()) };
        if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
            let arr = unsafe {
                shape_value::unified_array::UnifiedArray::from_heap_bits(source.raw_bits())
            };
            if position < arr.len() {
                let elem_bits = *arr.get(position).unwrap();
                return Some(unsafe { ValueWord::clone_from_bits(elem_bits) });
            }
            return std::option::Option::None;
        }
        return std::option::Option::None;
    }
    match source.as_heap_ref()? {
        HeapValue::Array(arr) => arr.get(position).cloned(),
        HeapValue::String(s) => s
            .chars()
            .nth(position)
            .map(|c| ValueWord::from_string(Arc::new(c.to_string()))),
        HeapValue::Range {
            start,
            end,
            inclusive,
        } => {
            let s = start
                .as_ref()
                .and_then(|v| v.as_number_coerce())
                .unwrap_or(0.0) as i64;
            let e = end
                .as_ref()
                .and_then(|v| v.as_number_coerce())
                .unwrap_or(0.0) as i64;
            let e = if *inclusive { e + 1 } else { e };
            let count = (e - s).max(0) as usize;
            if position < count {
                Some(ValueWord::from_i64(s + position as i64))
            } else {
                std::option::Option::None
            }
        }
        HeapValue::HashMap(hm) => {
            if position < hm.keys.len() {
                let pair = vec![hm.keys[position].clone(), hm.values[position].clone()];
                Some(ValueWord::from_array(Arc::new(pair)))
            } else {
                std::option::Option::None
            }
        }
        _ => std::option::Option::None,
    }
}

/// Pull the next transformed element from an iterator.
/// Mutates `state.position` and `state.done`.
/// Returns `None` when the iterator is exhausted.
fn advance_iterator(
    vm: &mut VirtualMachine,
    state: &mut IteratorState,
    ctx: &mut Option<&mut ExecutionContext>,
) -> Result<Option<ValueWord>, VMError> {
    let src_len = source_len(&state.source).unwrap_or(0);

    'outer: loop {
        if state.done || state.position >= src_len {
            state.done = true;
            return Ok(std::option::Option::None);
        }

        let raw = match source_element_at(&state.source, state.position) {
            Some(v) => v,
            std::option::Option::None => {
                state.done = true;
                return Ok(std::option::Option::None);
            }
        };
        state.position += 1;

        // Apply transform chain (Map/Filter/FlatMap only — Skip/Take handled at collect level)
        let mut current = raw;
        for transform in &state.transforms {
            match transform {
                IteratorTransform::Map(func) => {
                    let result_bits = vm.call_value_immediate_raw(func.raw_bits(), &[current.into_raw_bits()], ctx.as_deref_mut())?;
                    current = ValueWord::from_raw_bits(result_bits);
                }
                IteratorTransform::Filter(predicate) => {
                    let result_bits = vm.call_value_immediate_raw(
                        predicate.raw_bits(),
                        &[current.raw_bits()],
                        ctx.as_deref_mut(),
                    )?;
                    let truthy = raw_helpers::is_truthy_raw(result_bits);
                    drop(ValueWord::from_raw_bits(result_bits));
                    if !truthy {
                        continue 'outer;
                    }
                }
                IteratorTransform::Take(_) | IteratorTransform::Skip(_) => {
                    // Handled at the iterator level, not per-element
                }
                IteratorTransform::FlatMap(func) => {
                    let result_bits = vm.call_value_immediate_raw(func.raw_bits(), &[current.into_raw_bits()], ctx.as_deref_mut())?;
                    current = ValueWord::from_raw_bits(result_bits);
                }
            }
        }

        return Ok(Some(current));
    }
}

/// Collect all remaining elements from an iterator into a Vec.
///
/// Skip and Take transforms are interleaved with Map/Filter in the transform chain.
/// We segment the transforms at each Skip/Take boundary and process them in stages:
/// 1. Collect elements from source, applying Map/Filter/FlatMap up to the first Skip/Take
/// 2. Apply the Skip/Take to the intermediate result
/// 3. Continue with remaining transforms on the result
fn collect_all(
    vm: &mut VirtualMachine,
    state: &mut IteratorState,
    ctx: &mut Option<&mut ExecutionContext>,
) -> Result<Vec<ValueWord>, VMError> {
    // Check if there are any skip/take transforms. If not, fast path.
    let has_skip_take = state
        .transforms
        .iter()
        .any(|t| matches!(t, IteratorTransform::Skip(_) | IteratorTransform::Take(_)));

    if !has_skip_take {
        // No skip/take — just collect all elements with map/filter applied
        let mut results = Vec::new();
        loop {
            match advance_iterator(vm, state, ctx)? {
                Some(val) => results.push(val),
                std::option::Option::None => break,
            }
        }
        return Ok(results);
    }

    // Complex case: skip/take interleaved with map/filter.
    // We need to collect raw elements from source, then apply the FULL transform
    // chain in order: map/filter/skip/take.
    //
    // Strategy: collect all source elements first (skipping Map/Filter/FlatMap in
    // advance_iterator for now), then apply the complete chain.
    //
    // Actually, the simplest correct approach: collect with map/filter applied
    // inline (as advance_iterator does), then apply skip/take in order relative
    // to other skip/take ops and their positions among map/filter.
    //
    // Since advance_iterator already handles Map/Filter/FlatMap inline but
    // skips Skip/Take, we can:
    // 1. Segment transforms into groups split at Skip/Take boundaries
    // 2. For each segment, process map/filter via advance, then apply skip/take

    // Simpler correct approach: save the transforms, create a new state without
    // skip/take for advance_iterator, collect all elements, then apply skip/take in order.

    // First, collect all elements from advance_iterator (which applies Map/Filter/FlatMap)
    let mut results = Vec::new();
    loop {
        match advance_iterator(vm, state, ctx)? {
            Some(val) => results.push(val),
            std::option::Option::None => break,
        }
    }

    // Now apply skip/take transforms in order, maintaining their relative position.
    // The key insight: Map/Filter have already been applied by advance_iterator.
    // Skip/Take were skipped by advance_iterator, so we apply them now in order.
    //
    // BUT this is only correct if all Map/Filter come BEFORE all Skip/Take,
    // or all Skip/Take come BEFORE all Map/Filter.
    // For the interleaved case (skip.filter.map), we need a different approach.
    //
    // Correct general approach: partition transforms by checking if skip/take
    // appear before any map/filter. If so, we need to apply skip/take on the
    // raw source before map/filter.

    // Find the position of the first skip/take and the first map/filter/flatmap
    let first_skip_take_pos = state
        .transforms
        .iter()
        .position(|t| matches!(t, IteratorTransform::Skip(_) | IteratorTransform::Take(_)));
    let first_map_filter_pos = state.transforms.iter().position(|t| {
        matches!(
            t,
            IteratorTransform::Map(_)
                | IteratorTransform::Filter(_)
                | IteratorTransform::FlatMap(_)
        )
    });

    // If skip/take appears BEFORE map/filter (or no map/filter), we need to
    // re-collect: apply skip/take on raw source, then map/filter.
    if let Some(st_pos) = first_skip_take_pos {
        if first_map_filter_pos.is_none() || first_map_filter_pos.unwrap() > st_pos {
            // Skip/take comes first — need to re-do: collect raw source, skip/take, then map/filter
            // Reset and re-collect from source
            let src_len = source_len(&state.source).unwrap_or(0);
            let mut raw: Vec<ValueWord> = Vec::with_capacity(src_len);
            for i in 0..src_len {
                if let Some(elem) = source_element_at(&state.source, i) {
                    raw.push(elem);
                }
            }

            // Apply all transforms in order
            for transform in &state.transforms {
                match transform {
                    IteratorTransform::Skip(n) => {
                        if *n >= raw.len() {
                            raw.clear();
                        } else {
                            raw = raw.split_off(*n);
                        }
                    }
                    IteratorTransform::Take(n) => {
                        raw.truncate(*n);
                    }
                    IteratorTransform::Filter(predicate) => {
                        let mut filtered = Vec::new();
                        for elem in raw {
                            let result_bits = vm.call_value_immediate_raw(
                                predicate.raw_bits(),
                                &[elem.raw_bits()],
                                ctx.as_deref_mut(),
                            )?;
                            let truthy = raw_helpers::is_truthy_raw(result_bits);
                            drop(ValueWord::from_raw_bits(result_bits));
                            if truthy {
                                filtered.push(elem);
                            }
                        }
                        raw = filtered;
                    }
                    IteratorTransform::Map(func) => {
                        let mut mapped = Vec::with_capacity(raw.len());
                        for elem in raw {
                            let result_bits =
                                vm.call_value_immediate_raw(func.raw_bits(), &[elem.into_raw_bits()], ctx.as_deref_mut())?;
                            mapped.push(ValueWord::from_raw_bits(result_bits));
                        }
                        raw = mapped;
                    }
                    IteratorTransform::FlatMap(func) => {
                        let mut flat = Vec::new();
                        for elem in raw {
                            let result_bits =
                                vm.call_value_immediate_raw(func.raw_bits(), &[elem.into_raw_bits()], ctx.as_deref_mut())?;
                            let result = ValueWord::from_raw_bits(result_bits);
                            if let Some(inner_view) = result.as_any_array() {
                                let inner = inner_view.to_generic();
                                flat.extend_from_slice(&inner);
                            } else {
                                flat.push(result);
                            }
                        }
                        raw = flat;
                    }
                }
            }
            return Ok(raw);
        }
    }

    // Map/filter come first (already applied by advance_iterator).
    // Apply skip/take in order on the collected results.
    for transform in &state.transforms {
        match transform {
            IteratorTransform::Skip(n) => {
                if *n >= results.len() {
                    results.clear();
                } else {
                    results = results.split_off(*n);
                }
            }
            IteratorTransform::Take(n) => {
                results.truncate(*n);
            }
            _ => {} // Already applied by advance_iterator
        }
    }

    Ok(results)
}

// ── Lazy transform methods (return new Iterator) ───────────────────────────

/// Iterator.map(fn) -> Iterator
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 (MethodFnV2) iter handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Range.iter() -> Iterator (v2 native)
pub fn v2_range_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
        let receiver = ManuallyDrop::new(ValueWord::from_raw_bits(args[0]));
    let owned = (*receiver).clone();
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: owned,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}

// ── V2 Iterator method handlers ──────────────────────────────────────────
// True v2 implementations: work directly with raw u64 bits, no legacy delegation.

/// Iterator.map(fn) -> Iterator (v2 native)
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.map() requires a function argument".to_string(),
        ));
    }
    let receiver = borrow_vw(args[0]);
    let state = receiver.as_iterator().ok_or_else(|| VMError::TypeError {
        expected: "iterator",
        got: receiver.type_name(),
    })?;
    let closure_vw = borrow_vw(args[1]);
    if !is_callable(&closure_vw) {
        return Err(VMError::RuntimeError(
            "Iterator.map() argument must be a function".to_string(),
        ));
    }
    let result = with_transform(state, IteratorTransform::Map((*closure_vw).clone()));
    Ok(result.into_raw_bits())
}

/// Iterator.filter(fn) -> Iterator (v2 native)
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.filter() requires a predicate argument".to_string(),
        ));
    }
    let receiver = borrow_vw(args[0]);
    let state = receiver.as_iterator().ok_or_else(|| VMError::TypeError {
        expected: "iterator",
        got: receiver.type_name(),
    })?;
    let closure_vw = borrow_vw(args[1]);
    if !is_callable(&closure_vw) {
        return Err(VMError::RuntimeError(
            "Iterator.filter() argument must be a function".to_string(),
        ));
    }
    let result = with_transform(state, IteratorTransform::Filter((*closure_vw).clone()));
    Ok(result.into_raw_bits())
}

/// Iterator.take(n) -> Iterator (v2 native)
pub(crate) fn handle_take(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.take() requires a number argument".to_string(),
        ));
    }
    let receiver = borrow_vw(args[0]);
    let state = receiver.as_iterator().ok_or_else(|| VMError::TypeError {
        expected: "iterator",
        got: receiver.type_name(),
    })?;
    let n_vw = borrow_vw(args[1]);
    let n = n_vw.as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("Iterator.take() argument must be a number".to_string())
    })? as usize;
    let result = with_transform(state, IteratorTransform::Take(n));
    Ok(result.into_raw_bits())
}

/// Iterator.skip(n) -> Iterator (v2 native)
pub(crate) fn handle_skip(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.skip() requires a number argument".to_string(),
        ));
    }
    let receiver = borrow_vw(args[0]);
    let state = receiver.as_iterator().ok_or_else(|| VMError::TypeError {
        expected: "iterator",
        got: receiver.type_name(),
    })?;
    let n_vw = borrow_vw(args[1]);
    let n = n_vw.as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("Iterator.skip() argument must be a number".to_string())
    })? as usize;
    let result = with_transform(state, IteratorTransform::Skip(n));
    Ok(result.into_raw_bits())
}

/// Iterator.flatMap(fn) -> Iterator (v2 native)
pub(crate) fn handle_flat_map(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.flatMap() requires a function argument".to_string(),
        ));
    }
    let receiver = borrow_vw(args[0]);
    let state = receiver.as_iterator().ok_or_else(|| VMError::TypeError {
        expected: "iterator",
        got: receiver.type_name(),
    })?;
    let closure_vw = borrow_vw(args[1]);
    if !is_callable(&closure_vw) {
        return Err(VMError::RuntimeError(
            "Iterator.flatMap() argument must be a function".to_string(),
        ));
    }
    let result = with_transform(state, IteratorTransform::FlatMap((*closure_vw).clone()));
    Ok(result.into_raw_bits())
}

/// Iterator.enumerate() -> Iterator that yields [index, value] pairs (v2 native)
pub(crate) fn handle_enumerate(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    let pairs: Vec<ValueWord> = elements
        .into_iter()
        .enumerate()
        .map(|(i, v)| ValueWord::from_array(Arc::new(vec![ValueWord::from_i64(i as i64), v])))
        .collect();
    let new_state = IteratorState {
        source: ValueWord::from_array(Arc::new(pairs)),
        position: 0,
        transforms: vec![],
        done: false,
    };
    Ok(ValueWord::from_iterator(Box::new(new_state)).into_raw_bits())
}

/// Iterator.chain(other) -> Iterator that concatenates two iterators (v2 native)
pub(crate) fn handle_chain(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.chain() requires another iterator argument".to_string(),
        ));
    }
    // Collect elements from the receiver iterator
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state1 = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    let elems1 = collect_all(vm, &mut state1, &mut ctx)?;

    // The second argument may be an iterator or an array
    let arg1_vw = borrow_vw(args[1]);
    let elems2 = if let Some(it2) = arg1_vw.as_iterator() {
        let mut state2 = it2.clone();
        collect_all(vm, &mut state2, &mut ctx)?
    } else if let Some(view) = arg1_vw.as_any_array() {
        let generic = view.to_generic();
        (*generic).clone()
    } else {
        return Err(VMError::RuntimeError(
            "Iterator.chain() argument must be an iterator or array".to_string(),
        ));
    };

    let mut combined = elems1;
    combined.extend(elems2);

    let new_state = IteratorState {
        source: ValueWord::from_array(Arc::new(combined)),
        position: 0,
        transforms: vec![],
        done: false,
    };
    Ok(ValueWord::from_iterator(Box::new(new_state)).into_raw_bits())
}

/// Iterator.collect() / Iterator.toArray() -> Array (v2 native)
pub(crate) fn handle_collect(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    let results = collect_all(vm, &mut state, &mut ctx)?;
    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}

/// Iterator.forEach(fn) -> none (v2 native)
pub(crate) fn handle_for_each(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.forEach() requires a function argument".to_string(),
        ));
    }
    let func = borrow_vw(args[1]);
    if !is_callable(&func) {
        return Err(VMError::RuntimeError(
            "Iterator.forEach() argument must be a function".to_string(),
        ));
    }
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    for elem in elements {
        let result_bits = vm.call_value_immediate_raw(args[1], &[elem.raw_bits()], ctx.as_deref_mut())?;
        drop(ValueWord::from_raw_bits(result_bits));
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// Iterator.reduce(fn, init) -> value (v2 native)
pub(crate) fn handle_reduce(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Iterator.reduce() requires a function and initial value".to_string(),
        ));
    }
    let func = borrow_vw(args[1]);
    if !is_callable(&func) {
        return Err(VMError::RuntimeError(
            "Iterator.reduce() first argument must be a function".to_string(),
        ));
    }
    let mut acc_bits = raw_helpers::clone_raw_bits(args[2]);
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    for elem in elements {
        let new_acc = vm.call_value_immediate_raw(
            args[1],
            &[acc_bits, elem.into_raw_bits()],
            ctx.as_deref_mut(),
        )?;
        drop(ValueWord::from_raw_bits(acc_bits));
        acc_bits = new_acc;
    }
    Ok(acc_bits)
}

/// Iterator.count() -> int (v2 native)
pub(crate) fn handle_count(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    Ok(ValueWord::from_i64(elements.len() as i64).into_raw_bits())
}

/// Iterator.any(fn) -> bool (v2 native, short-circuits on first truthy)
pub(crate) fn handle_any(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.any() requires a predicate function".to_string(),
        ));
    }
    let predicate = borrow_vw(args[1]);
    if !is_callable(&predicate) {
        return Err(VMError::RuntimeError(
            "Iterator.any() argument must be a function".to_string(),
        ));
    }
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    loop {
        match advance_iterator(vm, &mut state, &mut ctx)? {
            Some(val) => {
                let result_bits =
                    vm.call_value_immediate_raw(args[1], &[val.raw_bits()], ctx.as_deref_mut())?;
                let truthy = raw_helpers::is_truthy_raw(result_bits);
                drop(ValueWord::from_raw_bits(result_bits));
                if truthy {
                    return Ok(ValueWord::from_bool(true).into_raw_bits());
                }
            }
            None => break,
        }
    }
    Ok(ValueWord::from_bool(false).into_raw_bits())
}

/// Iterator.all(fn) -> bool (v2 native, short-circuits on first falsy)
pub(crate) fn handle_all(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.all() requires a predicate function".to_string(),
        ));
    }
    let predicate = borrow_vw(args[1]);
    if !is_callable(&predicate) {
        return Err(VMError::RuntimeError(
            "Iterator.all() argument must be a function".to_string(),
        ));
    }
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    loop {
        match advance_iterator(vm, &mut state, &mut ctx)? {
            Some(val) => {
                let result_bits =
                    vm.call_value_immediate_raw(args[1], &[val.raw_bits()], ctx.as_deref_mut())?;
                let truthy = raw_helpers::is_truthy_raw(result_bits);
                drop(ValueWord::from_raw_bits(result_bits));
                if !truthy {
                    return Ok(ValueWord::from_bool(false).into_raw_bits());
                }
            }
            None => break,
        }
    }
    Ok(ValueWord::from_bool(true).into_raw_bits())
}

/// Iterator.find(fn) -> value | none (v2 native, short-circuits on first match)
pub(crate) fn handle_find(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.find() requires a predicate function".to_string(),
        ));
    }
    let predicate = borrow_vw(args[1]);
    if !is_callable(&predicate) {
        return Err(VMError::RuntimeError(
            "Iterator.find() argument must be a function".to_string(),
        ));
    }
    let receiver_vw = (*borrow_vw(args[0])).clone();
    let mut state = receiver_vw
        .as_iterator()
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: receiver_vw.type_name(),
        })?
        .clone();
    loop {
        match advance_iterator(vm, &mut state, &mut ctx)? {
            Some(val) => {
                let val_bits_clone = raw_helpers::clone_raw_bits(val.raw_bits());
                let result_bits =
                    vm.call_value_immediate_raw(args[1], &[val_bits_clone], ctx.as_deref_mut())?;
                let truthy = raw_helpers::is_truthy_raw(result_bits);
                drop(ValueWord::from_raw_bits(result_bits));
                if truthy {
                    return Ok(val.into_raw_bits());
                }
            }
            None => break,
        }
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// Array.iter() -> Iterator (v2 native)
pub(crate) fn handle_array_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let receiver = (*borrow_vw(args[0])).clone();
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}

/// String.iter() -> Iterator over characters (v2 native)
pub(crate) fn handle_string_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let receiver = (*borrow_vw(args[0])).clone();
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}

/// Range.iter() -> Iterator over range values (v2 native)
pub(crate) fn handle_range_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let receiver = (*borrow_vw(args[0])).clone();
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}

/// HashMap.iter() -> Iterator over [key, value] pairs (v2 native)
pub(crate) fn handle_hashmap_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let receiver = (*borrow_vw(args[0])).clone();
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}
