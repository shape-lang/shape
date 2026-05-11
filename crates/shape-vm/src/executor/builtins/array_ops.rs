//! Native array builtin implementations (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5b body migration: `&[KindedSlot] -> Result<KindedSlot, VMError>`.
//! Heap dispatch goes through `slot.as_heap_value()` + `HeapValue` match
//! (ADR-005 §1 single-discriminator); no per-heap-variant accessors on
//! `KindedSlot`.
//!
//! `push`/`pop`/`first`/`last`/`zip`/`filled` operate on the typed-array
//! `HeapValue::TypedArray(Arc<TypedArrayData>)` shape. Each call site
//! discriminates on `TypedArrayData` arm to pick the right element shape;
//! a heterogeneous-element path uses `the-deleted-heterogeneous-element-carrier` as the
//! catch-all storage shape.

use shape_value::{
    HeapKind, HeapValue, KindedSlot, NativeKind, TypedArrayData, TypedBuffer, VMError,
};
use std::sync::Arc;

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Borrow the `Arc<TypedArrayData>` payload from a `KindedSlot` whose
/// `kind == NativeKind::Ptr(HeapKind::TypedArray)`. Returns `None` for any
/// other kind. Heap dispatch follows ADR-005 §1: project through
/// `slot.as_heap_value()` then pattern-match the `HeapValue::TypedArray`
/// arm; no per-heap-variant `KindedSlot` accessor.
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

/// Wrap a fresh `TypedArrayData` as a `KindedSlot`.
#[inline]
fn typed_array_to_slot(arr: TypedArrayData) -> KindedSlot {
    KindedSlot::from_typed_array(Arc::new(arr))
}

/// Convert a `KindedSlot` element to an `Arc<HeapValue>` for storage in
/// a `the-deleted-heterogeneous-element-carrier` buffer (catch-all heterogeneous element
/// shape). Inline scalars wrap into the matching `HeapValue` arm.
fn slot_to_heap_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    match slot.kind {
        NativeKind::Int64 => {
            // BigInt is the closest Heap arm for int — but the strict-typed
            // Int64 path is "store the bits as i64". Round-trip via
            // BigInt(Arc<i64>) preserves the integer value.
            let i = slot.as_i64().expect("kind=Int64");
            Ok(Arc::new(HeapValue::BigInt(Arc::new(i))))
        }
        NativeKind::Float64 => {
            // No HeapValue::Float arm — Float64 in a heap-element buffer
            // routes through Decimal as a stable representation. (TypedArray
            // for f64 is the F64 arm; this slot_to_heap_arc path is only for
            // truly heterogeneous arrays via the-deleted-heterogeneous-element-carrier.)
            Err(type_error(
                "array element of kind Float64 cannot be heap-wrapped (use Vec<number> instead)",
            ))
        }
        NativeKind::Bool => Err(type_error(
            "array element of kind Bool cannot be heap-wrapped (use Vec<bool> instead)",
        )),
        NativeKind::String => match slot.slot.as_heap_value() {
            HeapValue::String(s) => Ok(Arc::new(HeapValue::String(Arc::clone(s)))),
            _ => Err(type_error("KindedSlot kind=String but heap arm mismatched")),
        },
        NativeKind::Ptr(_) => {
            // Heap pointer: clone the Arc<HeapValue> by re-projecting through
            // as_heap_value(). The slot owns one strong-count share; we
            // clone to bump it.
            let hv: &HeapValue = slot.slot.as_heap_value();
            Ok(Arc::new(hv.clone()))
        }
        _ => Err(type_error(format!(
            "array element of kind {:?} cannot be stored in heterogeneous array",
            slot.kind
        ))),
    }
}

