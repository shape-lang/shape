//! Array set operations
//!
//! Handles: union, intersect, except, unique, distinct, distinct_by
//!
//! ## Wave-δ `MR-array-sort-sets-joins` body migration (ADR-006 §2.7.10 / Q11)
//!
//! Bodies migrated off the prior `NotImplemented(SURFACE)` framing now
//! that the kinded `MethodFnV2` ABI is operational (Wave-γ
//! `G-method-fn-v2-abi`). The receiver enters as
//! `args[0]: KindedSlot { kind: NativeKind::Ptr(HeapKind::TypedArray) }`;
//! payload recovery follows ADR-005 §1 single-discriminator dispatch via
//! `args[0].slot.as_heap_value()` + `HeapValue::TypedArray(arc)` match,
//! mirroring the Wave 5b body precedent in
//! `executor/builtins/array_ops.rs` (`as_typed_array` borrow helper).
//! Result construction uses `KindedSlot::from_typed_array(Arc::new(...))`
//! per playbook §3.
//!
//! ## Per-element-kind equality (no closure)
//!
//! Set ops `union` / `intersect` / `except` / `unique` / `distinct`
//! discriminate equality via the `TypedArrayData::*` arm: each arm carries
//! a single Rust scalar/`Arc` type, so `PartialEq` on the inner element
//! matches the playbook's "per-element-kind equality" contract verbatim
//! (i64 `==`, f64 `==`, `Arc<String>` content equality). Non-finite f64
//! NaNs follow IEEE 754 (NaN != NaN); this mirrors the pre-Wave-6 body
//! behaviour.
//!
//! ## `distinctBy` — closure-discriminator path
//!
//! `distinctBy(arr, keyFn)` calls `keyFn(elem)` per element and deduplicates
//! by the resulting key. The closure callback path
//! (`executor/call_convention.rs::call_value_immediate_*` and
//! `executor/control_flow/mod.rs::op_call_value`) is itself
//! `NotImplemented(SURFACE)` post-§2.7.10 (the kinded callee dispatch +
//! `&[KindedSlot]` arg slice on the runtime side has not landed in
//! Wave-γ); without it the handler cannot invoke the user's closure.
//! Surface per playbook §7.4 REVISED.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{
    HeapKind, HeapValue, KindedSlot, NativeKind, TypedArrayData, TypedBuffer, VMError,
};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers — no shim usage; pure dispatch on `TypedArrayData` variants.
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow the `Arc<TypedArrayData>` payload from a `KindedSlot` whose
/// `kind == NativeKind::Ptr(HeapKind::TypedArray)`. Mirrors the Wave 5b
/// `as_typed_array` precedent in `executor/builtins/array_ops.rs`.
#[inline]
fn as_typed_array(slot: &KindedSlot) -> Option<&Arc<TypedArrayData>> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::TypedArray)) {
        return None;
    }
    match slot.slot.as_heap_value() {
        HeapValue::TypedArray(arc) => Some(arc),
        _ => None,
    }
}

#[inline]
fn typed_array_to_slot(arr: TypedArrayData) -> KindedSlot {
    KindedSlot::from_typed_array(Arc::new(arr))
}

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Empty `TypedArrayData` of the same arm shape as `arr`. Matches the
/// pre-Wave-6 set-op contract: an empty result preserves the source
/// element shape so downstream ops (push, sum, etc.) can keep dispatching
/// on the same arm.
fn empty_like(arr: &TypedArrayData) -> Result<TypedArrayData, VMError> {
    Ok(match arr {
        TypedArrayData::I64(_) => TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(Vec::new()))),
        TypedArrayData::F64(_) => TypedArrayData::F64(Arc::new(
            shape_value::AlignedTypedBuffer::from(shape_value::AlignedVec::from_vec(
                Vec::<f64>::new(),
            )),
        )),
        TypedArrayData::Bool(_) => TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(Vec::new()))),
        TypedArrayData::String(_) => {
            TypedArrayData::String(Arc::new(TypedBuffer::from_vec(Vec::new())))
        }
        other => {
            return Err(type_error(format!(
                "set ops not supported for TypedArrayData variant {} (Phase-2c reentry — \
                 narrow-width / matrix / heterogeneous-heap arms not part of the Wave-δ \
                 set-ops migration)",
                other.type_name()
            )));
        }
    })
}

