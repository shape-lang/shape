//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every,
//! any, all, single, take_while, skip_while, for_each
//!
//! ## W16.2-J.4-rest kind-generic closure-callback migration (2026-05-23)
//!
//! The read-only / scalar-result handlers (`some`, `every`, `any`, `all`,
//! `find`, `findIndex`, `forEach`) and the value-result `single` handler
//! are KIND-GENERIC: they extract the `V2TypedArrayView` from the receiver
//! `KindedSlot` (`Ptr(HeapKind::TypedArray)` carrier per r5c-2-β-CKPT-C)
//! and iterate via `v2_array_detect::read_element`, invoking the user
//! closure per element with `vm.call_value_immediate_nb` (ADR-006 §2.7.11
//! / Q12). No new `v2_array_detect` primitive needed — the closure-callback
//! ABI + the existing `read_element` per-`V2ElemType` body suffice.
//!
//! The result-builder handlers (`where`, `select`, `takeWhile`, `skipWhile`)
//! remain SURFACE — they construct a new `Ptr(HeapKind::TypedArray)`
//! result carrier whose element kind cannot be proven without inspecting
//! every closure return (varying-kind result rejection territory) AND
//! there is no `v2_array_detect::collect_*` primitive at HEAD for the
//! result-shape allocation. That's J.5 / V3-S5 ckpt-6 territory. Refusal
//! #1 binding: no fabricated builder, no Bool-default fallback.
//!
//! The value-search handlers (`indexOf`, `includes`) remain SURFACE —
//! per-kind value-equality comparison (especially for heap-element kinds
//! `StringV2` / `DecimalV2` / `TypedObject`) requires a `v2_array_detect::
//! position_of` / `contains_element` primitive that doesn't exist at
//! HEAD. J.5 territory.
//!
//! ## ADR-006 discipline preserved
//!
//! - §2.7.5 producer-side stamp: element kind stamped at allocation,
//!   never fabricated at the consumer.
//! - §2.7.10 / Q11 `MethodFnV2` ABI unchanged.
//! - §2.7.11 / Q12 closure-callback ABI unchanged.
//! - §2.7.14 forbids Bool-default for unknown element kinds — every
//!   `read_element` `None` surfaces a structured `RuntimeError`.
//! - ADR-005 §1 single-discriminator preserved.

use crate::executor::v2_handlers::v2_array_detect::{
    as_v2_typed_array, read_element, V2TypedArrayView,
};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// W16.2-J.4-rest kind-generic header-view helpers (mirror of
// array_aggregation::extract_view + slot_truthy + pair_to_slot)
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the kind-generic `V2TypedArrayView` from the receiver
/// `KindedSlot`. Receiver kind must be `Ptr(HeapKind::TypedArray)`
/// (r5c-2-β-CKPT-C single carrier).
#[inline]
fn extract_view(op: &'static str, slot: &KindedSlot) -> Result<V2TypedArrayView, VMError> {
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Array.{op}: expected v2 TypedArray receiver, got kind {:?}",
            slot.kind
        )));
    }
    as_v2_typed_array(slot.slot.raw(), slot.kind).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.{op}: receiver bits failed v2 TypedArray detection (kind {:?})",
            slot.kind
        ))
    })
}

/// Test a `KindedSlot` for truthiness — mirrors
/// `array_aggregation::slot_truthy`. Used to interpret a non-Bool closure
/// return as a predicate result.
#[inline]
fn slot_truthy(slot: &KindedSlot) -> bool {
    let bits = slot.slot.raw();
    match slot.kind {
        NativeKind::Bool => bits != 0,
        NativeKind::Float64 => f64::from_bits(bits) != 0.0,
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => bits != 0,
        NativeKind::NullableFloat64
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => bits != 0,
        NativeKind::Float32 => f32::from_bits(bits as u32) != 0.0,
        NativeKind::Char => bits != 0,
        NativeKind::StringV2 | NativeKind::DecimalV2 => bits != 0,
        NativeKind::String | NativeKind::Ptr(_) => bits != 0,
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9):
        // Null is the absence-of-value sentinel; falsy by definition.
        NativeKind::Null => false,
    }
}

