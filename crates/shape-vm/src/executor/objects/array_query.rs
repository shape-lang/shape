//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every,
//! any, all, single, take_while, skip_while, for_each
//!
//! ## Wave-9 `W9-array-query` body migration (playbook §1 / ADR-006 §2.7.10 / §2.7.11)
//!
//! Wave 7 (`call_value_immediate_nb` rebuild close `06cdfce` 2026-05-09)
//! brought the kinded value-call ABI live. The closure-callback handlers
//! in this file now invoke the user closure per element via
//! `vm.call_value_immediate_nb(&closure, &[elem], ctx)` per playbook §1
//! and ADR-006 §2.7.11 / Q12.
//!
//! ## Two classes of body
//!
//! 1. **Value-search methods** (`indexOf`, `includes`) — receiver array +
//!    a search value; no closure dispatch. Element comparison dispatches
//!    on the receiver's `TypedArrayData` variant cross-checked against
//!    the search value's `NativeKind`. CLAUDE.md "No runtime coercion" —
//!    kind mismatch implies "not present" without coercion.
//!
//! 2. **Higher-order closure-callback methods that return Bool / scalar /
//!    Optional<elem>** (`find`, `findIndex`, `some`, `every`, `any`,
//!    `all`, `single`, `forEach`) — invoke a user closure per element.
//!    Element kind comes from the receiver's `TypedArrayData` variant
//!    (see `read_element_at`); closure result kind comes from
//!    `call_value_immediate_nb`'s returned `KindedSlot`. Predicate-form
//!    handlers strictly require the closure return `NativeKind::Bool`
//!    (no Bool-default fallback per ADR-006 §2.7.7 #9 / §2.7.8 #4 — a
//!    non-Bool predicate result is a `RuntimeError`).
//!
//! 3. **Higher-order closure-callback methods that build a typed-Array
//!    result** (`where`, `select`, `takeWhile`, `skipWhile`) — surface
//!    with explicit Phase-2c §2.7.4 reason: typed-array reconstruction
//!    requires the per-NativeKind builder matrix that lives in the
//!    sibling W9-array-transform / W9-array-aggregation cluster
//!    (`handle_filter_v2` / `handle_map_v2`). Implementing the
//!    closure-driven scan here without the result builder would either
//!    re-create the per-variant accumulator pattern (cluster cascade) or
//!    silently degrade to a HeapValue fallback (forbidden per
//!    CLAUDE.md "Forbidden Patterns"). Wired in W9-array-transform.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers — value-search support (no shim usage; pure dispatch on
// TypedArrayData variant + NativeKind cross-check)
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow the `&TypedArrayData` referenced by a `Ptr(HeapKind::TypedArray)`-
/// kinded receiver. Mirrors `array_basic::typed_array_ref`.
#[inline]
fn typed_array_ref<'a>(slot: &'a KindedSlot) -> Result<&'a TypedArrayData, VMError> {
    match slot.kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let bits = slot.slot.raw();
            // SAFETY: per the kinded-ABI contract, `Ptr(HeapKind::TypedArray)`
            // bits are `Arc::into_raw::<TypedArrayData>` and the dispatch
            // shell owns one strong-count share for the call duration.
            Ok(unsafe { &*(bits as *const TypedArrayData) })
        }
        _ => Err(VMError::TypeError {
            expected: "Array",
            got: "non-array",
        }),
    }
}

/// Element count for a `TypedArrayData`, dispatching on the variant.
/// Mirrors `array_basic::typed_array_len`.
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
    }
}

