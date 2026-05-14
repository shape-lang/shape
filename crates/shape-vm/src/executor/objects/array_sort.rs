//! Array sort operations
//!
//! Handles: order_by, then_by, join_str
//!
//! ## Wave-δ `MR-array-sort-sets-joins` body migration (ADR-006 §2.7.10 / Q11)
//!
//! Receiver enters as
//! `args[0]: KindedSlot { kind: NativeKind::Ptr(HeapKind::TypedArray) }`;
//! payload recovery via ADR-005 §1 single-discriminator dispatch through
//! `args[0].slot.as_heap_value()` + `HeapValue::TypedArray(arc)` match
//! (Wave 5b precedent in `executor/builtins/array_ops.rs`).
//!
//! `joinStr` does not require a closure callback: it iterates a typed
//! array element-by-element, stringifies per-arm, and concatenates with a
//! separator pulled from `args[1]: KindedSlot { kind: NativeKind::String }`.
//! Body migrated.
//!
//! ## W17-array-closure-callback (Phase 2d Wave 2, 2026-05-11)
//!
//! `orderBy` / `thenBy` bodies migrated off `NotImplemented(SURFACE)`
//! now that the kinded value-call path (`call_value_immediate_nb` in
//! `call_convention.rs:767`, ADR-006 §2.7.11 / Q12) and `op_make_closure`
//! (`control_flow/mod.rs:447`, W17-make-closure close at `aa47364`) are
//! both live. Both methods treat `args[1]` as a *key function* (not a
//! comparator), invoking `keyFn(elem)` per element to produce sort keys
//! that are then ordered via per-NativeKind comparison; canonical
//! reference: `handle_sort_v2`'s closure-driven comparator path in
//! `array_transform.rs` (the comparator-form sort is the existing
//! template — the key-fn form here issues one closure call per element
//! up-front, then sorts the index permutation by comparing the cached
//! keys). The `direction` parameter (an optional `"asc"` / `"desc"`
//! string) flips the comparator. `slice::sort_by` keys.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::{
    HeapKind, HeapValue, KindedSlot, NativeKind, TypedArrayData, VMError,
};
use std::sync::Arc;

use crate::executor::objects::array_transform::{
    bump_closure_share, element_kinded as transform_element_kinded,
    project_indices as transform_project_indices, typed_array_arc_from_kinded,
    typed_array_len as transform_typed_array_len,
};

// Wave 2 Round 3a' Agent epsilon (2026-05-14) — Round 3a' per-handler-family
// split of A2-followup-mechanical. Imports for the v2-raw String/Decimal
// fast-path in `handle_join_str_v2` + surface-and-stop guards in
// `handle_order_by_v2` / `handle_then_by_v2`. UNREACHABLE at this commit
// (producer gate `should_use_typed_array` in `compiler/typed_emission.rs`
// returns None for ConcreteType::String/Decimal — no producer constructs
// a v2-raw `TypedArray<*const StringObj/DecimalObj>` and therefore no
// receiver reaches these branches). Cargo-check-clean per supervisor
// 2026-05-14 disposition (1); gate-flip itself lands as a separate
// sequential A2-followup-gate-flip agent post-Round-3a'-merge ceremony.
use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, V2ElemType};
use shape_value::v2::decimal_obj::DecimalObj;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::TypedArray;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

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

