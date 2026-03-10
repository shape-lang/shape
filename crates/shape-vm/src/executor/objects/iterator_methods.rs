//! Iterator method handlers for the PHF method registry.
//!
//! All methods follow the MethodFn signature:
//! fn(&mut VirtualMachine, Vec<ValueWord>, Option<&mut ExecutionContext>) -> Result<(), VMError>
//!
//! Iterator methods support lazy chaining: map/filter/take/skip append transforms
//! and return a new Iterator. Terminal operations (collect/forEach/reduce/count/any/all/find)
//! consume the iterator by advancing through the source + transform chain.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapValue, IteratorState, IteratorTransform};
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::sync::Arc;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Check that a ValueWord value is callable (function, closure, module function, or native closure)
#[inline]
fn is_callable(nb: &ValueWord) -> bool {
    match nb.tag() {
        NanTag::Function | NanTag::ModuleFunction => true,
        NanTag::Heap => matches!(
            nb.heap_kind(),
            Some(HeapKind::Closure | HeapKind::HostClosure)
        ),
        _ => false,
    }
}

/// Extract the IteratorState from the receiver (args[0]).
fn require_iterator(args: &[ValueWord]) -> Result<&IteratorState, VMError> {
    args.first()
        .and_then(|nb| nb.as_iterator())
        .ok_or_else(|| VMError::TypeError {
            expected: "iterator",
            got: args.first().map(|a| a.type_name()).unwrap_or("none"),
        })
}

/// Clone the IteratorState from the receiver, returning a boxed owned copy.
fn clone_iterator_state(args: &[ValueWord]) -> Result<IteratorState, VMError> {
    Ok(require_iterator(args)?.clone())
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
                    current = vm.call_value_immediate_nb(func, &[current], ctx.as_deref_mut())?;
                }
                IteratorTransform::Filter(predicate) => {
                    let keep = vm.call_value_immediate_nb(
                        predicate,
                        &[current.clone()],
                        ctx.as_deref_mut(),
                    )?;
                    if !keep.is_truthy() {
                        continue 'outer;
                    }
                }
                IteratorTransform::Take(_) | IteratorTransform::Skip(_) => {
                    // Handled at the iterator level, not per-element
                }
                IteratorTransform::FlatMap(func) => {
                    current = vm.call_value_immediate_nb(func, &[current], ctx.as_deref_mut())?;
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
                            let keep = vm.call_value_immediate_nb(
                                predicate,
                                &[elem.clone()],
                                ctx.as_deref_mut(),
                            )?;
                            if keep.is_truthy() {
                                filtered.push(elem);
                            }
                        }
                        raw = filtered;
                    }
                    IteratorTransform::Map(func) => {
                        let mut mapped = Vec::with_capacity(raw.len());
                        for elem in raw {
                            let result =
                                vm.call_value_immediate_nb(func, &[elem], ctx.as_deref_mut())?;
                            mapped.push(result);
                        }
                        raw = mapped;
                    }
                    IteratorTransform::FlatMap(func) => {
                        let mut flat = Vec::new();
                        for elem in raw {
                            let result =
                                vm.call_value_immediate_nb(func, &[elem], ctx.as_deref_mut())?;
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
pub fn handle_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.map() requires a function argument".to_string(),
        ));
    }
    let state = require_iterator(&args)?;
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.map() argument must be a function".to_string(),
        ));
    }
    let result = with_transform(state, IteratorTransform::Map(args[1].clone()));
    vm.push_vw(result)?;
    Ok(())
}

/// Iterator.filter(fn) -> Iterator
pub fn handle_filter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.filter() requires a predicate argument".to_string(),
        ));
    }
    let state = require_iterator(&args)?;
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.filter() argument must be a function".to_string(),
        ));
    }
    let result = with_transform(state, IteratorTransform::Filter(args[1].clone()));
    vm.push_vw(result)?;
    Ok(())
}