/// Element-kind-aware "contains" probe. Returns `Ok(true)` when `arr`
/// already contains an element equal to `arr[probe_idx]` somewhere in
/// `arr[0..probe_idx]`. Used by the unique/distinct stable dedup pass.
///
/// IEEE 754 NaN follows `==` semantics (NaN != NaN), matching the
/// pre-Wave-6 behaviour and the Rust `PartialEq` contract.
fn already_seen(arr: &TypedArrayData, probe_idx: usize) -> Result<bool, VMError> {
    Ok(match arr {
        TypedArrayData::I64(buf) => {
            let v = buf.data[probe_idx];
            buf.data[..probe_idx].iter().any(|&x| x == v)
        }
        TypedArrayData::F64(buf) => {
            let v = buf.data[probe_idx];
            buf.data[..probe_idx].iter().any(|&x| x == v)
        }
        TypedArrayData::Bool(buf) => {
            let v = buf.data[probe_idx];
            buf.data[..probe_idx].iter().any(|&x| x == v)
        }
        TypedArrayData::String(buf) => {
            let v = &buf.data[probe_idx];
            buf.data[..probe_idx].iter().any(|x| x.as_str() == v.as_str())
        }
        other => {
            return Err(type_error(format!(
                "set ops not supported for TypedArrayData variant {} (Phase-2c reentry — \
                 narrow-width / matrix / heterogeneous-heap arms not part of the Wave-δ \
                 set-ops migration)",
                other.type_name()
            )));
        }
    })
}

/// Element-kind-aware "contained in `other`" probe. Returns `true` when
/// `lhs[lhs_idx]` equals some element of `rhs`. Both arrays must have the
/// same arm shape; arm-shape mismatch surfaces as a `TypeError`.
fn lhs_in_rhs(lhs: &TypedArrayData, lhs_idx: usize, rhs: &TypedArrayData) -> Result<bool, VMError> {
    Ok(match (lhs, rhs) {
        (TypedArrayData::I64(a), TypedArrayData::I64(b)) => {
            let v = a.data[lhs_idx];
            b.data.iter().any(|&x| x == v)
        }
        (TypedArrayData::F64(a), TypedArrayData::F64(b)) => {
            let v = a.data[lhs_idx];
            b.data.iter().any(|&x| x == v)
        }
        (TypedArrayData::Bool(a), TypedArrayData::Bool(b)) => {
            let v = a.data[lhs_idx];
            b.data.iter().any(|&x| x == v)
        }
        (TypedArrayData::String(a), TypedArrayData::String(b)) => {
            let v = &a.data[lhs_idx];
            b.data.iter().any(|x| x.as_str() == v.as_str())
        }
        (l, r) => {
            return Err(type_error(format!(
                "set ops require matching element shape (lhs={}, rhs={}); cross-shape \
                 set ops are Phase-2c reentry territory",
                l.type_name(),
                r.type_name()
            )));
        }
    })
}

/// Build a result `TypedArrayData` of the same arm as `template` from a
/// list of source `(arr, idx)` pairs. Selecting via index avoids any
/// element-clone roundtrip through `KindedSlot` and keeps the inner
/// `Arc<String>` shares stable.
fn build_from_indices(
    template: &TypedArrayData,
    sources: &[(&TypedArrayData, &[usize])],
) -> Result<TypedArrayData, VMError> {
    Ok(match template {
        TypedArrayData::I64(_) => {
            let mut out: Vec<i64> = Vec::new();
            for (src, idxs) in sources {
                let buf = match src {
                    TypedArrayData::I64(b) => b,
                    other => {
                        return Err(type_error(format!(
                            "set-op build: arm mismatch (template=I64, source={})",
                            other.type_name()
                        )));
                    }
                };
                for &i in idxs.iter() {
                    out.push(buf.data[i]);
                }
            }
            TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(out)))
        }
        TypedArrayData::F64(_) => {
            let mut out: Vec<f64> = Vec::new();
            for (src, idxs) in sources {
                let buf = match src {
                    TypedArrayData::F64(b) => b,
                    other => {
                        return Err(type_error(format!(
                            "set-op build: arm mismatch (template=F64, source={})",
                            other.type_name()
                        )));
                    }
                };
                for &i in idxs.iter() {
                    out.push(buf.data[i]);
                }
            }
            let av = shape_value::AlignedVec::from_vec(out);
            TypedArrayData::F64(Arc::new(shape_value::AlignedTypedBuffer::from(av)))
        }
        TypedArrayData::Bool(_) => {
            let mut out: Vec<u8> = Vec::new();
            for (src, idxs) in sources {
                let buf = match src {
                    TypedArrayData::Bool(b) => b,
                    other => {
                        return Err(type_error(format!(
                            "set-op build: arm mismatch (template=Bool, source={})",
                            other.type_name()
                        )));
                    }
                };
                for &i in idxs.iter() {
                    out.push(buf.data[i]);
                }
            }
            TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(out)))
        }
        TypedArrayData::String(_) => {
            let mut out: Vec<Arc<String>> = Vec::new();
            for (src, idxs) in sources {
                let buf = match src {
                    TypedArrayData::String(b) => b,
                    other => {
                        return Err(type_error(format!(
                            "set-op build: arm mismatch (template=String, source={})",
                            other.type_name()
                        )));
                    }
                };
                for &i in idxs.iter() {
                    out.push(Arc::clone(&buf.data[i]));
                }
            }
            TypedArrayData::String(Arc::new(TypedBuffer::from_vec(out)))
        }
        other => {
            return Err(type_error(format!(
                "set ops not supported for TypedArrayData variant {} (Phase-2c reentry)",
                other.type_name()
            )));
        }
    })
}

