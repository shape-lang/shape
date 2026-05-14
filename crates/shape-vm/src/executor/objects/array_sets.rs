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
//! by the resulting key. The kinded value-call path
//! (`call_value_immediate_nb` in `call_convention.rs:767`,
//! `dispatch_call_value_immediate` in `control_flow/mod.rs:389`) is live
//! post-W7 (ADR-006 §2.7.11 / Q12). The upstream gate is `op_make_closure`
//! in `control_flow/mod.rs:447`, still
//! `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)` pending the kinded
//! capture-read + closure-block construction rebuild (ADR-006 §2.7.4 /
//! §2.7.5 / §2.7.8). Without it no user closure `KindedSlot` carrier
//! reaches `args[1]`. Surface per playbook §7.4 REVISED.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::{
    HeapKind, HeapValue, KindedSlot, NativeKind, TypedArrayData, TypedBuffer, VMError,
};
use std::sync::Arc;

use crate::executor::objects::array_transform::{
    bump_closure_share, element_kinded as transform_element_kinded,
    project_indices as transform_project_indices, typed_array_arc_from_kinded,
    typed_array_len as transform_typed_array_len,
};

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
// Wave 2 Round 3a' Agent δ — v2-raw String/Decimal handler arms.
//
// Per supervisor 2026-05-14 disposition (1): A2-followup-mechanical split into
// 7 per-handler-family parallel sub-agents; each per-handler-family file lands
// v2-raw String/Decimal handler arms as UNREACHABLE code while the producer
// gate `should_use_typed_array` in `compiler/typed_emission.rs` stays CLOSED
// for `ConcreteType::String / Decimal`. The gate-flip itself lands as a
// SEPARATE sequential `A2-followup-gate-flip` agent dispatched AFTER α-η merge
// ceremony — between sub-commits no type-confusion-window risk (unreachable
// code is just unreachable code).
//
// **Producer gate state (verified at HEAD `5d0f1524`)**:
//   `should_use_typed_array(ConcreteType::String / Decimal) → None`
//   → producers emit legacy `OpCode::NewArray` → `TypedArrayData::String /
//     Decimal(Arc::new(TypedBuffer::from_vec(...)))` shape, NOT v2-raw
//     `*mut TypedArray<*const StringObj/DecimalObj>` shape.
//   → no `args[0].kind == NativeKind::UInt64` with elem_type stamp
//     String/Decimal can reach these handlers under user code.
//
// **Why this arm exists pre-gate-flip**: lockstep landing requires the v2-raw
// shape's consumer surface to be complete BEFORE the gate flips; otherwise the
// gate-flip commit would itself need to land the consumer arms across 7+ files
// and exceed single-LLM-session capacity (ceiling-c finding from D3 Round 3a
// close). Splitting the consumer-side landing across α-η parallel sub-agents
// keeps each cargo-check-clean with bounded scope (~20-50 LoC each) while
// preserving the atomic-flip invariant for the gate flip itself.
//
// **Shape**: per audit §4.1.B.4 migration recipe. The receiver's v2-raw
// `TypedArray<*const StringObj/DecimalObj>` pointer is walked directly (no
// materialize-to-Arc<TypedArrayData> hop per §4.1.B.3 forbidden); equality
// uses pointer-content comparison via `StringObj::as_str(ptr)` /
// `DecimalObj::value(ptr)`; the result is a fresh
// `TypedArray::<*const StringObj/DecimalObj>::with_capacity(n)` with
// `v2_retain(&(*elem_ptr).header)` per stored element (the array owns one
// share per stored element); the result slot is
// `KindedSlot::new(ValueSlot::from_raw(ptr as u64), NativeKind::UInt64)`
// matching the v2-raw producer carrier shape.