/// Project a `TypedArrayData` element at index `idx` to a `KindedSlot` of
/// the matching kind. Used by `first`/`last`. Returns `None` when out of
/// bounds.
fn typed_array_element(arr: &TypedArrayData, idx: usize) -> Option<KindedSlot> {
    match arr {
        TypedArrayData::I64(buf) => buf.data.get(idx).copied().map(KindedSlot::from_int),
        TypedArrayData::F64(buf) => buf.data.get(idx).copied().map(KindedSlot::from_number),
        TypedArrayData::Bool(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|b| KindedSlot::from_bool(b != 0)),
        TypedArrayData::I8(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::I16(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::I32(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::U8(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::U16(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::U32(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::U64(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_int(v as i64)),
        TypedArrayData::F32(buf) => buf
            .data
            .get(idx)
            .copied()
            .map(|v| KindedSlot::from_number(v as f64)),
        TypedArrayData::String(buf) => buf
            .data
            .get(idx)
            .map(|s| KindedSlot::from_string_arc(Arc::clone(s))),
        // W17-typed-carrier-bundle-A checkpoint 3/4: Q25.A specialized arms.
        TypedArrayData::Decimal(buf) => buf.data.get(idx).map(|d| KindedSlot::from_decimal(Arc::clone(d))),
        TypedArrayData::BigInt(buf) => buf.data.get(idx).map(|b| KindedSlot::from_bigint(Arc::clone(b))),
        TypedArrayData::DateTime(buf) | TypedArrayData::Timespan(buf) | TypedArrayData::Duration(buf) => {
            buf.data.get(idx).map(|td| {
                let bits = Arc::into_raw(Arc::clone(td)) as u64;
                KindedSlot::new(shape_value::ValueSlot::from_raw(bits), NativeKind::Ptr(HeapKind::Temporal))
            })
        }
        TypedArrayData::Instant(buf) => buf.data.get(idx).map(|inst| {
            let bits = Arc::into_raw(Arc::clone(inst)) as u64;
            KindedSlot::new(shape_value::ValueSlot::from_raw(bits), NativeKind::Ptr(HeapKind::Instant))
        }),
        TypedArrayData::Char(buf) => buf.data.get(idx).copied().map(KindedSlot::from_char),
        TypedArrayData::TypedObject(buf) => buf.data.get(idx).map(|o| KindedSlot::from_typed_object(Arc::clone(o))),
        TypedArrayData::TraitObject(buf) => buf.data.get(idx).map(|t| KindedSlot::from_trait_object(Arc::clone(t))),
        TypedArrayData::Matrix(_) | TypedArrayData::FloatSlice { .. } => None,
    }
}

/// Re-wrap an `Arc<HeapValue>` as a `KindedSlot` using the per-FieldType
/// constructor matching the arm. Used when reading elements out of a
/// `the-deleted-heterogeneous-element-carrier` buffer.
fn heap_value_to_slot(hv: &Arc<HeapValue>) -> KindedSlot {
    match hv.as_ref() {
        HeapValue::String(s) => KindedSlot::from_string_arc(Arc::clone(s)),
        HeapValue::Decimal(d) => KindedSlot::from_decimal(Arc::clone(d)),
        HeapValue::BigInt(b) => KindedSlot::from_bigint(Arc::clone(b)),
        HeapValue::TypedArray(a) => KindedSlot::from_typed_array(Arc::clone(a)),
        HeapValue::TypedObject(o) => KindedSlot::from_typed_object(Arc::clone(o)),
        HeapValue::HashMap(m) => KindedSlot::from_hashmap(Arc::clone(m)),
        HeapValue::Char(c) => KindedSlot::from_char(*c),
        // Other heap arms — fall back to none() for now; they're
        // tile-uncovered until Wave 5e wires the matching constructors.
        _ => KindedSlot::none(),
    }
}

// ── Body migrations ────────────────────────────────────────────────────────

pub(in crate::executor) fn builtin_push(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("push() requires 2 arguments (array, value)"));
    }
    let arr = as_typed_array(&args[0])
        .ok_or_else(|| type_error("push() first argument must be an array"))?;
    let value = &args[1];

    // Dispatch on array element shape to pick the matching `push`.
    let new_arr: TypedArrayData = match arr.as_ref() {
        TypedArrayData::I64(buf) => {
            let i = match value.kind {
                NativeKind::Int64 => value.as_i64().expect("kind=Int64"),
                _ => {
                    return Err(type_error(
                        "push() value kind must match array element kind (int)",
                    ));
                }
            };
            let mut new_data = buf.data.clone();
            new_data.push(i);
            TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(new_data)))
        }
        TypedArrayData::F64(buf) => {
            let n = match value.kind {
                NativeKind::Float64 => value.as_f64().expect("kind=Float64"),
                NativeKind::Int64 => value.as_i64().expect("kind=Int64") as f64,
                _ => {
                    return Err(type_error(
                        "push() value kind must match array element kind (number)",
                    ));
                }
            };
            let mut new_data = buf.iter().copied().collect::<Vec<f64>>();
            new_data.push(n);
            TypedArrayData::F64(Arc::new(
                shape_value::AlignedTypedBuffer::from(shape_value::AlignedVec::from_vec(new_data)),
            ))
        }
        TypedArrayData::Bool(buf) => {
            let b = match value.kind {
                NativeKind::Bool => value.as_bool().expect("kind=Bool"),
                _ => {
                    return Err(type_error(
                        "push() value kind must match array element kind (bool)",
                    ));
                }
            };
            let mut new_data = buf.data.clone();
            new_data.push(if b { 1u8 } else { 0u8 });
            TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(new_data)))
        }
        TypedArrayData::String(buf) => {
            let s = match value.kind {
                NativeKind::String => match value.slot.as_heap_value() {
                    HeapValue::String(s) => Arc::clone(s),
                    _ => return Err(type_error("KindedSlot kind=String but heap arm mismatched")),
                },
                _ => {
                    return Err(type_error(
                        "push() value kind must match array element kind (string)",
                    ));
                }
            };
            let mut new_data = buf.data.clone();
            new_data.push(s);
            TypedArrayData::String(Arc::new(TypedBuffer::from_vec(new_data)))
        }
        // Width-narrowed integer/float arms: not exposed to user-level push
        // today (compiler emits I64/F64 by default). Reject explicitly
        // rather than silently widening.
        _ => {
            return Err(type_error(
                "push() not supported for narrow-width or matrix arrays",
            ));
        }
    };
    Ok(typed_array_to_slot(new_arr))
}

