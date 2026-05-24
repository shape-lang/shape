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
//! ## R8 W4 J.5b HOF builders (2026-05-24)
//!
//! The result-builder handlers (`where`, `select`, `takeWhile`, `skipWhile`)
//! are now KIND-GENERIC two-pass scan-then-allocate builders backed by the
//! new `v2_array_detect::native_kind_to_v2_elem_type` +
//! `allocate_empty_typed_array` + existing `push_element` primitives.
//!
//! - **Filter family (`where` / `takeWhile` / `skipWhile`):** output
//!   element kind = input view's `elem_type` (no kind-mismatch territory).
//!   Single-pass with `slot_truthy(closure_result)` deciding inclusion.
//!
//! - **`select`:** output element kind = closure-return kind, established
//!   by the FIRST closure invocation per supervisor D3 (2026-05-24):
//!   structured `VMError::RuntimeError` on subsequent kind mismatch (no
//!   coercion — forbidden per CLAUDE.md §Type System Rules — and no
//!   heterogeneous `Array<Any>` — Shape has no `any` type). Empty input
//!   returns an empty array stamped with the input's elem_type as a
//!   well-typed neutral fallback (no closure runs → no kind to establish).
//!
//! The value-search handlers (`indexOf`, `includes`) landed at R8 W4
//! J.5c (2026-05-24, supervisor D2) via the generic `eq_element` +
//! `position_of` / `contains_element` primitives at
//! `v2_handlers/v2_array_detect.rs`. The handler shells extract the
//! `V2TypedArrayView` from the receiver `KindedSlot`, validate the
//! needle kind matches the array's element type (strict — no coercion;
//! kind-mismatch returns `-1` / `false` to mirror JS-family semantics),
//! and invoke the primitive. Scalar arms use bitwise compare; String /
//! Decimal deref the v2-raw `StringObj` / `DecimalObj` carriers and
//! compare content; TypedObject deep-compares schema_id + per-field
//! `NativeKind` table + per-slot equality (recursive `eq_element` on
//! the field bits per ADR-006 §2.7.16 typed-Arc dispatch-label
//! receiver-recovery). No MethodFnV2 trait dispatch (supervisor D2
//! REFUSED; per CLAUDE.md §Renames-to-refuse the "MethodFnV2 bridge"
//! pattern is forbidden).
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
    allocate_empty_typed_array, as_v2_typed_array, contains_element,
    native_kind_to_v2_elem_type, position_of, push_element, read_element, V2ElemType,
    V2TypedArrayView,
};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::v2::typed_array::release_v2_typed_array;
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
// J.5c SURFACE (value-equality handlers: `indexOf`, `includes`)
//
// R8 W4 J.5b (2026-05-24) retired this helper's `where` / `select` /
// `takeWhile` / `skipWhile` callsites by promoting them to two-pass
// scan-then-allocate HOF builders. The remaining callsites (`indexOf` /
// `includes`) still need per-kind value-equality primitives
// (`v2_array_detect::position_of` / `contains_element`) — J.5c territory.
// ═══════════════════════════════════════════════════════════════════════════

/// Surface-and-stop body for the value-search handlers that still need a
/// `v2_array_detect` per-kind value-equality primitive (`indexOf`,
/// `includes`). Class-shift target: J.5c.
#[cold]
#[inline(never)]
fn j5_builder_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "Array.{op}: SURFACE — J.5c territory. \
         Per-kind value-equality comparison (especially for heap-element \
         kinds `StringV2` / `DecimalV2` / `TypedObject`) requires a \
         `v2_array_detect::position_of` / `contains_element` primitive \
         that doesn't exist at HEAD. NO Bool-default fallback (ADR-006 \
         §2.7.14). Receiver kind: {kind}.",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// R8 W4 J.5b HOF-builder driver helpers (2026-05-24)