/// Operation tag for the unified v2-raw String/Decimal set-op dispatcher.
#[derive(Copy, Clone, Eq, PartialEq)]
enum V2RawSetOp {
    /// `union(lhs, rhs)` — order-preserving dedup over concat lhs ++ rhs.
    Union,
    /// `intersect(lhs, rhs)` — elements in both, order over lhs, deduped.
    Intersect,
    /// `except(lhs, rhs)` — elements in lhs but not rhs, order over lhs, deduped.
    Except,
    /// `unique(arr)` / `distinct(arr)` — order-preserving dedup. No rhs.
    Unique,
}

/// Probe whether a `KindedSlot` is a v2-raw `*mut TypedArray<*const StringObj>`
/// (`V2ElemType::String`) or `*mut TypedArray<*const DecimalObj>`
/// (`V2ElemType::Decimal`) carrier. Returns the matched `V2ElemType` plus the
/// raw pointer bits + element count, or `None` for any other receiver shape.
///
/// **Unreachable at runtime under HEAD `5d0f1524`** — the producer gate stays
/// closed; documented as audit-deliverable for the A2-followup-gate-flip
/// sequential close. Compile-time correctness only.
#[inline]
fn as_v2_raw_string_decimal(
    slot: &KindedSlot,
) -> Option<(crate::executor::v2_handlers::v2_array_detect::V2ElemType, u64, u32)> {
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, V2ElemType};
    if slot.kind != NativeKind::UInt64 {
        return None;
    }
    let bits = slot.slot.raw();
    let view = as_v2_typed_array(bits, NativeKind::UInt64)?;
    match view.elem_type {
        V2ElemType::String | V2ElemType::Decimal => Some((view.elem_type, bits, view.len)),
        _ => None,
    }
}

