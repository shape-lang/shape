//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join
//!
//! ## Wave-δ `MR-array-sort-sets-joins` body migration (ADR-006 §2.7.10 / Q11)
//!
//! Every handler in this file requires the kinded value-call path
//! (`call_value_immediate_nb` in `executor/call_convention.rs`,
//! `op_call_value` / `dispatch_call_value_immediate` in
//! `executor/control_flow/mod.rs`) for left-key / right-key /
//! result-selector closure invocations, plus per-element kind dispatch
//! on two `TypedArrayData` arms (the cross-array-shape join product).
//! The kinded `MethodFnV2` ABI landed in Wave-γ `G-method-fn-v2-abi` —
//! `args[0..1]` arrive as
//! `KindedSlot { kind: NativeKind::Ptr(HeapKind::TypedArray) }` carriers
//! per ADR-005 §1 single-discriminator dispatch.
//!
//! Wave 7 (W7-cv-static / W7-cv-method / W7-op-call-value) closed the
//! kinded value-call ABI per ADR-006 §2.7.11 / Q12:
//! `call_value_immediate_nb(callee: &KindedSlot, args: &[KindedSlot]) ->
//!  Result<KindedSlot, VMError>` is live in `call_convention.rs:767`,
//! `op_call_value` and `dispatch_call_value_immediate` are filled, and
//! the closure-self carrier integrates `closure_heap_kind: Option<NativeKind>`
//! per the §2.7.8/Q10 lockstep. The remaining upstream gate for *user-
//! produced* closures is `op_make_closure` in
//! `executor/control_flow/mod.rs:447`, still
//! `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)` pending the kinded
//! capture-read + closure-block construction rebuild (ADR-006 §2.7.4 /
//! §2.7.5 / §2.7.8). Without `op_make_closure`, no user closure
//! `KindedSlot { kind: NativeKind::Ptr(HeapKind::Closure) }` carrier
//! reaches a join handler's arg slice; filling the body here would
//! produce dead code rather than an end-to-end working dispatch path.
//!
//! The Wave-β `M-datatable` cluster's `executor/objects/datatable_methods/joins.rs`
//! (`handle_inner_join` / `handle_left_join`) flipped its ABI to
//! `&[KindedSlot] → Result<KindedSlot, _>` (commit `eb78699`) but kept
//! the bodies as `NotImplemented(SURFACE: phase-2c body migration)` for
//! the same upstream `op_make_closure` reason. Routing this file's
//! handlers through `datatable_methods::joins` does not unblock; the
//! shared blocker is closure construction.
//!
//! Per playbook §7.4 REVISED: surface explicitly with the upstream
//! `op_make_closure` gate named, never a forbidden-pattern workaround.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

use crate::executor::objects::array_transform::{
    bump_closure_share, collect_homogeneous_results as transform_collect_homogeneous_results,
    element_kinded as transform_element_kinded, typed_array_arc_from_kinded,
    typed_array_len as transform_typed_array_len,
};

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers — closure-callback support
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Recover an independent `Arc<TypedArrayData>` strong-count share from
/// `slot`. Accepts both array carriers (heap-Arc + v2 raw-pointer) via
/// the shared helper.
fn typed_array_arc(slot: &KindedSlot, op: &str) -> Result<Arc<TypedArrayData>, VMError> {
    typed_array_arc_from_kinded(slot, op)
}

/// Validate a closure callee kind. Accepts Closure / function-ref;
/// rejects anything else with a `RuntimeError`.
fn closure_arg<'a>(args: &'a [KindedSlot], idx: usize, op: &'static str) -> Result<&'a KindedSlot, VMError> {
    let Some(slot) = args.get(idx) else {
        return Err(type_error(format!(
            "{}: missing closure argument at index {}",
            op, idx
        )));
    };
    match slot.kind {
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => Ok(slot),
        other => Err(type_error(format!(
            "{}: argument {} must be a closure or function ref, got kind {:?}",
            op, idx, other
        ))),
    }
}