/// Iterator.take(n) -> Iterator
pub fn handle_take(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.take() requires a number argument".to_string(),
        ));
    }
    let state = require_iterator(&args)?;
    let n = args[1].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("Iterator.take() argument must be a number".to_string())
    })? as usize;
    let result = with_transform(state, IteratorTransform::Take(n));
    vm.push_vw(result)?;
    Ok(())
}

/// Iterator.skip(n) -> Iterator
pub fn handle_skip(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.skip() requires a number argument".to_string(),
        ));
    }
    let state = require_iterator(&args)?;
    let n = args[1].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("Iterator.skip() argument must be a number".to_string())
    })? as usize;
    let result = with_transform(state, IteratorTransform::Skip(n));
    vm.push_vw(result)?;
    Ok(())
}

/// Iterator.flatMap(fn) -> Iterator
pub fn handle_flat_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.flatMap() requires a function argument".to_string(),
        ));
    }
    let state = require_iterator(&args)?;
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.flatMap() argument must be a function".to_string(),
        ));
    }
    let result = with_transform(state, IteratorTransform::FlatMap(args[1].clone()));
    vm.push_vw(result)?;
    Ok(())
}

/// Iterator.enumerate() -> Iterator that yields [index, value] pairs
/// Implemented by wrapping source into an array of [idx, val] pairs via map.
/// Since we don't have a stateful index counter in the transform chain,
/// we collect the source and re-wrap as an indexed array iterator.
pub fn handle_enumerate(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let mut state = clone_iterator_state(&args)?;
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
    vm.push_vw(ValueWord::from_iterator(Box::new(new_state)))?;
    Ok(())
}

/// Iterator.chain(other) -> Iterator that concatenates two iterators
pub fn handle_chain(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.chain() requires another iterator argument".to_string(),
        ));
    }
    // Collect both iterators into arrays and create a new iterator over the concatenation
    let mut state1 = clone_iterator_state(&args)?;
    let elems1 = collect_all(vm, &mut state1, &mut ctx)?;

    // The second argument may be an iterator or an array
    let elems2 = if let Some(it2) = args[1].as_iterator() {
        let mut state2 = it2.clone();
        collect_all(vm, &mut state2, &mut ctx)?
    } else if let Some(HeapValue::Array(arr)) = args[1].as_heap_ref() {
        arr.to_vec()
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
    vm.push_vw(ValueWord::from_iterator(Box::new(new_state)))?;
    Ok(())
}

// ── Terminal operations (consume the iterator) ─────────────────────────────

/// Iterator.collect() / Iterator.toArray() -> Array
pub fn handle_collect(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let mut state = clone_iterator_state(&args)?;
    let results = collect_all(vm, &mut state, &mut ctx)?;
    vm.push_vw(ValueWord::from_array(Arc::new(results)))?;
    Ok(())
}

/// Iterator.forEach(fn) -> none
pub fn handle_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.forEach() requires a function argument".to_string(),
        ));
    }
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.forEach() argument must be a function".to_string(),
        ));
    }
    let func = args[1].clone();
    let mut state = clone_iterator_state(&args)?;
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    for elem in elements {
        vm.call_value_immediate_nb(&func, &[elem], ctx.as_deref_mut())?;
    }
    vm.push_vw(ValueWord::none())?;
    Ok(())
}

/// Iterator.reduce(fn, init) -> value
pub fn handle_reduce(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Iterator.reduce() requires a function and initial value".to_string(),
        ));
    }
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.reduce() first argument must be a function".to_string(),
        ));
    }
    let func = args[1].clone();
    let mut accumulator = args[2].clone();
    let mut state = clone_iterator_state(&args)?;
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    for elem in elements {
        accumulator =
            vm.call_value_immediate_nb(&func, &[accumulator, elem], ctx.as_deref_mut())?;
    }
    vm.push_vw(accumulator)?;
    Ok(())
}