/// v2-raw set-op dispatcher for `V2ElemType::String / Decimal` carriers.
///
/// Per audit §4.1.B.4: snapshots the receiver buffer(s) into local key vectors
/// (`Vec<&str>` for String, `Vec<Decimal>` for Decimal) for per-element-kind
/// equality (mirrors the heap-Arc helper contract — `Arc<String>` content
/// equality + `Decimal == Decimal` `PartialEq`); computes the kept-index
/// permutation per `op`; allocates a fresh `TypedArray<*const <X>Obj>` with
/// `v2_retain(&(*p).header)` on each stored element pointer; stamps the
/// elem_type byte; returns a `KindedSlot` of kind `NativeKind::UInt64`. The
/// source receivers are NOT consumed — the dispatch shell owns the source
/// shares; this helper only READS the source buffers.
///
/// **Unreachable at runtime under HEAD `5d0f1524`** per the gate-state comment
/// at the section head. The body is the consumer-surface contract for the
/// post-gate-flip lockstep close per A2-followup-gate-flip directive.
fn set_op_v2_raw_string_decimal(
    op: V2RawSetOp,
    lhs_args: &KindedSlot,
    rhs_args: Option<&KindedSlot>,
) -> Result<KindedSlot, VMError> {
    use crate::executor::v2_handlers::v2_array_detect::{
        stamp_elem_type, V2ElemType, ELEM_TYPE_DECIMAL, ELEM_TYPE_STRING,
    };
    use shape_value::v2::decimal_obj::DecimalObj;
    use shape_value::v2::refcount::v2_retain;
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::TypedArray;
    use shape_value::ValueSlot;

    let (lhs_elem, lhs_bits, lhs_len) = as_v2_raw_string_decimal(lhs_args)
        .ok_or_else(|| type_error("set op v2-raw: lhs not a v2-raw String/Decimal array"))?;
    let rhs_triple = match rhs_args {
        Some(rhs) => {
            let (re, rb, rl) = as_v2_raw_string_decimal(rhs)
                .ok_or_else(|| type_error("set op v2-raw: rhs not a v2-raw String/Decimal array"))?;
            if re != lhs_elem {
                return Err(type_error(
                    "set op v2-raw: lhs/rhs element-type mismatch (String vs Decimal)",
                ));
            }
            Some((rb, rl))
        }
        None => None,
    };

    // Generic kept-index builder. Given lhs / rhs key vectors of equal-kind
    // values that support `PartialEq`, returns `(lhs_keep, rhs_keep)` per the
    // operation. Mirrors `already_seen` / `lhs_in_rhs` / `unique_indices` in
    // the heap-Arc helpers above.
    fn build_keep<K: PartialEq>(
        op: V2RawSetOp,
        lhs: &[K],
        rhs: Option<&[K]>,
    ) -> (Vec<u32>, Vec<u32>) {
        let mut lhs_keep: Vec<u32> = Vec::new();
        let mut rhs_keep: Vec<u32> = Vec::new();
        let lhs_dedup = |lhs_keep: &mut Vec<u32>, i: usize| {
            if !lhs_keep.iter().any(|&j| lhs[j as usize] == lhs[i]) {
                lhs_keep.push(i as u32);
            }
        };
        match op {
            V2RawSetOp::Unique => {
                for i in 0..lhs.len() {
                    lhs_dedup(&mut lhs_keep, i);
                }
            }
            V2RawSetOp::Intersect => {
                let rhs = rhs.expect("intersect requires rhs");
                for i in 0..lhs.len() {
                    if rhs.iter().any(|x| *x == lhs[i]) {
                        lhs_dedup(&mut lhs_keep, i);
                    }
                }
            }
            V2RawSetOp::Except => {
                let rhs = rhs.expect("except requires rhs");
                for i in 0..lhs.len() {
                    if !rhs.iter().any(|x| *x == lhs[i]) {
                        lhs_dedup(&mut lhs_keep, i);
                    }
                }
            }
            V2RawSetOp::Union => {
                let rhs = rhs.expect("union requires rhs");
                for i in 0..lhs.len() {
                    lhs_dedup(&mut lhs_keep, i);
                }
                for i in 0..rhs.len() {
                    if lhs_keep.iter().any(|&j| lhs[j as usize] == rhs[i]) {
                        continue;
                    }
                    if !rhs_keep.iter().any(|&j| rhs[j as usize] == rhs[i]) {
                        rhs_keep.push(i as u32);
                    }
                }
            }
        }
        (lhs_keep, rhs_keep)
    }

    // Allocate the output, retain per stored element, return the v2-raw slot.
    // The elem_type byte at offset 7 of the HeapHeader is stamped per the
    // producer convention in `executor/v2_handlers/array.rs::OpCode::
    // NewTypedArrayString / NewTypedArrayDecimal`.
    let out_bits = match lhs_elem {
        V2ElemType::String => unsafe {
            let lhs_arr = lhs_bits as *const TypedArray<*const StringObj>;
            let lhs_keys: Vec<&str> = (0..lhs_len)
                .map(|i| StringObj::as_str(TypedArray::<*const StringObj>::get_unchecked(lhs_arr, i)))
                .collect();
            let rhs_keys_storage: Vec<&str> = match rhs_triple {
                Some((rb, rl)) => {
                    let rhs_arr = rb as *const TypedArray<*const StringObj>;
                    (0..rl)
                        .map(|i| StringObj::as_str(TypedArray::<*const StringObj>::get_unchecked(rhs_arr, i)))
                        .collect()
                }
                None => Vec::new(),
            };
            let rhs_keys_opt: Option<&[&str]> = rhs_triple.map(|_| rhs_keys_storage.as_slice());
            let (lhs_keep, rhs_keep) = build_keep(op, &lhs_keys, rhs_keys_opt);

            let total = lhs_keep.len() + rhs_keep.len();
            let out = TypedArray::<*const StringObj>::with_capacity(total as u32);
            stamp_elem_type(out as *mut u8, ELEM_TYPE_STRING);
            for &i in &lhs_keep {
                let p = TypedArray::<*const StringObj>::get_unchecked(lhs_arr, i);
                v2_retain(&(*p).header);
                TypedArray::<*const StringObj>::push(out, p);
            }
            if let Some((rb, _)) = rhs_triple {
                let rhs_arr = rb as *const TypedArray<*const StringObj>;
                for &i in &rhs_keep {
                    let p = TypedArray::<*const StringObj>::get_unchecked(rhs_arr, i);
                    v2_retain(&(*p).header);
                    TypedArray::<*const StringObj>::push(out, p);
                }
            }
            out as u64
        },
        V2ElemType::Decimal => unsafe {
            let lhs_arr = lhs_bits as *const TypedArray<*const DecimalObj>;
            let lhs_keys: Vec<rust_decimal::Decimal> = (0..lhs_len)
                .map(|i| DecimalObj::value(TypedArray::<*const DecimalObj>::get_unchecked(lhs_arr, i)))
                .collect();
            let rhs_keys_storage: Vec<rust_decimal::Decimal> = match rhs_triple {
                Some((rb, rl)) => {
                    let rhs_arr = rb as *const TypedArray<*const DecimalObj>;
                    (0..rl)
                        .map(|i| DecimalObj::value(TypedArray::<*const DecimalObj>::get_unchecked(rhs_arr, i)))
                        .collect()
                }
                None => Vec::new(),
            };
            let rhs_keys_opt: Option<&[rust_decimal::Decimal]> =
                rhs_triple.map(|_| rhs_keys_storage.as_slice());
            let (lhs_keep, rhs_keep) = build_keep(op, &lhs_keys, rhs_keys_opt);

            let total = lhs_keep.len() + rhs_keep.len();
            let out = TypedArray::<*const DecimalObj>::with_capacity(total as u32);
            stamp_elem_type(out as *mut u8, ELEM_TYPE_DECIMAL);
            for &i in &lhs_keep {
                let p = TypedArray::<*const DecimalObj>::get_unchecked(lhs_arr, i);
                v2_retain(&(*p).header);
                TypedArray::<*const DecimalObj>::push(out, p);
            }
            if let Some((rb, _)) = rhs_triple {
                let rhs_arr = rb as *const TypedArray<*const DecimalObj>;
                for &i in &rhs_keep {
                    let p = TypedArray::<*const DecimalObj>::get_unchecked(rhs_arr, i);
                    v2_retain(&(*p).header);
                    TypedArray::<*const DecimalObj>::push(out, p);
                }
            }
            out as u64
        },
        _ => unreachable!("as_v2_raw_string_decimal filtered other variants"),
    };
    Ok(KindedSlot::new(ValueSlot::from_raw(out_bits), NativeKind::UInt64))
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
    // Wave 2 Round 3a' Agent δ — v2-raw String/Decimal fast path. UNREACHABLE
    // under HEAD `5d0f1524`: gate `should_use_typed_array` returns None for
    // ConcreteType::String/Decimal. See section comment above.
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Union, &args[0], Some(&args[1]));
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
    // Wave 2 Round 3a' Agent δ — v2-raw String/Decimal fast path (UNREACHABLE
    // under closed producer gate; see section comment).
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Intersect, &args[0], Some(&args[1]));
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
    // Wave 2 Round 3a' Agent δ — v2-raw String/Decimal fast path (UNREACHABLE
    // under closed producer gate; see section comment).
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Except, &args[0], Some(&args[1]));
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
    // Wave 2 Round 3a' Agent δ — v2-raw String/Decimal fast path (UNREACHABLE
    // under closed producer gate; see section comment). `distinct` aliases
    // `unique` so it inherits this routing.
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Unique, &args[0], None);
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