pub(in crate::executor) fn builtin_pop(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("pop() requires 1 argument (array)"));
    }
    let arr = as_typed_array(&args[0])
        .ok_or_else(|| type_error("pop() argument must be an array"))?;

    let new_arr: TypedArrayData = match arr.as_ref() {
        TypedArrayData::I64(buf) => {
            let mut new_data = buf.data.clone();
            new_data.pop();
            TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(new_data)))
        }
        TypedArrayData::F64(buf) => {
            let mut new_data = buf.iter().copied().collect::<Vec<f64>>();
            new_data.pop();
            TypedArrayData::F64(Arc::new(
                shape_value::AlignedTypedBuffer::from(shape_value::AlignedVec::from_vec(new_data)),
            ))
        }
        TypedArrayData::Bool(buf) => {
            let mut new_data = buf.data.clone();
            new_data.pop();
            TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(new_data)))
        }
        TypedArrayData::String(buf) => {
            let mut new_data = buf.data.clone();
            new_data.pop();
            TypedArrayData::String(Arc::new(TypedBuffer::from_vec(new_data)))
        }
        _ => {
            return Err(type_error(
                "pop() not supported for narrow-width or matrix arrays",
            ));
        }
    };
    Ok(typed_array_to_slot(new_arr))
}

pub(in crate::executor) fn builtin_first(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("first() requires 1 argument"));
    }
    let arr = as_typed_array(&args[0])
        .ok_or_else(|| type_error("first() argument must be an array"))?;
    Ok(typed_array_element(arr.as_ref(), 0).unwrap_or_else(KindedSlot::none))
}

pub(in crate::executor) fn builtin_last(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("last() requires 1 argument"));
    }
    let arr = as_typed_array(&args[0])
        .ok_or_else(|| type_error("last() argument must be an array"))?;
    let len = match arr.as_ref() {
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
        // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A arms.
        TypedArrayData::Decimal(b) => b.len(),
        TypedArrayData::BigInt(b) => b.len(),
        TypedArrayData::DateTime(b) => b.len(),
        TypedArrayData::Timespan(b) => b.len(),
        TypedArrayData::Duration(b) => b.len(),
        TypedArrayData::Instant(b) => b.len(),
        TypedArrayData::Char(b) => b.len(),
        TypedArrayData::TypedObject(b) => b.len(),
        TypedArrayData::TraitObject(b) => b.len(),
        TypedArrayData::Matrix(_) | TypedArrayData::FloatSlice { .. } => 0,
    };
    if len == 0 {
        return Ok(KindedSlot::none());
    }
    Ok(typed_array_element(arr.as_ref(), len - 1).unwrap_or_else(KindedSlot::none))
}