/// Compute the indices of `arr` that produce the deduplicated, order-
/// preserving "unique" subsequence. Each index `i` is included iff no
/// earlier `j < i` has `arr[j] == arr[i]`.
fn unique_indices(arr: &TypedArrayData) -> Result<Vec<usize>, VMError> {
    let len = match arr {
        TypedArrayData::I64(b) => b.len(),
        TypedArrayData::F64(b) => b.len(),
        TypedArrayData::Bool(b) => b.len(),
        TypedArrayData::String(b) => b.len(),
        other => {
            return Err(type_error(format!(
                "set ops not supported for TypedArrayData variant {} (Phase-2c reentry)",
                other.type_name()
            )));
        }
    };
    let mut out: Vec<usize> = Vec::with_capacity(len);
    for i in 0..len {
        if !already_seen(arr, i)? {
            out.push(i);
        }
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `union` — set union of two arrays (deduplicated, order-preserving).
///
/// args: [array, other_array]
pub(crate) fn handle_union_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("union() requires 2 arguments (array, other)"));
    }
    let lhs = as_typed_array(&args[0])
        .ok_or_else(|| type_error("union(): receiver must be an Array"))?;
    let rhs = as_typed_array(&args[1])
        .ok_or_else(|| type_error("union(): argument must be an Array"))?;
    let lhs = lhs.as_ref();
    let rhs = rhs.as_ref();

    // Concatenated dedup, lhs-first then rhs (order-preserving). The
    // already_seen predicate is per-element-kind equality on the concat
    // result; we materialize the concat lazily via the index-list build.
    let lhs_uniq_idx = unique_indices(lhs)?;
    let mut rhs_extra_idx: Vec<usize> = Vec::new();
    let rhs_len = match rhs {
        TypedArrayData::I64(b) => b.len(),
        TypedArrayData::F64(b) => b.len(),
        TypedArrayData::Bool(b) => b.len(),
        TypedArrayData::String(b) => b.len(),
        other => {
            return Err(type_error(format!(
                "union(): arm {} unsupported",
                other.type_name()
            )));
        }
    };
    for i in 0..rhs_len {
        // Skip if rhs[i] is in lhs (the deduped lhs uses original
        // ordering, so probing the full lhs is equivalent and avoids
        // double-bookkeeping).
        if lhs_in_rhs(rhs, i, lhs)? {
            continue;
        }
        // Skip if rhs[i] equals an earlier rhs element we've already
        // taken (rhs-internal dedup, order-preserving).
        let already = rhs_extra_idx
            .iter()
            .map(|&j| j)
            .try_fold(false, |acc, j| -> Result<bool, VMError> {
                if acc {
                    return Ok(true);
                }
                // pairwise equality at indices i and j on rhs
                Ok(rhs_pair_eq(rhs, i, j)?)
            })?;
        if !already {
            rhs_extra_idx.push(i);
        }
    }

    let template = lhs;
    let result = build_from_indices(template, &[(lhs, &lhs_uniq_idx), (rhs, &rhs_extra_idx)])?;
    Ok(typed_array_to_slot(result))
}

/// Pairwise equality on two indices of the same `TypedArrayData`.
fn rhs_pair_eq(arr: &TypedArrayData, i: usize, j: usize) -> Result<bool, VMError> {
    Ok(match arr {
        TypedArrayData::I64(b) => b.data[i] == b.data[j],
        TypedArrayData::F64(b) => b.data[i] == b.data[j],
        TypedArrayData::Bool(b) => b.data[i] == b.data[j],
        TypedArrayData::String(b) => b.data[i].as_str() == b.data[j].as_str(),
        other => {
            return Err(type_error(format!(
                "set ops: arm {} unsupported",
                other.type_name()
            )));
        }
    })
}