/// Test whether two `KindedSlot` keys are equal under strict typing.
/// Heterogeneous kinds always compare unequal; same-kind comparison
/// dispatches on `NativeKind` per the §2.7.6 / Q8 heterogeneous-kind
/// body pattern (mirrors `cmp_key_kinded` in `array_sort.rs`).
fn key_eq(a: &KindedSlot, b: &KindedSlot) -> Result<bool, VMError> {
    if a.kind != b.kind {
        // Strict typing: no implicit promotion. Different-kind keys are
        // never equal (matches the IEEE 754 NaN != NaN convention used
        // elsewhere in the dedup pass — `already_seen`).
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
        // IEEE 754 semantics: NaN never compares equal — `==` on f64
        // matches the pre-Wave-6 set-op behaviour.
        NativeKind::Float64 => a.slot.as_f64() == b.slot.as_f64(),
        NativeKind::Bool => a.slot.as_bool() == b.slot.as_bool(),
        NativeKind::String => {
            let sa = a.as_str().unwrap_or("");
            let sb = b.as_str().unwrap_or("");
            sa == sb
        }
        other => {
            return Err(VMError::NotImplemented(format!(
                "distinctBy: key equality for kind {:?} — SURFACE: only \
                 inline-scalar / String key kinds dispatched in \
                 W17-array-closure-callback. Heap-typed keys (Decimal, \
                 BigInt, ...) need an ADR-006 §2.7.6 / Q8 per-kind \
                 equality table; Phase-2c reentry.",
                other
            )));
        }
    })
}