/// `zip(a, b)` — pairs elements of two arrays into `Pair<A,B>` TypedObjects.
///
/// W17-typed-carrier-bundle-A checkpoint 2/4: per the C+ resolution
/// recorded in `phase-2d-playbook.md` §3 (Bundle-A checkpoint-2 amendment),
/// each pair is constructed as a TypedObject with fields `{first, second}`
/// rather than the prior 2-element `[a, b]` heterogeneous array. User code
/// reads `pair.first` / `pair.second` rather than `pair[0]` / `pair[1]` —
/// breaking change for stdlib + tests. Shape's tuple representation
/// lowers to TypedObject (`closure_layout.rs:843`) — no distinct tuple
/// runtime carrier; named fields are the right shape.
pub(in crate::executor) fn builtin_zip(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("zip() requires 2 arguments"));
    }
    let a_arc = as_typed_array(&args[0])
        .ok_or_else(|| type_error("zip() first argument must be an array"))?;
    let b_arc = as_typed_array(&args[1])
        .ok_or_else(|| type_error("zip() second argument must be an array"))?;
    let a = a_arc.as_ref();
    let b = b_arc.as_ref();

    let len_a = match a {
        TypedArrayData::I64(x) => x.len(),
        TypedArrayData::F64(x) => x.len(),
        TypedArrayData::Bool(x) => x.len(),
        TypedArrayData::String(x) => x.len(),
        _ => 0,
    };
    let len_b = match b {
        TypedArrayData::I64(x) => x.len(),
        TypedArrayData::F64(x) => x.len(),
        TypedArrayData::Bool(x) => x.len(),
        TypedArrayData::String(x) => x.len(),
        _ => 0,
    };
    let n = len_a.min(len_b);

    let mut pair_storages: Vec<Arc<shape_value::heap_value::TypedObjectStorage>> =
        Vec::with_capacity(n);
    for i in 0..n {
        let xa = typed_array_element(a, i).unwrap_or_else(KindedSlot::none);
        let xb = typed_array_element(b, i).unwrap_or_else(KindedSlot::none);
        let pair_slot = shape_runtime::type_schema::typed_object_from_pairs(&[
            ("first", xa),
            ("second", xb),
        ]);
        match pair_slot.slot.as_heap_value() {
            HeapValue::TypedObject(s) => pair_storages.push(Arc::clone(s)),
            other => {
                return Err(type_error(format!(
                    "zip: typed_object_from_pairs returned non-TypedObject: {:?}",
                    other.kind()
                )))
            }
        }
        drop(pair_slot);
    }
    Ok(KindedSlot::from_typed_array(Arc::new(
        TypedArrayData::TypedObject(Arc::new(TypedBuffer::from_vec(pair_storages))),
    )))
}