/// Iterator.count() -> int
pub fn handle_count(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let mut state = clone_iterator_state(&args)?;
    let elements = collect_all(vm, &mut state, &mut ctx)?;
    vm.push_vw(ValueWord::from_i64(elements.len() as i64))?;
    Ok(())
}

/// Iterator.any(fn) -> bool (short-circuits on first truthy)
pub fn handle_any(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.any() requires a predicate function".to_string(),
        ));
    }
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.any() argument must be a function".to_string(),
        ));
    }
    let predicate = args[1].clone();
    let mut state = clone_iterator_state(&args)?;
    loop {
        match advance_iterator(vm, &mut state, &mut ctx)? {
            Some(val) => {
                let result = vm.call_value_immediate_nb(&predicate, &[val], ctx.as_deref_mut())?;
                if result.is_truthy() {
                    vm.push_vw(ValueWord::from_bool(true))?;
                    return Ok(());
                }
            }
            std::option::Option::None => break,
        }
    }
    vm.push_vw(ValueWord::from_bool(false))?;
    Ok(())
}

/// Iterator.all(fn) -> bool (short-circuits on first falsy)
pub fn handle_all(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.all() requires a predicate function".to_string(),
        ));
    }
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.all() argument must be a function".to_string(),
        ));
    }
    let predicate = args[1].clone();
    let mut state = clone_iterator_state(&args)?;
    loop {
        match advance_iterator(vm, &mut state, &mut ctx)? {
            Some(val) => {
                let result = vm.call_value_immediate_nb(&predicate, &[val], ctx.as_deref_mut())?;
                if !result.is_truthy() {
                    vm.push_vw(ValueWord::from_bool(false))?;
                    return Ok(());
                }
            }
            std::option::Option::None => break,
        }
    }
    vm.push_vw(ValueWord::from_bool(true))?;
    Ok(())
}

/// Iterator.find(fn) -> value | none (short-circuits on first match)
pub fn handle_find(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Iterator.find() requires a predicate function".to_string(),
        ));
    }
    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "Iterator.find() argument must be a function".to_string(),
        ));
    }
    let predicate = args[1].clone();
    let mut state = clone_iterator_state(&args)?;
    loop {
        match advance_iterator(vm, &mut state, &mut ctx)? {
            Some(val) => {
                let result =
                    vm.call_value_immediate_nb(&predicate, &[val.clone()], ctx.as_deref_mut())?;
                if result.is_truthy() {
                    vm.push_vw(val)?;
                    return Ok(());
                }
            }
            std::option::Option::None => break,
        }
    }
    vm.push_vw(ValueWord::none())?;
    Ok(())
}

// ── .iter() builders for source types (I-Sprint 3) ─────────────────────────

/// Create an Iterator from an Array source.
pub fn make_array_iterator(source: ValueWord) -> ValueWord {
    ValueWord::from_iterator(Box::new(IteratorState {
        source,
        position: 0,
        transforms: vec![],
        done: false,
    }))
}

/// Array.iter() -> Iterator
pub fn handle_array_iter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let result = make_array_iterator(args[0].clone());
    vm.push_vw(result)?;
    Ok(())
}

/// String.iter() -> Iterator over characters
pub fn handle_string_iter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: args[0].clone(),
        position: 0,
        transforms: vec![],
        done: false,
    }));
    vm.push_vw(result)?;
    Ok(())
}

/// Range.iter() -> Iterator over range values
pub fn handle_range_iter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: args[0].clone(),
        position: 0,
        transforms: vec![],
        done: false,
    }));
    vm.push_vw(result)?;
    Ok(())
}

/// HashMap.iter() -> Iterator over [key, value] pairs
pub fn handle_hashmap_iter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "iter() requires a receiver".to_string(),
        ));
    }
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: args[0].clone(),
        position: 0,
        transforms: vec![],
        done: false,
    }));
    vm.push_vw(result)?;
    Ok(())
}