/// Closure-arg validation for closure-callback handlers. The closure-arg
/// kind contract is preserved pre-iteration so the early shape-error is
/// surfaced before any element read.
#[inline]
fn require_closure(op: &str, arg: &KindedSlot) -> Result<(), VMError> {
    if arg.kind != NativeKind::Ptr(HeapKind::Closure) {
        Err(VMError::RuntimeError(format!(
            "Array.{}: predicate must be a closure, got kind {:?}",
            op, arg.kind
        )))
    } else {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// W16.2-J.4-rest J.5 SURFACE builder (preserved for result-builder /
// value-search handlers that still need a v2_array_detect primitive)
// ═══════════════════════════════════════════════════════════════════════════

/// Surface-and-stop body for the handlers that need a result-builder or
/// per-kind value-equality primitive (`where`, `select`, `takeWhile`,
/// `skipWhile`, `indexOf`, `includes`). Class-shift target: J.5 /
/// V3-S5 ckpt-6.
#[cold]
#[inline(never)]
fn j5_builder_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "Array.{op}: SURFACE — W16.2-J.5 / V3-S5 ckpt-6 territory. \
         The closure-callback / read-only handlers in this file landed at \
         W16.2-J.4-rest (2026-05-23), but this handler builds a new \
         TypedArray result whose element kind cannot be proven without a \
         `v2_array_detect::collect_*` builder primitive (varying-kind \
         result rejection territory); OR performs per-kind value-equality \
         comparison requiring a `v2_array_detect::position_of` / \
         `contains_element` primitive. Neither exists at HEAD. J.5 / \
         ckpt-6 territory. NO Bool-default fallback (ADR-006 §2.7.14). \
         Receiver kind: {kind}.",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) public handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.where(|x| ...)` — predicate-filter projection. SURFACE: builds a
/// new TypedArray result whose element kind requires a builder primitive.
/// J.5 territory.
pub(crate) fn handle_where_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2 {
        require_closure("where", &args[1])?;
    }
    Err(j5_builder_surface("where", args))
}

/// `arr.select(|x| ...)` — per-element transform projection. SURFACE:
/// builds a new TypedArray result whose element kind is the closure's
/// return kind — varying-kind result rejection requires a builder
/// primitive. J.5 territory.
pub(crate) fn handle_select_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2 {
        require_closure("select", &args[1])?;
    }
    Err(j5_builder_surface("select", args))
}

/// `arr.find(|x| ...)` — first element satisfying the predicate, or the
/// `null` sentinel if none match. Kind-generic via `read_element` +
/// closure-callback.
pub(crate) fn handle_find_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.find expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("find", &args[1])?;
    let view = extract_view("find", &args[0])?;
    let closure = &args[1];
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.find: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        // Clone the element slot for the predicate call: a found element
        // is also the return value, and the closure invocation consumes
        // its arg shares.
        let elem_for_pred = elem_slot.clone();
        let result = vm.call_value_immediate_nb(closure, &[elem_for_pred], ctx.as_deref_mut())?;
        if slot_truthy(&result) {
            return Ok(elem_slot);
        }
    }
    Ok(KindedSlot::none())
}

/// `arr.findIndex(|x| ...)` — index of the first element satisfying the
/// predicate as `Int64`, or `-1` if none match. Kind-generic via
/// `read_element` + closure-callback.
pub(crate) fn handle_find_index_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.findIndex expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("findIndex", &args[1])?;
    let view = extract_view("findIndex", &args[0])?;
    let closure = &args[1];
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.findIndex: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let result = vm.call_value_immediate_nb(closure, &[elem_slot], ctx.as_deref_mut())?;
        if slot_truthy(&result) {
            return Ok(KindedSlot::from_int(i as i64));
        }
    }
    Ok(KindedSlot::from_int(-1))
}

/// `arr.indexOf(value)` — value-search. SURFACE: per-kind equality
/// requires a `v2_array_detect::position_of` primitive. J.5 territory.
pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(j5_builder_surface("indexOf", args))
}

/// `arr.includes(value)` — value-search. SURFACE: per-kind equality
/// requires a `v2_array_detect::contains_element` primitive. J.5 territory.
pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(j5_builder_surface("includes", args))
}

/// `arr.some(|x| ...)` — true iff at least one element satisfies the
/// predicate. Kind-generic via `read_element` + closure-callback.
pub(crate) fn handle_some_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.some expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("some", &args[1])?;
    let view = extract_view("some", &args[0])?;
    let closure = &args[1];
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.some: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let result = vm.call_value_immediate_nb(closure, &[elem_slot], ctx.as_deref_mut())?;
        if slot_truthy(&result) {
            return Ok(KindedSlot::from_bool(true));
        }
    }
    Ok(KindedSlot::from_bool(false))
}

/// `arr.every(|x| ...)` — true iff every element satisfies the predicate.
/// Kind-generic via `read_element` + closure-callback.
pub(crate) fn handle_every_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.every expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("every", &args[1])?;
    let view = extract_view("every", &args[0])?;
    let closure = &args[1];
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.every: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let result = vm.call_value_immediate_nb(closure, &[elem_slot], ctx.as_deref_mut())?;
        if !slot_truthy(&result) {
            return Ok(KindedSlot::from_bool(false));
        }
    }
    Ok(KindedSlot::from_bool(true))
}

/// `arr.any(|x| ...)` — alias for `some` per `vec.shape` `Vec<T>` extend
/// block. Kind-generic via `read_element` + closure-callback.
pub(crate) fn handle_any_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_some_v2(vm, args, ctx)
}

/// `arr.all(|x| ...)` — alias for `every` per `vec.shape` `Vec<T>` extend
/// block. Kind-generic via `read_element` + closure-callback.
pub(crate) fn handle_all_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_every_v2(vm, args, ctx)
}

/// `arr.single(|x| ...)` — find the unique element matching the
/// predicate. Kind-generic via `read_element` + closure-callback.
/// Returns the matching element if exactly one matches; surfaces a
/// `RuntimeError` if zero or more than one match.
pub(crate) fn handle_single_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.single expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("single", &args[1])?;
    let view = extract_view("single", &args[0])?;
    let closure = &args[1];
    let mut found: Option<KindedSlot> = None;
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.single: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let elem_for_pred = elem_slot.clone();
        let result = vm.call_value_immediate_nb(closure, &[elem_for_pred], ctx.as_deref_mut())?;
        if slot_truthy(&result) {
            if found.is_some() {
                return Err(VMError::RuntimeError(
                    "Array.single: more than one element matched the predicate".into(),
                ));
            }
            found = Some(elem_slot);
        }
    }
    found.ok_or_else(|| {
        VMError::RuntimeError(
            "Array.single: no element matched the predicate".into(),
        )
    })
}

/// `arr.takeWhile(|x| ...)` — prefix elements while the predicate
/// returns true. SURFACE: builds a new TypedArray result whose element
/// kind needs a builder primitive. J.5 territory.
pub(crate) fn handle_take_while_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2 {
        require_closure("takeWhile", &args[1])?;
    }
    Err(j5_builder_surface("takeWhile", args))
}

/// `arr.skipWhile(|x| ...)` — drop prefix elements while the predicate
/// returns true. SURFACE: builds a new TypedArray result. J.5 territory.
pub(crate) fn handle_skip_while_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2 {
        require_closure("skipWhile", &args[1])?;
    }
    Err(j5_builder_surface("skipWhile", args))
}

/// `arr.forEach(|x| ...)` — invoke the closure per element for side
/// effects; return the unit/null sentinel. Kind-generic via
/// `read_element` + closure-callback.
pub(crate) fn handle_for_each_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.forEach expects 1 argument: (closure)".into(),
        ));
    }
    require_closure("forEach", &args[1])?;
    let view = extract_view("forEach", &args[0])?;
    let closure = &args[1];
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.forEach: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        // Drop the closure return (forEach is for side effects only).
        let _ = vm.call_value_immediate_nb(closure, &[elem_slot], ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}