/// `Array.filled(size, value)` — produce an array of `size` repeats of `value`.
/// Element shape follows `value.kind`.
pub(in crate::executor) fn builtin_filled(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Array.filled() requires 2 arguments (size, value)"));
    }
    let size = args[0]
        .as_i64()
        .map(|i| i as usize)
        .or_else(|| args[0].as_f64().map(|f| f as usize))
        .ok_or_else(|| type_error("Array.filled() size must be a number"))?;
    let value = &args[1];

    let new_arr: TypedArrayData = match value.kind {
        NativeKind::Int64 => {
            let i = value.as_i64().expect("kind=Int64");
            TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(vec![i; size])))
        }
        NativeKind::Float64 => {
            let n = value.as_f64().expect("kind=Float64");
            let av = shape_value::AlignedVec::from_vec(vec![n; size]);
            TypedArrayData::F64(Arc::new(shape_value::AlignedTypedBuffer::from(av)))
        }
        NativeKind::Bool => {
            let b = value.as_bool().expect("kind=Bool");
            TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(vec![
                if b { 1u8 } else { 0u8 };
                size
            ])))
        }
        NativeKind::String => {
            let s = match value.slot.as_heap_value() {
                HeapValue::String(s) => Arc::clone(s),
                _ => return Err(type_error("KindedSlot kind=String but heap arm mismatched")),
            };
            TypedArrayData::String(Arc::new(TypedBuffer::from_vec(vec![s; size])))
        }
        // W17-typed-carrier-bundle-A checkpoint 2/4: heap-kinded value
        // dispatches to the specialized variant per §2.7.24 Q25.A. Each
        // arm holds a single Arc cloned `size` times — no HeapValue
        // catch-all carrier. The element kind is uniform per variant.
        NativeKind::Ptr(HeapKind::TypedObject) => {
            let storage = match value.slot.as_heap_value() {
                HeapValue::TypedObject(s) => Arc::clone(s),
                _ => return Err(type_error("KindedSlot kind=Ptr(TypedObject) heap arm mismatched")),
            };
            TypedArrayData::TypedObject(Arc::new(TypedBuffer::from_vec(vec![storage; size])))
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let d = match value.slot.as_heap_value() {
                HeapValue::Decimal(d) => Arc::clone(d),
                _ => return Err(type_error("KindedSlot kind=Ptr(Decimal) heap arm mismatched")),
            };
            TypedArrayData::Decimal(Arc::new(TypedBuffer::from_vec(vec![d; size])))
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            let b = match value.slot.as_heap_value() {
                HeapValue::BigInt(b) => Arc::clone(b),
                _ => return Err(type_error("KindedSlot kind=Ptr(BigInt) heap arm mismatched")),
            };
            TypedArrayData::BigInt(Arc::new(TypedBuffer::from_vec(vec![b; size])))
        }
        NativeKind::Ptr(HeapKind::Char) => {
            let c = value.slot.as_char().ok_or_else(|| {
                type_error("KindedSlot kind=Ptr(Char) but slot bits decode failed")
            })?;
            TypedArrayData::Char(Arc::new(TypedBuffer::from_vec(vec![c; size])))
        }
        other => {
            return Err(type_error(format!(
                "Array.filled() element kind {:?} not supported \
                 post-§2.7.24 Q25.A — add a specialized TypedArrayData arm.",
                other
            )))
        }
    };
    Ok(typed_array_to_slot(new_arr))
}

/// `range(n)` / `range(start, end)` / `range(start, end, step)` — produce
/// an `Array<int>` (when all args are Int) or `Array<number>` otherwise.
pub(in crate::executor) fn builtin_range(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    let all_int = !args.is_empty() && args.iter().all(|a| matches!(a.kind, NativeKind::Int64));
    if all_int {
        return builtin_range_int(args);
    }

    let (start, end, step) = match args.len() {
        1 => {
            let n = super::kind_coerce::coerce_to_f64(&args[0])
                .ok_or_else(|| type_error("range() argument must be a number"))?;
            (0.0, n, 1.0)
        }
        2 => {
            let s = super::kind_coerce::coerce_to_f64(&args[0])
                .ok_or_else(|| type_error("range() start must be a number"))?;
            let e = super::kind_coerce::coerce_to_f64(&args[1])
                .ok_or_else(|| type_error("range() end must be a number"))?;
            (s, e, 1.0)
        }
        3 => {
            let s = super::kind_coerce::coerce_to_f64(&args[0])
                .ok_or_else(|| type_error("range() start must be a number"))?;
            let e = super::kind_coerce::coerce_to_f64(&args[1])
                .ok_or_else(|| type_error("range() end must be a number"))?;
            let st = super::kind_coerce::coerce_to_f64(&args[2])
                .ok_or_else(|| type_error("range() step must be a number"))?;
            if st == 0.0 {
                return Err(type_error("range() step cannot be zero"));
            }
            (s, e, st)
        }
        _ => return Err(type_error("range() requires 1, 2, or 3 arguments")),
    };

    let mut values: Vec<f64> = Vec::new();
    if step > 0.0 {
        let mut current = start;
        while current < end {
            values.push(current);
            current += step;
        }
    } else {
        let mut current = start;
        while current > end {
            values.push(current);
            current += step;
        }
    }
    let av = shape_value::AlignedVec::from_vec(values);
    let buf = shape_value::AlignedTypedBuffer::from(av);
    Ok(typed_array_to_slot(TypedArrayData::F64(Arc::new(buf))))
}