/// v2 `intersect` — set intersection of two arrays (deduplicated, order-
/// preserving over the lhs).
///
/// args: [array, other_array]
pub(crate) fn handle_intersect_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "intersect() requires 2 arguments (array, other)",
        ));
    }
    let lhs = as_typed_array(&args[0])
        .ok_or_else(|| type_error("intersect(): receiver must be an Array"))?;
    let rhs = as_typed_array(&args[1])
        .ok_or_else(|| type_error("intersect(): argument must be an Array"))?;
    let lhs = lhs.as_ref();
    let rhs = rhs.as_ref();

    let lhs_len = match lhs {
        TypedArrayData::I64(b) => b.len(),
        TypedArrayData::F64(b) => b.len(),
        TypedArrayData::Bool(b) => b.len(),
        TypedArrayData::String(b) => b.len(),
        other => {
            return Err(type_error(format!(
                "intersect(): arm {} unsupported",
                other.type_name()
            )));
        }
    };
    let mut idxs: Vec<usize> = Vec::new();
    for i in 0..lhs_len {
        if !lhs_in_rhs(lhs, i, rhs)? {
            continue;
        }
        // Dedup against earlier picks.
        let mut already = false;
        for &j in idxs.iter() {
            if rhs_pair_eq(lhs, i, j)? {
                already = true;
                break;
            }
        }
        if !already {
            idxs.push(i);
        }
    }
    let result = if idxs.is_empty() {
        empty_like(lhs)?
    } else {
        build_from_indices(lhs, &[(lhs, &idxs)])?
    };
    Ok(typed_array_to_slot(result))
}

/// v2 `except` — set difference of two arrays (deduplicated, order-
/// preserving over the lhs).
///
/// args: [array, other_array]
pub(crate) fn handle_except_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("except() requires 2 arguments (array, other)"));
    }
    let lhs = as_typed_array(&args[0])
        .ok_or_else(|| type_error("except(): receiver must be an Array"))?;
    let rhs = as_typed_array(&args[1])
        .ok_or_else(|| type_error("except(): argument must be an Array"))?;
    let lhs = lhs.as_ref();
    let rhs = rhs.as_ref();

    let lhs_len = match lhs {
        TypedArrayData::I64(b) => b.len(),
        TypedArrayData::F64(b) => b.len(),
        TypedArrayData::Bool(b) => b.len(),
        TypedArrayData::String(b) => b.len(),
        other => {
            return Err(type_error(format!(
                "except(): arm {} unsupported",
                other.type_name()
            )));
        }
    };
    let mut idxs: Vec<usize> = Vec::new();
    for i in 0..lhs_len {
        if lhs_in_rhs(lhs, i, rhs)? {
            continue;
        }
        // Dedup against earlier picks.
        let mut already = false;
        for &j in idxs.iter() {
            if rhs_pair_eq(lhs, i, j)? {
                already = true;
                break;
            }
        }
        if !already {
            idxs.push(i);
        }
    }
    let result = if idxs.is_empty() {
        empty_like(lhs)?
    } else {
        build_from_indices(lhs, &[(lhs, &idxs)])?
    };
    Ok(typed_array_to_slot(result))
}

/// v2 `unique` — deduplicate array elements (order-preserving).
///
/// args: [array]
pub(crate) fn handle_unique_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("unique() requires 1 argument (array)"));
    }
    let arc = as_typed_array(&args[0])
        .ok_or_else(|| type_error("unique(): receiver must be an Array"))?;
    let arr = arc.as_ref();
    let idxs = unique_indices(arr)?;
    let result = if idxs.is_empty() {
        empty_like(arr)?
    } else {
        build_from_indices(arr, &[(arr, &idxs)])?
    };
    Ok(typed_array_to_slot(result))
}

/// v2 `distinct` — alias for `unique`.
///
/// args: [array]
pub(crate) fn handle_distinct_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_unique_v2(vm, args, ctx)
}

/// v2 `distinctBy` — deduplicate by a key function (order-preserving).
///
/// args: [array, key_fn]
///
/// **SURFACE — Wave-δ closure-callback dependency.** Per playbook §7.4
/// REVISED, the kinded closure-callback path
/// (`call_value_immediate_*` / `op_call_value` in
/// `executor/control_flow/mod.rs` and `executor/call_convention.rs`) is
/// itself an unmigrated `NotImplemented(SURFACE)` post-§2.7.10: the
/// kinded-callee dispatch + `&[KindedSlot]` arg-slice on the runtime side
/// is Phase-2c rebuild territory. Without it, the handler cannot invoke
/// the user's `key_fn`. The body shape would otherwise mirror
/// `handle_unique_v2` with a per-element `key_fn(elem)` callback driving
/// a stable dedup over computed keys.
pub(crate) fn handle_distinct_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "distinctBy — SURFACE: closure-callback path unmigrated. \
         The kinded MethodFnV2 ABI landed (ADR-006 §2.7.10 / Q11), but \
         `call_value_immediate_*` / `op_call_value` (executor/call_convention.rs, \
         executor/control_flow/mod.rs) still return NotImplemented(SURFACE) \
         pending the kinded callee dispatch + `&[KindedSlot]` arg-slice rebuild \
         per ADR-006 §2.7.4 / §2.7.8. Body shape would mirror handle_unique_v2 \
         with a per-element `key_fn(elem)` callback driving a stable dedup over \
         computed keys."
            .to_string(),
    ))
}