/// Read element at `idx` as a kinded carrier. The kind matches the
/// receiver's `TypedArrayData` variant; per-variant the bits encoding is
/// the same as `array_basic::read_element_at` (the canonical reference).
/// Narrow-int / matrix / heap-value-backed / float-slice arrays surface
/// per playbook §1 — element-kind-aware reads for those variants are
/// W9-array-basic's territory (the §2.7.6 / Q8 per-variant constructor
/// matrix needed to round-trip them through a `KindedSlot`).
fn read_element_at(arr: &TypedArrayData, idx: usize) -> Result<KindedSlot, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let v = buf.data.get(idx).copied().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_int(v))
        }
        TypedArrayData::F64(buf) => {
            let v = buf.data.get(idx).copied().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_number(v))
        }
        TypedArrayData::Bool(buf) => {
            let v = buf.data.get(idx).copied().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_bool(v != 0))
        }
        TypedArrayData::String(buf) => {
            let v = buf.data.get(idx).cloned().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_string_arc(v))
        }
        other => Err(VMError::NotImplemented(format!(
            "array element read: TypedArrayData variant {} — Phase-2c \
             reentry per ADR-006 §2.7.4. Element-kind-aware read for \
             narrow-int / matrix / heap-value-backed / float-slice arrays \
             needs the §2.7.6 / Q8 per-variant constructor matrix completed.",
            other.type_name()
        ))),
    }
}

/// Linear search for `needle` in `arr`. Returns `Some(idx)` on match or
/// `None` if absent / kind mismatch (no runtime coercion per
/// CLAUDE.md). Element kind on the receiver is captured per playbook §2
/// (typed-array element-source rule).
fn find_first_index(arr: &TypedArrayData, needle: &KindedSlot) -> Result<Option<usize>, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let Some(target) = needle.as_i64() else {
                return Ok(None);
            };
            Ok(buf.data.iter().position(|&v| v == target))
        }
        TypedArrayData::F64(buf) => {
            let Some(target) = needle.as_f64() else {
                return Ok(None);
            };
            // Float equality follows the IEEE-754 == relation as in the
            // pre-Wave-6.5 body — NaN never compares equal to itself.
            Ok(buf.data.iter().position(|&v| v == target))
        }
        TypedArrayData::Bool(buf) => {
            let Some(target) = needle.as_bool() else {
                return Ok(None);
            };
            let target_bits: u8 = if target { 1 } else { 0 };
            Ok(buf.data.iter().position(|&v| v == target_bits))
        }
        TypedArrayData::String(buf) => {
            let Some(target) = needle.as_str() else {
                return Ok(None);
            };
            Ok(buf.data.iter().position(|s| s.as_str() == target))
        }
        // Narrow-int / matrix / float-slice / heap-value-backed receivers
        // need element-kind-aware comparison the §2.7.6 / Q8 per-variant
        // constructor matrix completes in the W9-array-basic follow-up.
        // Surface explicitly per playbook §4 (API gap).
        other => Err(VMError::NotImplemented(format!(
            "Array.indexOf/includes: TypedArrayData variant {} — Phase-2c \
             reentry per ADR-006 §2.7.4. Element-kind-aware comparison \
             needs narrow-int width, heap-value-backed equality, matrix \
             shape, and float-slice view comparison.",
            other.type_name()
        ))),
    }
}

/// Read `args[1]` as a closure-kinded callee `&KindedSlot`. The
/// closure-callback handlers expect a `Ptr(HeapKind::Closure)` callee in
/// the second slot (receiver in slot 0). Surface a `RuntimeError` if the
/// kind doesn't classify (CLAUDE.md "No Bool-default fallback for
/// unknown kinds").
#[inline]
fn closure_arg<'a>(args: &'a [KindedSlot], op: &'static str) -> Result<&'a KindedSlot, VMError> {
    let Some(slot) = args.get(1) else {
        return Err(VMError::argument_count_error(op, 1, 0));
    };
    match slot.kind {
        // §2.7.11 / Q12 callee-classification kinds. `UInt64` is also
        // accepted by `call_value_immediate_nb` (function-id callees);
        // forward both kinds and let the dispatch body classify.
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => Ok(slot),
        other => Err(VMError::RuntimeError(format!(
            "Array.{}: closure argument must be a Closure or function ref, got {:?}",
            op, other
        ))),
    }
}