fn builtin_range_int(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    let (start, end, step) = match args.len() {
        1 => (0i64, args[0].as_i64().unwrap(), 1i64),
        2 => (
            args[0].as_i64().unwrap(),
            args[1].as_i64().unwrap(),
            1i64,
        ),
        3 => {
            let st = args[2].as_i64().unwrap();
            if st == 0 {
                return Err(type_error("range() step cannot be zero"));
            }
            (args[0].as_i64().unwrap(), args[1].as_i64().unwrap(), st)
        }
        _ => return Err(type_error("range() requires 1, 2, or 3 arguments")),
    };

    let mut values: Vec<i64> = Vec::new();
    if step > 0 {
        let mut current = start;
        while current < end {
            values.push(current);
            current += step;
        }
    } else {
        let mut current = start;
        while current > end {
            values.push(current);
            current += step;
        }
    }
    Ok(typed_array_to_slot(TypedArrayData::I64(Arc::new(
        TypedBuffer::from_vec(values),
    ))))
}

/// `slice(arr, start, [end])` — return a subarray. Preserves element shape.
pub(in crate::executor) fn builtin_slice(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(type_error(
            "slice() requires 2 or 3 arguments (array, start, [end])",
        ));
    }
    let arr_arc = as_typed_array(&args[0])
        .ok_or_else(|| type_error("slice() first argument must be an array"))?;
    let arr = arr_arc.as_ref();

    let len = match arr {
        TypedArrayData::I64(x) => x.len(),
        TypedArrayData::F64(x) => x.len(),
        TypedArrayData::Bool(x) => x.len(),
        TypedArrayData::String(x) => x.len(),
        TypedArrayData::I8(x) => x.len(),
        TypedArrayData::I16(x) => x.len(),
        TypedArrayData::I32(x) => x.len(),
        TypedArrayData::U8(x) => x.len(),
        TypedArrayData::U16(x) => x.len(),
        TypedArrayData::U32(x) => x.len(),
        TypedArrayData::U64(x) => x.len(),
        TypedArrayData::F32(x) => x.len(),
        // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A arms.
        TypedArrayData::Decimal(x) => x.len(),
        TypedArrayData::BigInt(x) => x.len(),
        TypedArrayData::DateTime(x) => x.len(),
        TypedArrayData::Timespan(x) => x.len(),
        TypedArrayData::Duration(x) => x.len(),
        TypedArrayData::Instant(x) => x.len(),
        TypedArrayData::Char(x) => x.len(),
        TypedArrayData::TypedObject(x) => x.len(),
        TypedArrayData::TraitObject(x) => x.len(),
        TypedArrayData::Matrix(_) | TypedArrayData::FloatSlice { .. } => 0,
    } as isize;

    let start = super::kind_coerce::coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("slice() start must be a number"))? as isize;
    let end = if args.len() == 3 {
        super::kind_coerce::coerce_to_f64(&args[2])
            .ok_or_else(|| type_error("slice() end must be a number"))? as isize
    } else {
        len
    };

    let start_idx = if start < 0 {
        (len + start).max(0) as usize
    } else {
        start.min(len) as usize
    };
    let end_idx = if end < 0 {
        (len + end).max(0) as usize
    } else {
        end.min(len) as usize
    };

    let new_arr: TypedArrayData = if start_idx > end_idx {
        // Empty slice — produce an empty TypedArray of matching shape.
        match arr {
            TypedArrayData::I64(_) => {
                TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(Vec::new())))
            }
            TypedArrayData::F64(_) => TypedArrayData::F64(Arc::new(
                shape_value::AlignedTypedBuffer::from(shape_value::AlignedVec::from_vec(
                    Vec::<f64>::new(),
                )),
            )),
            TypedArrayData::Bool(_) => {
                TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(Vec::new())))
            }
            TypedArrayData::String(_) => {
                TypedArrayData::String(Arc::new(TypedBuffer::from_vec(Vec::new())))
            }
            _ => {
                return Err(type_error(
                    "slice() not supported for narrow-width or matrix arrays",
                ));
            }
        }
    } else {
        match arr {
            TypedArrayData::I64(buf) => TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(
                buf.data[start_idx..end_idx].to_vec(),
            ))),
            TypedArrayData::F64(buf) => {
                let av = shape_value::AlignedVec::from_vec(buf.data[start_idx..end_idx].to_vec());
                TypedArrayData::F64(Arc::new(shape_value::AlignedTypedBuffer::from(av)))
            }
            TypedArrayData::Bool(buf) => TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(
                buf.data[start_idx..end_idx].to_vec(),
            ))),
            TypedArrayData::String(buf) => TypedArrayData::String(Arc::new(
                TypedBuffer::from_vec(buf.data[start_idx..end_idx].to_vec()),
            )),
            _ => {
                return Err(type_error(
                    "slice() not supported for narrow-width or matrix arrays",
                ));
            }
        }
    };
    Ok(typed_array_to_slot(new_arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_array(v: Vec<i64>) -> KindedSlot {
        typed_array_to_slot(TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(v))))
    }

    #[test]
    fn push_int_array_grows() {
        let arr = int_array(vec![1, 2, 3]);
        let r = builtin_push(&[arr, KindedSlot::from_int(4)]).unwrap();
        let arc = as_typed_array(&r).expect("push returns array");
        match arc.as_ref() {
            TypedArrayData::I64(buf) => assert_eq!(buf.data, vec![1, 2, 3, 4]),
            _ => panic!("expected I64 array"),
        }
    }

    #[test]
    fn pop_int_array_shrinks() {
        let arr = int_array(vec![1, 2, 3]);
        let r = builtin_pop(&[arr]).unwrap();
        let arc = as_typed_array(&r).expect("pop returns array");
        match arc.as_ref() {
            TypedArrayData::I64(buf) => assert_eq!(buf.data, vec![1, 2]),
            _ => panic!("expected I64 array"),
        }
    }

    #[test]
    fn first_int_array() {
        let arr = int_array(vec![10, 20, 30]);
        let r = builtin_first(&[arr]).unwrap();
        assert_eq!(r.as_i64(), Some(10));
    }

    #[test]
    fn last_int_array() {
        let arr = int_array(vec![10, 20, 30]);
        let r = builtin_last(&[arr]).unwrap();
        assert_eq!(r.as_i64(), Some(30));
    }

    #[test]
    fn first_empty_returns_none() {
        let arr = int_array(vec![]);
        let r = builtin_first(&[arr]).unwrap();
        assert_eq!(r.slot().raw(), 0);
    }

    #[test]
    fn filled_int() {
        let r =
            builtin_filled(&[KindedSlot::from_int(3), KindedSlot::from_int(42)]).unwrap();
        let arc = as_typed_array(&r).expect("filled returns array");
        match arc.as_ref() {
            TypedArrayData::I64(buf) => assert_eq!(buf.data, vec![42, 42, 42]),
            _ => panic!("expected I64 array"),
        }
    }

    #[test]
    fn range_int_basic() {
        let r = builtin_range(&[KindedSlot::from_int(5)]).unwrap();
        let arc = as_typed_array(&r).expect("range returns array");
        match arc.as_ref() {
            TypedArrayData::I64(buf) => assert_eq!(buf.data, vec![0, 1, 2, 3, 4]),
            _ => panic!("expected I64 array"),
        }
    }

    #[test]
    fn range_widens_to_float_with_float_arg() {
        let r = builtin_range(&[KindedSlot::from_number(3.0)]).unwrap();
        let arc = as_typed_array(&r).expect("range returns array");
        match arc.as_ref() {
            TypedArrayData::F64(buf) => assert_eq!(buf.iter().copied().collect::<Vec<_>>(), vec![0.0, 1.0, 2.0]),
            _ => panic!("expected F64 array"),
        }
    }

    #[test]
    fn slice_int_basic() {
        let arr = int_array(vec![0, 1, 2, 3, 4]);
        let r = builtin_slice(&[arr, KindedSlot::from_int(1), KindedSlot::from_int(4)]).unwrap();
        let arc = as_typed_array(&r).expect("slice returns array");
        match arc.as_ref() {
            TypedArrayData::I64(buf) => assert_eq!(buf.data, vec![1, 2, 3]),
            _ => panic!("expected I64 array"),
        }
    }
}