/// Stringify a single element of `arr` at `idx` in the canonical
/// per-arm format. Mirrors the pre-Wave-6 join semantics: integer/float
/// formatting, bool as "true"/"false", string passthrough.
fn element_to_string(arr: &TypedArrayData, idx: usize, out: &mut String) -> Result<(), VMError> {
    use std::fmt::Write as _;
    match arr {
        TypedArrayData::I64(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::F64(buf) => {
            let v = buf.data[idx];
            // Match the Display contract used elsewhere in the runtime
            // (TypedArrayData::F64 in heap_value.rs:996): integer-valued
            // f64s render as integers when |v| < 1e15.
            if v == v.trunc() && v.abs() < 1e15 {
                write!(out, "{}", v as i64).map_err(|e| type_error(e.to_string()))
            } else {
                write!(out, "{}", v).map_err(|e| type_error(e.to_string()))
            }
        }
        TypedArrayData::Bool(buf) => {
            let b = buf.data[idx] != 0;
            write!(out, "{}", b).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::I8(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::I16(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::I32(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U8(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U16(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U32(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U64(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::F32(buf) => {
            let v = buf.data[idx];
            if v == v.trunc() && v.abs() < 1e15 {
                write!(out, "{}", v as i64).map_err(|e| type_error(e.to_string()))
            } else {
                write!(out, "{}", v).map_err(|e| type_error(e.to_string()))
            }
        }
        TypedArrayData::String(buf) => {
            out.push_str(buf.data[idx].as_str());
            Ok(())
        }
        // ADR-006 §2.7.22 amendment (Round 18 S3): Matrix / FloatSlice
        // exit `TypedArrayData`. Their joinStr lives on the new HeapKind
        // method registries when needed.
        // W17-typed-carrier-bundle-A checkpoint 3/4: Q25.A specialized arms.
        TypedArrayData::Decimal(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::BigInt(buf) => {
            write!(out, "{}", *buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::Char(buf) => {
            out.push(buf.data[idx]);
            Ok(())
        }
        TypedArrayData::TypedObject(_) => Err(type_error(format!(
            "joinStr: {} elements need per-schema stringification — out of joinStr \
             scope; use .map(|x| x.toString()).join() form",
            arr.type_name()
        ))),
    }
}

fn array_len(arr: &TypedArrayData) -> Result<usize, VMError> {
    Ok(match arr {
        TypedArrayData::I64(b) => b.len(),
        TypedArrayData::F64(b) => b.len(),
        TypedArrayData::Bool(b) => b.len(),
        TypedArrayData::I8(b) => b.len(),
        TypedArrayData::I16(b) => b.len(),
        TypedArrayData::I32(b) => b.len(),
        TypedArrayData::U8(b) => b.len(),
        TypedArrayData::U16(b) => b.len(),
        TypedArrayData::U32(b) => b.len(),
        TypedArrayData::U64(b) => b.len(),
        TypedArrayData::F32(b) => b.len(),
        TypedArrayData::String(b) => b.len(),
        // ADR-006 §2.7.22 amendment (Round 18 S3): Matrix / FloatSlice
        // exit `TypedArrayData`.
        // W17-typed-carrier-bundle-A checkpoint 3/4: Q25.A specialized arms.
        TypedArrayData::Decimal(b) => b.len(),
        TypedArrayData::BigInt(b) => b.len(),
        TypedArrayData::Char(b) => b.len(),
        TypedArrayData::TypedObject(b) => b.len(),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Recover the `Arc<TypedArrayData>` payload from `args[0]`. Accepts
/// both array carriers (heap-Arc + v2 raw-pointer) via the shared
/// `array_transform::typed_array_arc_from_kinded` helper.
fn receiver_arc_clone(slot: &KindedSlot, op: &str) -> Result<Arc<TypedArrayData>, VMError> {
    typed_array_arc_from_kinded(slot, op)
}

/// Parse the optional `direction` argument from `args[2..]`. Accepts
/// the canonical `"asc"` / `"desc"` strings; missing argument defaults
/// to ascending. Anything else is a `RuntimeError`.
fn parse_direction(args: &[KindedSlot], op: &str) -> Result<SortDirection, VMError> {
    if args.len() <= 2 {
        return Ok(SortDirection::Ascending);
    }
    match args[2].kind {
        NativeKind::String => match args[2].as_str() {
            Some("asc") | Some("ascending") => Ok(SortDirection::Ascending),
            Some("desc") | Some("descending") => Ok(SortDirection::Descending),
            Some(other) => Err(VMError::RuntimeError(format!(
                "{}: direction must be \"asc\" or \"desc\", got {:?}",
                op, other
            ))),
            None => Err(VMError::RuntimeError(format!(
                "{}: direction slot kind=String but bits empty",
                op
            ))),
        },
        other => Err(VMError::RuntimeError(format!(
            "{}: direction must be a string (\"asc\" or \"desc\"), got kind {:?}",
            op, other
        ))),
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum SortDirection {
    Ascending,
    Descending,
}

/// Wave 2 Round 3a' Agent epsilon (2026-05-14) — surface-and-stop guard
/// for v2-raw String/Decimal receivers entering a closure-callback sort
/// (`orderBy` / `thenBy`). UNREACHABLE at this commit (producer gate
/// `should_use_typed_array` returns None for ConcreteType::String/Decimal
/// per ADR-006 §2.7.24 Q25.A SUPERSEDED #3 mixed-migration forbidden).
/// The v2-raw key-fn sort recipe (read element with `v2_retain`, push as
/// `KindedSlot{kind: StringV2/DecimalV2}`, call keyFn, sort indices,
/// allocate `TypedArray<*const StringObj/DecimalObj>` output with
/// per-stored-elem `v2_retain`) is W17-array-closure-callback-v2-raw
/// territory — out of Round 3a' ε per-handler-family scope. The non-
/// closure direct-read path lives in `handle_join_str_v2` (in scope).
#[inline]
fn surface_and_stop_v2_raw_closure(slot: &KindedSlot, op: &'static str) -> Result<(), VMError> {
    if let Some(view) = as_v2_typed_array(slot.slot.raw(), slot.kind) {
        if matches!(view.elem_type, V2ElemType::String | V2ElemType::Decimal) {
            return Err(VMError::NotImplemented(format!(
                "{}: SURFACE — v2-raw Array<{}> (NativeKind::StringV2/DecimalV2) \
                 closure-callback sort reached the per-handler entry-point before \
                 W17-array-closure-callback-v2-raw lands the keyFn ABI extension. \
                 Per Round 3a' ε per-handler-family scope: the closure-callback \
                 path is out of scope; `handle_join_str_v2` covers the non-closure \
                 direct-read path. Tracked as W17-array-closure-callback-v2-raw \
                 per ADR-006 §2.7.11 / Q12 + §2.7.24 Q25.A SUPERSEDED.",
                op,
                if matches!(view.elem_type, V2ElemType::String) {
                    "string"
                } else {
                    "decimal"
                }
            )));
        }
    }
    Ok(())
}

/// Compare two `KindedSlot` keys produced by `keyFn(elem)`. Both keys
/// must share the same `NativeKind`; mismatched kinds surface as a
/// `RuntimeError` (no implicit coercion per CLAUDE.md "No runtime
/// coercion"). Float comparison uses `total_cmp` for NaN-safety; Bool
/// orders false-before-true; String orders lexically.
fn cmp_key_kinded(a: &KindedSlot, b: &KindedSlot, op: &str) -> Result<std::cmp::Ordering, VMError> {
    if a.kind != b.kind {
        return Err(VMError::RuntimeError(format!(
            "{}: key function produced heterogeneous result kinds {:?} vs {:?} \
             (CLAUDE.md \"No runtime coercion\" — keys must be monomorphic)",
            op, a.kind, b.kind
        )));
    }
    Ok(match a.kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => (a.slot.raw() as i64).cmp(&(b.slot.raw() as i64)),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => a.slot.raw().cmp(&b.slot.raw()),
        NativeKind::Float64 => a.slot.as_f64().total_cmp(&b.slot.as_f64()),
        NativeKind::Bool => a.slot.as_bool().cmp(&b.slot.as_bool()),
        NativeKind::String => {
            let sa = a.as_str().unwrap_or("");
            let sb = b.as_str().unwrap_or("");
            sa.cmp(sb)
        }
        other => {
            return Err(VMError::NotImplemented(format!(
                "{}: comparison of key kind {:?} — SURFACE: only inline-scalar / \
                 String key kinds dispatched in W17-array-closure-callback. \
                 Heap-typed keys (Decimal, BigInt, ...) need an ADR-006 §2.7.6 / Q8 \
                 per-kind comparator table; Phase-2c reentry.",
                op, other
            )));
        }
    })
}

/// Read closure callee at `args[1]` and validate it carries a closure
/// kind. Returns the borrowed slot for the call site.
fn closure_arg<'a>(args: &'a [KindedSlot], op: &'static str) -> Result<&'a KindedSlot, VMError> {
    let Some(slot) = args.get(1) else {
        return Err(VMError::RuntimeError(format!(
            "{}: missing key function argument",
            op
        )));
    };
    match slot.kind {
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => Ok(slot),
        other => Err(VMError::RuntimeError(format!(
            "{}: key function must be a closure or function ref, got kind {:?}",
            op, other
        ))),
    }
}

/// Sort `receiver_arc` by `keyFn(elem)` and return the sorted typed
/// array. The shared body for both `orderBy` (primary sort) and
/// `thenBy` (secondary sort — the input array is already partially
/// ordered, and `slice::sort_by` is stable, so a single pass with the
/// secondary key produces the lexicographic primary→secondary order).
fn sort_by_key_fn(
    vm: &mut VirtualMachine,
    receiver_arc: &Arc<TypedArrayData>,
    closure: &KindedSlot,
    direction: SortDirection,
    mut ctx: Option<&mut ExecutionContext>,
    op: &'static str,
) -> Result<Arc<TypedArrayData>, VMError> {
    let len = transform_typed_array_len(receiver_arc);

    // Compute keys up front — one closure call per element. This
    // separates closure invocation (which requires `&mut vm`) from the
    // sort comparator (which would otherwise need mutable VM access
    // during `slice::sort_by` and complicate error propagation).
    let mut keys: Vec<KindedSlot> = Vec::with_capacity(len);
    for i in 0..len {
        let elem = transform_element_kinded(receiver_arc, i)?;
        bump_closure_share(closure);
        let key = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        keys.push(key);
    }

    // Sort an index permutation by comparing cached keys. `sort_by`
    // cannot return errors; capture the first comparison failure via a
    // sticky shadow and short-circuit the rest by returning
    // `Ordering::Equal` (matching `handle_sort_v2`'s precedent in
    // `array_transform.rs`).
    let mut idx: Vec<usize> = (0..len).collect();
    let mut cmp_err: Option<VMError> = None;
    idx.sort_by(|&a, &b| {
        if cmp_err.is_some() {
            return std::cmp::Ordering::Equal;
        }
        let order = match cmp_key_kinded(&keys[a], &keys[b], op) {
            Ok(o) => o,
            Err(e) => {
                cmp_err = Some(e);
                return std::cmp::Ordering::Equal;
            }
        };
        match direction {
            SortDirection::Ascending => order,
            SortDirection::Descending => order.reverse(),
        }
    });
    if let Some(e) = cmp_err {
        return Err(e);
    }

    transform_project_indices(receiver_arc, &idx)
}

/// v2 `orderBy` — sort an array by a key function (optionally with direction).
///
/// args: [array, key_fn, direction?]
///
/// W17-array-closure-callback: body filled now that `op_make_closure`
/// (W17-make-closure close `aa47364`) and `call_value_immediate_nb` (W7
/// close `06cdfce`, ADR-006 §2.7.11 / Q12) are both live. The key
/// function is invoked once per element to produce a sort key; the
/// resulting permutation is materialized via
/// `array_transform::project_indices`. Direction defaults to ascending.
pub(crate) fn handle_order_by_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error(
            "orderBy: expected (array, key_fn, direction?)",
        ));
    }
    // Wave 2 Round 3a' Agent epsilon (2026-05-14) — v2-raw String/Decimal
    // SURFACE-AND-STOP guard. UNREACHABLE at this commit (producer gate
    // closed). The closure-callback path on `NativeKind::StringV2` /
    // `NativeKind::DecimalV2` element kinds is W17-array-closure-callback-
    // v2-raw territory (not Round 3a' ε per-handler-family scope). The
    // v2-raw output-shape (`TypedArray<*const StringObj/DecimalObj>`)
    // allocation pattern lands in that sub-cluster alongside the keyFn
    // ABI extension. See `handle_join_str_v2` for the v2-raw direct-read
    // fast-path landed in this commit (no closure, in scope).
    surface_and_stop_v2_raw_closure(&args[0], "orderBy")?;
    let closure = closure_arg(args, "orderBy")?;
    let direction = parse_direction(args, "orderBy")?;
    let receiver_arc = receiver_arc_clone(&args[0], "orderBy")?;
    let out = sort_by_key_fn(vm, &receiver_arc, closure, direction, ctx, "orderBy")?;
    Ok(KindedSlot::from_typed_array(out))
}

/// v2 `thenBy` — sort an already-ordered array by a secondary key.
///
/// args: [array, key_fn, direction?]
///
/// Shares the body shape with `orderBy`: `slice::sort_by` is stable, so
/// re-sorting the (assumed-already-primary-sorted) input by the
/// secondary key produces the lexicographic primary→secondary order.
/// Callers chaining `orderBy(...).thenBy(...)` get the expected
/// multi-key sort semantics.
pub(crate) fn handle_then_by_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error("thenBy: expected (array, key_fn, direction?)"));
    }
    // Wave 2 Round 3a' Agent epsilon (2026-05-14) — v2-raw String/Decimal
    // SURFACE-AND-STOP guard (mirror of `handle_order_by_v2`).
    surface_and_stop_v2_raw_closure(&args[0], "thenBy")?;
    let closure = closure_arg(args, "thenBy")?;
    let direction = parse_direction(args, "thenBy")?;
    let receiver_arc = receiver_arc_clone(&args[0], "thenBy")?;
    let out = sort_by_key_fn(vm, &receiver_arc, closure, direction, ctx, "thenBy")?;
    Ok(KindedSlot::from_typed_array(out))
}

/// v2 `joinStr` — join array elements into a single string with a separator.
///
/// args: [array, separator]
///
/// Receiver kind = `NativeKind::Ptr(HeapKind::TypedArray)`; separator kind
/// = `NativeKind::String`. Element stringification dispatches on the
/// `TypedArrayData::*` arm (no closure callback). The result is a fresh
/// `Arc<String>` carried as `KindedSlot::from_string_arc`.
pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "joinStr() requires 2 arguments (array, separator)",
        ));
    }

    let sep: &str = match args[1].kind {
        NativeKind::String => args[1]
            .as_str()
            .ok_or_else(|| type_error("joinStr(): separator slot kind=String but bits empty"))?,
        _ => {
            return Err(type_error(format!(
                "joinStr(): separator must be a string, got {:?}",
                args[1].kind
            )));
        }
    };

    // Wave 2 Round 3a' Agent epsilon (2026-05-14) — v2-raw String/Decimal
    // fast-path. UNREACHABLE at this commit (producer gate closed). Per
    // audit §4.1.B.4: read each element pointer directly from the v2-raw
    // `TypedArray<*const StringObj/DecimalObj>` data buffer and stringify
    // in place — NO materialize-on-read into Arc<TypedArrayData::String/
    // Decimal> (forbidden per §4.1.B.3). Receiver slot is owned by the
    // caller's KindedSlot (no refcount-share consumption needed; we only
    // READ, the slot's own share keeps element pointers alive).
    if let Some(view) = as_v2_typed_array(args[0].slot.raw(), args[0].kind) {
        match view.elem_type {
            V2ElemType::String => {
                let arr = view.ptr as *const TypedArray<*const StringObj>;
                let mut out = String::new();
                for i in 0..view.len {
                    if i > 0 {
                        out.push_str(sep);
                    }
                    let elem = unsafe { TypedArray::<*const StringObj>::get_unchecked(arr, i) };
                    out.push_str(unsafe { StringObj::as_str(elem) });
                }
                return Ok(KindedSlot::from_string_arc(Arc::new(out)));
            }
            V2ElemType::Decimal => {
                use std::fmt::Write as _;
                let arr = view.ptr as *const TypedArray<*const DecimalObj>;
                let mut out = String::new();
                for i in 0..view.len {
                    if i > 0 {
                        out.push_str(sep);
                    }
                    let elem = unsafe { TypedArray::<*const DecimalObj>::get_unchecked(arr, i) };
                    write!(out, "{}", unsafe { DecimalObj::value(elem) })
                        .map_err(|e| type_error(e.to_string()))?;
                }
                return Ok(KindedSlot::from_string_arc(Arc::new(out)));
            }
            _ => {}
        }
    }

    let arc = as_typed_array(&args[0])
        .ok_or_else(|| type_error("joinStr(): receiver must be an Array"))?;
    let arr = arc.as_ref();

    let len = array_len(arr)?;
    let mut out = String::new();
    for i in 0..len {
        if i > 0 {
            out.push_str(sep);
        }
        element_to_string(arr, i, &mut out)?;
    }
    Ok(KindedSlot::from_string_arc(Arc::new(out)))
}

// Tests intentionally not added in this file: handler tests need a
// minimal `VirtualMachine` instance and the dispatch shell
// (`op_call_method`) is itself a §2.7.10 SURFACE pending the
// receiver-classification cascade. Test coverage for `joinStr` lands
// alongside the dispatch-shell rebuild via the same harness pattern as
// `executor/builtins/array_ops.rs::tests` (Wave 5b body migrations).