/// Invoke `closure(elem)` and require a Bool result. The kind of the
/// returned `KindedSlot` is sourced from the callee's last produced
/// opcode (per `call_value_immediate_nb`'s `pop_kinded` at the return
/// site); a non-Bool result surfaces as a `RuntimeError` rather than a
/// Bool-default fallback (forbidden per ADR-006 §2.7.7 #9 / §2.7.8 #4).
#[inline]
fn invoke_predicate(
    vm: &mut VirtualMachine,
    closure: &KindedSlot,
    elem: KindedSlot,
    ctx: Option<&mut ExecutionContext>,
    op: &'static str,
) -> Result<bool, VMError> {
    let result = vm.call_value_immediate_nb(closure, std::slice::from_ref(&elem), ctx)?;
    // `elem` Drop runs at end of scope, releasing its share. `result`
    // owns one share for the predicate's return (Bool: no Arc share, but
    // Drop is still a no-op).
    match result.as_bool() {
        Some(b) => Ok(b),
        None => Err(VMError::RuntimeError(format!(
            "Array.{}: predicate must return Bool, got {:?}",
            op,
            result.kind()
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (kinded ABI per ADR-006 §2.7.10 / Q11) handlers
// ═══════════════════════════════════════════════════════════════════════════
//
// Three classes:
//
// 1. Value-search (`indexOf`, `includes`) — real bodies, no closure dispatch.
// 2. Closure-callback predicates / scalar return (`find`, `findIndex`,
//    `some`, `every`, `any`, `all`, `single`, `forEach`) — real bodies
//    via `call_value_immediate_nb`.
// 3. Closure-callback typed-Array result (`where`, `select`, `takeWhile`,
//    `skipWhile`) — surface with explicit Phase-2c §2.7.4 reason; typed-
//    array reconstruction is W9-array-transform's territory.

const TYPED_ARRAY_BUILDER_SURFACE: &str =
    "SURFACE per ADR-006 §2.7.4 (Phase-2c reentry): typed-Array result \
     reconstruction requires the per-NativeKind result-builder matrix \
     that lives alongside `array_transform::handle_filter_v2` / \
     `handle_map_v2` in the sibling W9-array-transform sub-cluster. \
     Implementing the closure-driven scan here without that builder \
     would either re-create the per-variant accumulator pattern (cluster \
     cascade) or silently widen to `TypedArrayData::HeapValue` (forbidden \
     per CLAUDE.md \"No runtime coercion\" / \"Forbidden Patterns\"). \
     Wired in W9-array-transform once the per-variant builder lands.";

/// `arr.where(predicate)` — filter via closure callback.
///
/// SURFACE — typed-Array result builder. See `TYPED_ARRAY_BUILDER_SURFACE`.
pub(crate) fn handle_where_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.where — {}",
        TYPED_ARRAY_BUILDER_SURFACE
    )))
}

/// `arr.select(projector)` — project via closure callback.
///
/// SURFACE — typed-Array result builder. See `TYPED_ARRAY_BUILDER_SURFACE`.
pub(crate) fn handle_select_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.select — {}",
        TYPED_ARRAY_BUILDER_SURFACE
    )))
}

/// `arr.find(predicate)` — first matching element via closure callback.
/// Returns the matching element or `null` if no match.
pub(crate) fn handle_find_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "find")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        // Read element a second time so we can return the matching one
        // without depending on whether `invoke_predicate` consumed `elem`.
        // (`elem`'s share is released by its Drop at end of the predicate
        // call; the receiver still owns the source share, so a fresh
        // `read_element_at` is a clean independent share.)
        let matched = invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "find")?;
        if matched {
            return read_element_at(arr, i);
        }
    }
    Ok(KindedSlot::none())
}

/// `arr.findIndex(predicate)` — index of first matching element via
/// closure callback. Returns `-1` if no match.
pub(crate) fn handle_find_index_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "findIndex")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        if invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "findIndex")? {
            return Ok(KindedSlot::from_int(i as i64));
        }
    }
    Ok(KindedSlot::from_int(-1))
}