//
// Closure-callback ABI = ADR-006 §2.7.11 / Q12 (`vm.call_value_immediate_nb`).
// The HOF body sits here (not in `v2_array_detect`) because the closure
// invocation needs `&mut VirtualMachine`, which the leaf
// `v2_array_detect` module cannot reach.
//
// Supervisor D3 binding (2026-05-24): closure-return kind mismatch on
// `select` surfaces a structured `VMError::RuntimeError` naming the
// expected/got kinds + offending index. NO coercion (forbidden per
// CLAUDE.md §Type System Rules). NO heterogeneous `Array<Any>` carrier
// (Shape has no `any` type).
// ═══════════════════════════════════════════════════════════════════════════

/// Wrap a freshly-built v2 typed-array carrier pointer into a
/// `Ptr(HeapKind::TypedArray)`-kinded slot suitable for returning from a
/// `MethodFnV2` handler.
#[inline]
fn wrap_typed_array_result(ptr: *mut u8) -> KindedSlot {
    KindedSlot::new(
        ValueSlot::from_raw(ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

/// Filter-family driver shared by `where` / `takeWhile` / `skipWhile`.
/// Output element kind = input view's `elem_type` (filter operations
/// preserve element kind — no closure-return kind mismatch territory).
///
/// `mode` selects the semantic:
/// - `FilterMode::All`: include every element where the predicate is true
///   (the `where` semantic).
/// - `FilterMode::TakePrefix`: include the prefix until the predicate is
///   false (the `takeWhile` semantic — stops at the first false).
/// - `FilterMode::SkipPrefix`: skip the prefix while the predicate is
///   true; include everything afterwards regardless of the predicate (the
///   `skipWhile` semantic — only the prefix is gated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterMode {
    All,
    TakePrefix,
    SkipPrefix,
}

fn run_filter_builder(
    op: &'static str,
    mode: FilterMode,
    vm: &mut VirtualMachine,
    view: &V2TypedArrayView,
    closure: &KindedSlot,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // Output element kind = input elem_type (filter ops never change the
    // carrier monomorphization). Allocate with capacity = input length
    // (worst case: every element passes); the buffer may be over-
    // provisioned but is correctly stamped.
    let out_ptr = allocate_empty_typed_array(view.elem_type, view.len);
    let out_view = match as_v2_typed_array(
        out_ptr as usize as u64,
        NativeKind::Ptr(HeapKind::TypedArray),
    ) {
        Some(v) => v,
        None => {
            // Allocator failure mode (should be impossible — stamp+detect
            // is structural). Release the allocation and surface.
            unsafe { release_v2_typed_array(out_ptr) };
            return Err(VMError::RuntimeError(format!(
                "Array.{op}: failed to re-detect freshly-allocated TypedArray<{:?}>",
                view.elem_type
            )));
        }
    };

    let mut skipping = matches!(mode, FilterMode::SkipPrefix);
    for i in 0..view.len {
        let (bits, kind) = match read_element(view, i) {
            Some(pair) => pair,
            None => {
                unsafe { release_v2_typed_array(out_ptr) };
                return Err(VMError::RuntimeError(format!(
                    "Array.{op}: read_element({i}) returned None for element kind {:?}",
                    view.elem_type
                )));
            }
        };
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);

        // Two roles for the element slot: predicate arg + (on inclusion)
        // copy pushed into the output. Clone the slot bits to share the
        // refcount for heap-element carriers.
        let elem_for_pred = elem_slot.clone();
        let pred = match vm.call_value_immediate_nb(closure, &[elem_for_pred], ctx.as_deref_mut()) {
            Ok(p) => p,
            Err(e) => {
                unsafe { release_v2_typed_array(out_ptr) };
                return Err(e);
            }
        };
        let truthy = slot_truthy(&pred);

        let include = match mode {
            FilterMode::All => truthy,
            FilterMode::TakePrefix => {
                if !truthy {
                    // Stop accumulating; remaining elements dropped.
                    break;
                }
                true
            }
            FilterMode::SkipPrefix => {
                if skipping {
                    if truthy {
                        // Still skipping prefix.
                        continue;
                    }
                    skipping = false;
                }
                // Past the prefix: include unconditionally (`skipWhile`
                // only gates the prefix per stdlib semantics).
                true
            }
        };
        if include {
            let push_bits = elem_slot.slot.raw();
            let push_kind = elem_slot.kind;
            if let Err(msg) = push_element(&out_view, push_bits, push_kind) {
                unsafe { release_v2_typed_array(out_ptr) };
                return Err(VMError::RuntimeError(format!(
                    "Array.{op}: push_element failed at index {i}: {msg}"
                )));
            }
            // The element-slot ownership transferred into the output
            // array via push_element (for heap-element carriers,
            // push_element stores the caller's share — see
            // `v2_array_detect::push_element` String / Decimal /
            // TypedObject arms). Forget the local clone so the share
            // isn't double-released when `elem_slot` drops.
            std::mem::forget(elem_slot);
        }
    }

    Ok(wrap_typed_array_result(out_ptr))
}

/// `select`-driver (per-element transform projection). Output element
/// kind is the closure's return kind, established by the first
/// invocation. Subsequent kind mismatch surfaces a structured
/// `VMError::RuntimeError` per supervisor D3 (2026-05-24). Two-pass:
/// pass 1 invokes the closure on every element and collects
/// `KindedSlot` returns; pass 2 allocates the output `TypedArray<T>` for
/// the established kind and pushes each collected slot.
fn run_select_builder(
    vm: &mut VirtualMachine,
    view: &V2TypedArrayView,
    closure: &KindedSlot,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // Edge case: empty input. No closure runs → no kind to establish.
    // Return an empty array stamped with the input's elem_type as a
    // well-typed neutral fallback. (No Bool-default; no Any.)
    if view.len == 0 {
        let out_ptr = allocate_empty_typed_array(view.elem_type, 0);
        return Ok(wrap_typed_array_result(out_ptr));
    }

    // Pass 1: scan. Collect closure results; establish the output kind
    // on the first invocation; reject any subsequent kind mismatch with
    // a structured error.
    let mut results: Vec<KindedSlot> = Vec::with_capacity(view.len as usize);
    let mut established_kind: Option<NativeKind> = None;

    for i in 0..view.len {
        let (bits, kind) = read_element(view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.select: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let result = vm.call_value_immediate_nb(closure, &[elem_slot], ctx.as_deref_mut())?;

        match established_kind {
            None => {
                established_kind = Some(result.kind);
            }
            Some(expected) if expected != result.kind => {
                // Supervisor D3 structured error: name the expected /
                // got kinds + the offending index. NO coercion (forbidden
                // per CLAUDE.md §Type System Rules); NO heterogeneous
                // Array<Any> (Shape has no `any` type).
                //
                // `results` and `result` drop here, releasing their
                // accumulated shares cleanly.
                return Err(VMError::RuntimeError(format!(
                    "Array.select: closure-return kind mismatch at index {i}: \
                     expected {expected:?} (established by index 0), got {got:?}. \
                     HOF builders require a single output element kind per \
                     CLAUDE.md \"No `any` type\" rule + D3 binding (no coercion).",
                    expected = expected,
                    got = result.kind,
                    i = i,
                )));
            }
            _ => {}
        }
        results.push(result);
    }

    // Pass 2: allocate + push. The established kind must map to a
    // `V2ElemType`; otherwise the output carrier is unsupported and we
    // surface (no Bool-default, no fabricated carrier).
    let result_kind = established_kind.expect("non-empty input → established_kind is Some");
    let elem_type = native_kind_to_v2_elem_type(result_kind).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.select: closure-return kind {result_kind:?} has no `TypedArray<T>` \
             carrier monomorphization (no element-type stamp). Supported result \
             kinds: Float64/Int64/Int32/Int16/Int8/UInt32/UInt16/UInt8/Float32/Char/\
             Bool/StringV2/DecimalV2/Ptr(TypedObject). J.5d / future tuple-carrier \
             territory for other kinds."
        ))
    })?;
    let out_ptr = allocate_empty_typed_array(elem_type, view.len);
    let out_view = match as_v2_typed_array(
        out_ptr as usize as u64,
        NativeKind::Ptr(HeapKind::TypedArray),
    ) {
        Some(v) => v,
        None => {
            unsafe { release_v2_typed_array(out_ptr) };
            return Err(VMError::RuntimeError(format!(
                "Array.select: failed to re-detect freshly-allocated TypedArray<{elem_type:?}>"
            )));
        }
    };

    for (i, slot) in results.into_iter().enumerate() {
        let push_bits = slot.slot.raw();
        let push_kind = slot.kind;
        if let Err(msg) = push_element(&out_view, push_bits, push_kind) {
            unsafe { release_v2_typed_array(out_ptr) };
            return Err(VMError::RuntimeError(format!(
                "Array.select: push_element failed at index {i}: {msg}"
            )));
        }
        // Per the heap-element push contract (String / Decimal /
        // TypedObject) the caller's refcount share transfers into the
        // output array. Forget the slot so the share isn't double-
        // released when `slot` would otherwise drop.
        std::mem::forget(slot);
    }

    Ok(wrap_typed_array_result(out_ptr))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) public handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.where(|x| ...)` — predicate-filter projection. Output element
/// kind = input view's elem_type (filter ops preserve carrier mono-
/// morphization). R8 W4 J.5b (2026-05-24): kind-generic two-pass scan-
/// then-allocate via `v2_array_detect` builder primitives.
pub(crate) fn handle_where_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.where expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("where", &args[1])?;
    let view = extract_view("where", &args[0])?;
    let closure = &args[1];
    run_filter_builder("where", FilterMode::All, vm, &view, closure, ctx)
}

/// `arr.select(|x| ...)` — per-element transform projection. Output
/// element kind = closure-return kind (established by the first
/// invocation; subsequent kind mismatch surfaces a structured
/// `VMError::RuntimeError` per supervisor D3, 2026-05-24). R8 W4 J.5b:
/// kind-generic two-pass scan-then-allocate.
pub(crate) fn handle_select_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.select expects 1 argument: (transform)".into(),
        ));
    }
    require_closure("select", &args[1])?;
    let view = extract_view("select", &args[0])?;
    let closure = &args[1];
    run_select_builder(vm, &view, closure, ctx)
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

/// `arr.indexOf(value)` — value-search per the value-equality primitive
/// `v2_array_detect::position_of`. Returns the first index whose element
/// equals the needle (per `V2ElemType` equality), or `Int64(-1)` on no
/// match.
///
/// R8 W4 J.5c (2026-05-24, supervisor D2): wired to the generic
/// `eq_element` + `position_of` deep-equality primitives. Per-kind
/// equality semantics: scalar arms use bitwise compare on the
/// significant slot bits (matches `==` for integers / bool / char,
/// matches BITWISE equality for float — IEEE NaN compares equal to its
/// own bit pattern; the user can opt into IEEE semantics via
/// `find(|x| x != needle)`); String / Decimal arms deref the v2-raw
/// `StringObj` / `DecimalObj` carrier and compare content;
/// TypedObject arms deep-compare schema_id + every field via recursive
/// dispatch (see `eq_element` documentation).
///
/// Kind-mismatch between the needle and the array's element type is a
/// strict "not found" — returns `-1` without invoking the per-element
/// scan. This matches the JS-family `indexOf` semantics
/// (`[1,2,3].indexOf("1") === -1`).
pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.indexOf expects 1 argument: (value)".into(),
        ));
    }
    let view = extract_view("indexOf", &args[0])?;
    let needle = &args[1];
    if !needle_kind_matches(view.elem_type, needle.kind) {
        return Ok(KindedSlot::from_int(-1));
    }
    let needle_bits = needle.slot.raw();
    match position_of(&view, needle_bits) {
        Some(i) => Ok(KindedSlot::from_int(i as i64)),
        None => Ok(KindedSlot::from_int(-1)),
    }
}

/// `arr.includes(value)` — value-search per the value-equality primitive
/// `v2_array_detect::contains_element`. Returns `true` iff any element
/// equals the needle under the per-`V2ElemType` equality. See
/// `handle_index_of_v2` for equality-semantics documentation.
///
/// Kind-mismatch between the needle and the array's element type returns
/// `false` without invoking the scan (mirrors `indexOf` returning -1).
pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.includes expects 1 argument: (value)".into(),
        ));
    }
    let view = extract_view("includes", &args[0])?;
    let needle = &args[1];
    if !needle_kind_matches(view.elem_type, needle.kind) {
        return Ok(KindedSlot::from_bool(false));
    }
    let needle_bits = needle.slot.raw();
    Ok(KindedSlot::from_bool(contains_element(&view, needle_bits)))
}

/// Check whether `needle_kind` is a value-equality match for the array's
/// `elem_type`. Returns `false` for kind-mismatch (strict, no coercion
/// per CLAUDE.md §Type-System-Rules "NO runtime coercion"). For each
/// `V2ElemType` arm, only the canonical producer-stamped `NativeKind`
/// is accepted: Bool/F64/I64/I32/I8/U8/I16/U16/U32/F32/Char map to their
/// matching `NativeKind::*` variant; the heap-element arms (String /
/// Decimal / TypedObject) require the matching v2-raw carrier
/// (`StringV2` / `DecimalV2` / `Ptr(HeapKind::TypedObject)`).
#[inline]
fn needle_kind_matches(elem_type: V2ElemType, needle_kind: NativeKind) -> bool {
    match (elem_type, needle_kind) {
        (V2ElemType::F64, NativeKind::Float64) => true,
        (V2ElemType::I64, NativeKind::Int64) => true,
        (V2ElemType::I32, NativeKind::Int32) => true,
        (V2ElemType::Bool, NativeKind::Bool) => true,
        (V2ElemType::I8, NativeKind::Int8) => true,
        (V2ElemType::U8, NativeKind::UInt8) => true,
        (V2ElemType::I16, NativeKind::Int16) => true,
        (V2ElemType::U16, NativeKind::UInt16) => true,
        (V2ElemType::U32, NativeKind::UInt32) => true,
        (V2ElemType::F32, NativeKind::Float32) => true,
        (V2ElemType::Char, NativeKind::Char) => true,
        (V2ElemType::String, NativeKind::StringV2) => true,
        (V2ElemType::Decimal, NativeKind::DecimalV2) => true,
        (V2ElemType::TypedObject, NativeKind::Ptr(HeapKind::TypedObject)) => true,
        _ => false,
    }
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
/// returns true. Output element kind = input view's elem_type (filter op,
/// no kind-mismatch territory). R8 W4 J.5b (2026-05-24): kind-generic
/// two-pass scan-then-allocate via `v2_array_detect` builder primitives.
pub(crate) fn handle_take_while_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.takeWhile expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("takeWhile", &args[1])?;
    let view = extract_view("takeWhile", &args[0])?;
    let closure = &args[1];
    run_filter_builder("takeWhile", FilterMode::TakePrefix, vm, &view, closure, ctx)
}

/// `arr.skipWhile(|x| ...)` — drop prefix elements while the predicate
/// returns true, then include the remainder unconditionally. Output
/// element kind = input view's elem_type. R8 W4 J.5b (2026-05-24):
/// kind-generic two-pass scan-then-allocate.
pub(crate) fn handle_skip_while_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.skipWhile expects 1 argument: (predicate)".into(),
        ));
    }
    require_closure("skipWhile", &args[1])?;
    let view = extract_view("skipWhile", &args[0])?;
    let closure = &args[1];
    run_filter_builder("skipWhile", FilterMode::SkipPrefix, vm, &view, closure, ctx)
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