/// v2 `distinctBy` — deduplicate by a key function (order-preserving).
///
/// args: [array, key_fn]
///
/// W17-array-closure-callback: body filled now that `op_make_closure`
/// (W17-make-closure close `aa47364`) and `call_value_immediate_nb`
/// (W7 close `06cdfce`, ADR-006 §2.7.11 / Q12) are both live. Mirrors
/// `handle_unique_v2`'s order-preserving dedup, but the equality probe
/// runs on `key_fn(elem)` keys rather than on element identity. The
/// keys are computed up-front in a single closure-callback pass; the
/// dedup then walks the cached keys to build a kept-index permutation,
/// which `array_transform::project_indices` materializes against the
/// receiver to produce the result `TypedArrayData` of the same arm.
pub(crate) fn handle_distinct_by_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error(
            "distinctBy() requires 2 arguments (array, key_fn)",
        ));
    }
    // Receiver: accept both array carriers (heap-Arc + v2 raw-pointer)
    // via the shared helper. The returned Arc is an independent share —
    // safe to use across `call_value_immediate_nb`.
    let receiver_arc: Arc<TypedArrayData> = typed_array_arc_from_kinded(&args[0], "distinctBy")?;

    // Validate the closure callee kind. Same shape as
    // `array_query::closure_arg`; only Closure / function-ref accepted.
    let closure = match args[1].kind {
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => &args[1],
        other => {
            return Err(type_error(format!(
                "distinctBy: key function must be a closure or function ref, got kind {:?}",
                other
            )));
        }
    };

    let len = transform_typed_array_len(&receiver_arc);

    // Compute one key per element via the closure callback. Each call
    // bumps the closure share because the frame teardown's
    // `drop_with_kind(closure_heap_bits, ...)` would otherwise consume
    // the dispatch-shell-owned share, leaving the carrier dangling on
    // subsequent iterations. Same pattern as `array_sort.rs::sort_by_key_fn`.
    let mut keys: Vec<KindedSlot> = Vec::with_capacity(len);
    for i in 0..len {
        let elem = transform_element_kinded(&receiver_arc, i)?;
        bump_closure_share(closure);
        let key = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        keys.push(key);
    }

    // Order-preserving dedup: walk indices, keep the first occurrence
    // of each key. O(n^2) on the kept-set size — matches the existing
    // `unique_indices` pattern; a hash-keyed variant would require a
    // per-NativeKind hasher matrix and is deferred to the
    // §2.7.6/Q8 follow-up.
    let mut keep_idxs: Vec<usize> = Vec::with_capacity(len);
    for i in 0..len {
        let mut already = false;
        for &j in keep_idxs.iter() {
            if key_eq(&keys[i], &keys[j])? {
                already = true;
                break;
            }
        }
        if !already {
            keep_idxs.push(i);
        }
    }

    let out = transform_project_indices(&receiver_arc, &keep_idxs)?;
    Ok(KindedSlot::from_typed_array(out))
}