/// `arr.indexOf(value)` — first index of `value`, or `-1` if absent. No
/// closure dispatch; pure value-search.
pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::argument_count_error("indexOf", 1, 0));
    }
    let arr = typed_array_ref(&args[0])?;
    let idx = find_first_index(arr, &args[1])?;
    Ok(KindedSlot::from_int(idx.map(|i| i as i64).unwrap_or(-1)))
}

/// `arr.includes(value)` — true if `value` is present. No closure
/// dispatch; pure value-search.
pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::argument_count_error("includes", 1, 0));
    }
    let arr = typed_array_ref(&args[0])?;
    let idx = find_first_index(arr, &args[1])?;
    Ok(KindedSlot::from_bool(idx.is_some()))
}

/// `arr.some(predicate)` — true if any element satisfies the closure
/// predicate. Short-circuits on the first match.
pub(crate) fn handle_some_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "some")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        if invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "some")? {
            return Ok(KindedSlot::from_bool(true));
        }
    }
    Ok(KindedSlot::from_bool(false))
}

/// `arr.every(predicate)` — true if all elements satisfy the closure
/// predicate. Short-circuits on the first failure.
pub(crate) fn handle_every_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "every")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        if !invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "every")? {
            return Ok(KindedSlot::from_bool(false));
        }
    }
    Ok(KindedSlot::from_bool(true))
}

/// `arr.any(predicate)` — alias for `some` in the SQL-like query DSL.
pub(crate) fn handle_any_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "any")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        if invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "any")? {
            return Ok(KindedSlot::from_bool(true));
        }
    }
    Ok(KindedSlot::from_bool(false))
}

/// `arr.all(predicate)` — alias for `every` in the SQL-like query DSL.
pub(crate) fn handle_all_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "all")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        if !invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "all")? {
            return Ok(KindedSlot::from_bool(false));
        }
    }
    Ok(KindedSlot::from_bool(true))
}

/// `arr.single(predicate)` — exactly-one-match via closure predicate;
/// errors on zero or multiple matches.
pub(crate) fn handle_single_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "single")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    let mut found_idx: Option<usize> = None;
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        if invoke_predicate(vm, closure, elem, ctx.as_deref_mut(), "single")? {
            if found_idx.is_some() {
                return Err(VMError::RuntimeError(
                    "Array.single: more than one element matched the predicate".to_string(),
                ));
            }
            found_idx = Some(i);
        }
    }
    match found_idx {
        Some(i) => read_element_at(arr, i),
        None => Err(VMError::RuntimeError(
            "Array.single: no element matched the predicate".to_string(),
        )),
    }
}

/// `arr.takeWhile(predicate)` — prefix where the closure predicate
/// holds.
///
/// SURFACE — typed-Array result builder. The handler can scan the array
/// to compute the boundary index `k` (first index where the predicate
/// fails), but constructing the resulting `arr[0..k]` shares its
/// per-NativeKind reconstruction surface with `where` / `select` /
/// `slice`. See `TYPED_ARRAY_BUILDER_SURFACE`.
pub(crate) fn handle_take_while_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.takeWhile — {}",
        TYPED_ARRAY_BUILDER_SURFACE
    )))
}

/// `arr.skipWhile(predicate)` — suffix from the first element where the
/// closure predicate fails.
///
/// SURFACE — typed-Array result builder. Same dependency as `takeWhile`.
/// See `TYPED_ARRAY_BUILDER_SURFACE`.
pub(crate) fn handle_skip_while_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.skipWhile — {}",
        TYPED_ARRAY_BUILDER_SURFACE
    )))
}

/// `arr.forEach(action)` — invoke the closure on every element. The
/// closure result is discarded (its share is released by `result`'s
/// `Drop` when it goes out of scope each iteration). Returns null per
/// the JS / SQL `forEach` contract.
pub(crate) fn handle_for_each_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let closure = closure_arg(args, "forEach")?;
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    for i in 0..len {
        let elem = read_element_at(arr, i)?;
        // Result share is released by its `Drop` at end of scope; we
        // do not require any particular kind for forEach (action form).
        let _ =
            vm.call_value_immediate_nb(closure, std::slice::from_ref(&elem), ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}