/// Per-element-kind equality probe for join keys. Same semantics as
/// `array_sets::key_eq`: strict typing, no implicit coercion,
/// IEEE 754 NaN-never-equal for f64.
fn key_eq(a: &KindedSlot, b: &KindedSlot, op: &str) -> Result<bool, VMError> {
    if a.kind != b.kind {
        return Ok(false);
    }
    Ok(match a.kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => (a.slot.raw() as i64) == (b.slot.raw() as i64),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => a.slot.raw() == b.slot.raw(),
        NativeKind::Float64 => a.slot.as_f64() == b.slot.as_f64(),
        NativeKind::Bool => a.slot.as_bool() == b.slot.as_bool(),
        NativeKind::String => {
            let sa = a.as_str().unwrap_or("");
            let sb = b.as_str().unwrap_or("");
            sa == sb
        }
        other => {
            return Err(VMError::NotImplemented(format!(
                "{}: key equality for kind {:?} — SURFACE per ADR-006 §2.7.6 / Q8: \
                 only inline-scalar / String key kinds are dispatched in \
                 W17-array-closure-callback. Heap-typed keys (Decimal, BigInt, \
                 TypedObject, ...) need a per-kind equality table; Phase-2c reentry.",
                op, other
            )));
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — closure-callback dependency surfaces
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `innerJoin` — inner join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
///
/// W17-array-closure-callback: body filled now that `op_make_closure`
/// (W17-make-closure close `aa47364`) and `call_value_immediate_nb`
/// (W7 close `06cdfce`) are both live. Body shape:
///   1. Compute right-side keys up-front (one closure call per right
///      element) and cache them.
///   2. For each left element, compute its key, then linear-scan the
///      right-side key cache for matches (O(n*m) — a hash-keyed
///      multimap would require a per-NativeKind hasher matrix, which
///      is the §2.7.6 / Q8 cluster cascade; the linear pass matches
///      typed-array equality and is fast enough for the row counts the
///      stdlib query DSL targets).
///   3. For each match, invoke `result_selector_fn(left, right)` to
///      produce a result element.
///   4. Collect homogeneous results via
///      `array_transform::collect_homogeneous_results` (same
///      per-NativeKind builder matrix as `select` / `map`).
///
/// Heterogeneous-result kinds and `the-deleted-heterogeneous-element-carrier` receivers
/// surface per the §2.7.4 per-element kind metadata gap (the documented
/// closure-callback homogeneity contract).
pub(crate) fn handle_inner_join_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 5 {
        return Err(type_error(
            "innerJoin: expected (left, right, leftKey, rightKey, selector)",
        ));
    }
    let left_arc = typed_array_arc(&args[0], "innerJoin")?;
    let right_arc = typed_array_arc(&args[1], "innerJoin")?;
    let left_key = closure_arg(args, 2, "innerJoin")?;
    let right_key = closure_arg(args, 3, "innerJoin")?;
    let selector = closure_arg(args, 4, "innerJoin")?;

    let left_len = transform_typed_array_len(&left_arc);
    let right_len = transform_typed_array_len(&right_arc);

    // Cache right-side keys (one closure call per right element).
    // Per-iteration `bump_closure_share` balances `op_return`'s
    // `drop_with_kind` on the frame's `closure_heap_bits` — see the
    // helper's doc-comment for the §2.7.11 / Q12 caller-side
    // ownership contract.
    let mut right_keys: Vec<KindedSlot> = Vec::with_capacity(right_len);
    for j in 0..right_len {
        let elem = transform_element_kinded(&right_arc, j)?;
        bump_closure_share(right_key);
        let key = vm.call_value_immediate_nb(right_key, &[elem], ctx.as_deref_mut())?;
        right_keys.push(key);
    }

    // For each left element, compute its key and emit selector results
    // for every right match.
    let mut results: Vec<KindedSlot> = Vec::new();
    for i in 0..left_len {
        let l_elem = transform_element_kinded(&left_arc, i)?;
        // Cache the kind so we can keep the value alive across the key
        // call and reread it for each match (cheap — element_kinded is
        // a pure dispatch + scalar copy for the supported arms).
        bump_closure_share(left_key);
        let lk = vm.call_value_immediate_nb(left_key, &[l_elem], ctx.as_deref_mut())?;
        for (j, rk) in right_keys.iter().enumerate() {
            if key_eq(&lk, rk, "innerJoin")? {
                // Re-read both elements per match. element_kinded clones
                // share for heap kinds (e.g. String Arc), so each
                // selector arg owns an independent share.
                let l_arg = transform_element_kinded(&left_arc, i)?;
                let r_arg = transform_element_kinded(&right_arc, j)?;
                bump_closure_share(selector);
                let res = vm.call_value_immediate_nb(
                    selector,
                    &[l_arg, r_arg],
                    ctx.as_deref_mut(),
                )?;
                results.push(res);
            }
        }
    }

    let out = transform_collect_homogeneous_results(results)?;
    Ok(KindedSlot::from_typed_array(out))
}

/// v2 `leftJoin` — left join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
///
/// W17-array-closure-callback: body filled. Same shape as
/// `innerJoin`, with one addition: when a left element has no matching
/// right element, the selector is invoked with `(left, null)` — the
/// null sentinel is `KindedSlot::none()`, the canonical absent-value
/// carrier (kind = Bool, bits = 0, per `KindedSlot::none()`). This
/// matches the JS/SQL `LEFT JOIN` contract; the selector is
/// responsible for handling the null right argument.
///
/// The §2.7.7 #9 "no Bool-default fallback" rule applies to kind
/// fabrication when a kind source is missing — not to the documented
/// `KindedSlot::none()` sentinel, which is itself a real value carrier
/// observed in `KindedSlot::from_int(-1)` / `KindedSlot::none()`
/// returns across the codebase. `find`, `single`, `option_get` all
/// use this same sentinel; `leftJoin`'s unmatched-row case is the
/// natural call site for it.
pub(crate) fn handle_left_join_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 5 {
        return Err(type_error(
            "leftJoin: expected (left, right, leftKey, rightKey, selector)",
        ));
    }
    let left_arc = typed_array_arc(&args[0], "leftJoin")?;
    let right_arc = typed_array_arc(&args[1], "leftJoin")?;
    let left_key = closure_arg(args, 2, "leftJoin")?;
    let right_key = closure_arg(args, 3, "leftJoin")?;
    let selector = closure_arg(args, 4, "leftJoin")?;

    let left_len = transform_typed_array_len(&left_arc);
    let right_len = transform_typed_array_len(&right_arc);

    let mut right_keys: Vec<KindedSlot> = Vec::with_capacity(right_len);
    for j in 0..right_len {
        let elem = transform_element_kinded(&right_arc, j)?;
        bump_closure_share(right_key);
        let key = vm.call_value_immediate_nb(right_key, &[elem], ctx.as_deref_mut())?;
        right_keys.push(key);
    }

    let mut results: Vec<KindedSlot> = Vec::new();
    for i in 0..left_len {
        let l_elem = transform_element_kinded(&left_arc, i)?;
        bump_closure_share(left_key);
        let lk = vm.call_value_immediate_nb(left_key, &[l_elem], ctx.as_deref_mut())?;
        let mut matched = false;
        for (j, rk) in right_keys.iter().enumerate() {
            if key_eq(&lk, rk, "leftJoin")? {
                let l_arg = transform_element_kinded(&left_arc, i)?;
                let r_arg = transform_element_kinded(&right_arc, j)?;
                bump_closure_share(selector);
                let res = vm.call_value_immediate_nb(
                    selector,
                    &[l_arg, r_arg],
                    ctx.as_deref_mut(),
                )?;
                results.push(res);
                matched = true;
            }
        }
        if !matched {
            // Unmatched-left row: invoke selector(left, null) using the
            // canonical absent-value sentinel `KindedSlot::none()`.
            let l_arg = transform_element_kinded(&left_arc, i)?;
            bump_closure_share(selector);
            let res = vm.call_value_immediate_nb(
                selector,
                &[l_arg, KindedSlot::none()],
                ctx.as_deref_mut(),
            )?;
            results.push(res);
        }
    }

    let out = transform_collect_homogeneous_results(results)?;
    Ok(KindedSlot::from_typed_array(out))
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product).
///
/// args: [left_array, right_array, result_selector_fn]
///
/// W17-array-closure-callback: body filled. Cartesian product over
/// (left, right) pairs with the selector applied to each pair. No key
/// extractors — every pair is emitted. Result kind comes from the
/// selector return (homogeneous via `collect_homogeneous_results`).
pub(crate) fn handle_cross_join_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(type_error(
            "crossJoin: expected (left, right, selector)",
        ));
    }
    let left_arc = typed_array_arc(&args[0], "crossJoin")?;
    let right_arc = typed_array_arc(&args[1], "crossJoin")?;
    let selector = closure_arg(args, 2, "crossJoin")?;

    let left_len = transform_typed_array_len(&left_arc);
    let right_len = transform_typed_array_len(&right_arc);

    let mut results: Vec<KindedSlot> = Vec::with_capacity(left_len * right_len);
    for i in 0..left_len {
        for j in 0..right_len {
            let l_arg = transform_element_kinded(&left_arc, i)?;
            let r_arg = transform_element_kinded(&right_arc, j)?;
            bump_closure_share(selector);
            let res = vm.call_value_immediate_nb(
                selector,
                &[l_arg, r_arg],
                ctx.as_deref_mut(),
            )?;
            results.push(res);
        }
    }

    let out = transform_collect_homogeneous_results(results)?;
    Ok(KindedSlot::from_typed_array(out))
}
