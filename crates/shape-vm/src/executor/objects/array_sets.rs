//! Array set operations
//!
//! Handles: union, intersect, except, unique, distinct, distinct_by
//!
//! ## V3-S5 ckpt-2 consumer-cascade surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's
//! `Arc<TypedArrayData>` receiver-recovery (`as_typed_array`) +
//! per-variant set-op helpers (`empty_like / already_seen / lhs_in_rhs /
//! build_from_indices / unique_indices / rhs_pair_eq`) all cascade-break
//! at the enum deletion site.
//!
//! Heap-Arc public handler bodies are replaced with structured
//! surface-and-stop returning `VMError::NotImplemented`. The
//! `set_op_v2_raw_string_decimal` v2-raw dispatcher PRESERVED intact
//! (operates on `TypedArray<*const StringObj/DecimalObj>` directly via
//! `v2_array_detect::as_v2_typed_array` + `StringObj::as_str` /
//! `DecimalObj::value` + `TypedArray::with_capacity / push` —
//! independent of `TypedArrayData`); the Wave 2 Agent δ pre-gate-flip
//! arm at each public handler stays wired so the A2-followup-gate-flip
//! lockstep close picks up a structurally-complete v2-raw producer +
//! consumer pair.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X` arm
//! in the heap-Arc helpers (`empty_like / already_seen / lhs_in_rhs /
//! build_from_indices / unique_indices`) migrates to the v2-raw
//! `TypedArray<T>` flat-struct carrier per-T monomorphization. Per
//! audit §3.2 / §3.3 the heap-element variant arms (String / Decimal /
//! BigInt / DateTime / Timespan / Duration / Instant / TypedObject /
//! TraitObject) need carrier `<X>Obj` structs landed first; the v2-raw
//! String/Decimal path here already operates on the canonical
//! `StringObj` / `DecimalObj` carriers per V3-A2-followup-producer-
//! cascade landing.
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::{HeapKind, KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-2 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Common surface-and-stop body for the heap-Arc `Ptr(HeapKind::TypedArray)`
/// arm of every public handler in this file. The v2-raw `UInt64` arm
/// remains wired through `set_op_v2_raw_string_decimal` per Wave 2 Agent δ
/// (unaffected by `TypedArrayData` deletion).
#[cold]
#[inline(never)]
fn ckpt2_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface for \
         the heap-Arc `Ptr(HeapKind::TypedArray)` set-op arm. \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The previous `Arc<TypedArrayData>` receiver-recovery \
         (`as_typed_array`) + per-variant equality helpers \
         (`already_seen / lhs_in_rhs / build_from_indices / \
         unique_indices`) cascade-broke at the enum deletion site \
         (`crates/shape-value/src/heap_value.rs:3944`). Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 heap-element \
         variants; per-T monomorphization landing across ckpt-3 \
         (array_ops/typed_array_methods/iterator_methods/array_sort/\
         concat/property_access) + ckpt-4 (TypedBuffer<T> / \
         HeapValue::TypedArray arm / HeapKind::TypedArray ordinal) + \
         ckpt-5 (wire/json/marshal + 4-table lockstep) + ckpt-6 (JIT \
         FFI). The v2-raw `TypedArray<*const StringObj/DecimalObj>` \
         path (Wave 2 Agent δ, `set_op_v2_raw_string_decimal`) remains \
         live and is independent of `TypedArrayData` — invokable post-\
         A2-followup-gate-flip via the per-handler fast-path. Receiver \
         kind: {kind}. UNREACHABLE until ckpt-6 STRICT close. REFUSED \
         ON SIGHT: TypedArrayData resurrection under any rename \
         (Refusal #1, W12 audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Wave 2 Round 3a' Agent δ — v2-raw String/Decimal handler arms.
// PRESERVED through V3-S5 ckpt-2: independent of `TypedArrayData`.
//
// Per supervisor 2026-05-14 disposition (1): the v2-raw `TypedArray<*const
// StringObj/DecimalObj>` consumer surface lands as UNREACHABLE under the
// closed producer gate (`should_use_typed_array(ConcreteType::String/Decimal)
// → None`); the A2-followup-gate-flip ceremony post-Round 3a' merge flips
// the gate atomically and these arms become reachable in one commit.
//
// Shape (audit §4.1.B.4): receiver's v2-raw `TypedArray<*const
// StringObj/DecimalObj>` walked directly; equality via `StringObj::as_str` /
// `DecimalObj::value` content comparison; result is a fresh `TypedArray::
// <*const <X>Obj>::with_capacity(n)` with `v2_retain(&(*p).header)` per
// stored element; result slot is `KindedSlot::new(ValueSlot::from_raw(ptr
// as u64), NativeKind::UInt64)` per the v2-raw producer carrier shape.
// ═══════════════════════════════════════════════════════════════════════════

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
    // operation.
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
// MethodFnV2 (native ABI) public handlers — ckpt-2 surface-and-stop stubs
// Signatures preserved for `method_registry.rs` PHF integrity. Heap-Arc
// receiver arm surface-and-stops; v2-raw String/Decimal arm reachable via
// `set_op_v2_raw_string_decimal` post-A2-followup-gate-flip.
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `union` — set union of two arrays (deduplicated, order-preserving).
pub(crate) fn handle_union_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("union() requires 2 arguments (array, other)"));
    }
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Union, &args[0], Some(&args[1]));
    }
    Err(ckpt2_surface("union", args))
}

/// v2 `intersect` — set intersection of two arrays.
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
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Intersect, &args[0], Some(&args[1]));
    }
    Err(ckpt2_surface("intersect", args))
}

/// v2 `except` — set difference of two arrays.
pub(crate) fn handle_except_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("except() requires 2 arguments (array, other)"));
    }
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Except, &args[0], Some(&args[1]));
    }
    Err(ckpt2_surface("except", args))
}

/// v2 `unique` — deduplicate array elements.
pub(crate) fn handle_unique_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("unique() requires 1 argument (array)"));
    }
    if as_v2_raw_string_decimal(&args[0]).is_some() {
        return set_op_v2_raw_string_decimal(V2RawSetOp::Unique, &args[0], None);
    }
    Err(ckpt2_surface("unique", args))
}

/// v2 `distinct` — alias for `unique`.
pub(crate) fn handle_distinct_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_unique_v2(vm, args, ctx)
}

/// v2 `distinctBy` — deduplicate by a key function.
///
/// The closure-callback dispatch shape (ADR-006 §2.7.11 / Q12) re-instates
/// once the receiver-shape migration lands at ckpt-6 STRICT close —
/// `vm.call_value_immediate_nb(key_fn, [elem], ctx)` itself is unaffected
/// by `TypedArrayData` deletion. Closure-arg shape validation preserved at
/// the entry-point.
pub(crate) fn handle_distinct_by_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error(
            "distinctBy() requires 2 arguments (array, key_fn)",
        ));
    }
    match args[1].kind {
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => {}
        other => {
            return Err(type_error(format!(
                "distinctBy: key function must be a closure or function ref, got kind {:?}",
                other
            )));
        }
    }
    Err(ckpt2_surface("distinctBy", args))
}
