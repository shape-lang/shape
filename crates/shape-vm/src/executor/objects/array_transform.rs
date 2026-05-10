//! Array transformation operations
//!
//! Handles: map, filter, sort, slice, concat, take, drop, skip, flatten,
//! flat_map, group_by
//!
//! ## Wave-δ MR-array-transform-aggregation migration (playbook §10 / §3 /
//! ADR-006 §2.7.10 / Q11)
//!
//! Wave-γ `G-method-fn-v2-abi` flipped `MethodFnV2` to the kinded carrier
//! slice form (`fn(&mut VM, &[KindedSlot], _) -> Result<KindedSlot, VMError>`).
//! Pure transforms (slice / take / drop / skip / concat / flatten with
//! TypedArrayData::FloatSlice fast-path) dispatch on
//! `args[0].kind == NativeKind::Ptr(HeapKind::TypedArray)` and reconstruct
//! the receiver share via `Arc::<TypedArrayData>::from_raw` — borrow,
//! project, then `Arc::into_raw` to restore (cluster A precedent in
//! `executor/v2_handlers/typed_array_elem.rs:119`).
//!
//! ## W9-array-transform body migration (closure-callback wave)
//!
//! Wave-γ G-method-fn-v2-abi flipped MethodFnV2 to the kinded carrier
//! slice form. W7 Round-2 (`06cdfce`) filled
//! `call_value_immediate_nb` with the kinded value-call dispatch body
//! per ADR-006 §2.7.11 / Q12 — the closure-callback path is now LIVE
//! and bodies that need per-element / per-key closure invocation
//! migrate via `vm.call_value_immediate_nb(&closure, &[elem], ctx)`
//! per the wave-9 method-refill playbook §1 recipe.
//!
//! ## Cross-variant ambiguity surfaces
//!
//! - `concat`: cross-variant concat (e.g. `[i64...].concat([f64...])`)
//!   is ambiguous under strict typing — no implicit promotion exists.
//!   Same-variant concat is implemented; cross-variant surfaces.
//! - `flatten` requires `TypedArrayData::HeapValue` per-element kind
//!   metadata to reclassify each entry as scalar-or-nested-array. The
//!   single-level `FloatSlice` fast-path is implemented; the general
//!   nested-array case surfaces.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HashMapData, HeapKind, HeapValue, TypedArrayData};
use shape_value::typed_buffer::{AlignedTypedBuffer, TypedBuffer};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers — receiver borrow + range arithmetic
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow the receiver `Arc<TypedArrayData>` from `args[0]` without
/// disturbing its strong-count share. Mirror of the
/// `array_aggregation.rs::with_typed_array` precedent.
fn with_typed_array<F, R>(args: &[KindedSlot], op: &'static str, f: F) -> Result<R, VMError>
where
    F: FnOnce(&TypedArrayData) -> Result<R, VMError>,
{
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "{}: missing receiver",
            op
        )));
    }
    match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let result = f(&arc);
            let _ = Arc::into_raw(arc);
            result
        }
        other => Err(VMError::RuntimeError(format!(
            "{}: expected Array receiver, got kind {:?}",
            op, other
        ))),
    }
}

/// Per-variant element count for `TypedArrayData`.
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

/// Read an integer-kinded slot as `i64` — used for slice indices and
/// `take`/`drop`/`skip` counts. Fails with a typed error for non-integer
/// kinds. Per-width sign-extension matches the v2_array_detect.rs:165
/// precedent so small-int negatives are handled correctly.
fn read_int_arg(slot: &KindedSlot, op: &'static str) -> Result<i64, VMError> {
    let bits = slot.slot.raw();
    match slot.kind {
        NativeKind::Int8 | NativeKind::NullableInt8 => Ok(bits as u8 as i8 as i64),
        NativeKind::Int16 | NativeKind::NullableInt16 => Ok(bits as u16 as i16 as i64),
        NativeKind::Int32 | NativeKind::NullableInt32 => Ok(bits as u32 as i32 as i64),
        NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize => Ok(bits as i64),
        NativeKind::UInt8 | NativeKind::NullableUInt8 => Ok((bits as u8) as i64),
        NativeKind::UInt16 | NativeKind::NullableUInt16 => Ok((bits as u16) as i64),
        NativeKind::UInt32 | NativeKind::NullableUInt32 => Ok((bits as u32) as i64),
        NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => Ok(bits as i64),
        _ => Err(VMError::RuntimeError(format!(
            "{}: expected integer argument, got kind {:?}",
            op, slot.kind
        ))),
    }
}

/// Python-style range clamp: negative indices count from the end; result
/// always satisfies `0 <= s <= e <= len`.
fn clamp_range(start: i64, end: i64, len: i64) -> (usize, usize) {
    let s = if start < 0 {
        (len + start).max(0)
    } else {
        start.min(len)
    };
    let e = if end < 0 {
        (len + end).max(0)
    } else {
        end.min(len)
    };
    let s = s.max(0);
    let e = e.max(s);
    (s as usize, e as usize)
}

/// Slice a `TypedArrayData` at `[start, end)` into a fresh
/// `Arc<TypedArrayData>` of the same variant. The `FloatSlice` arm
/// materializes to a flat `F64`. Errors on variants the cluster cannot
/// physically slice (`Matrix` is row-major; `String`/`HeapValue` need
/// retain-on-write — surface for Phase-2c).
fn slice_typed_array(
    arr: &TypedArrayData,
    start: i64,
    end: i64,
) -> Result<Arc<TypedArrayData>, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<i64> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::I64(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::F64(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<f64> = if s < e {
                buf.data.as_slice()[s..e].to_vec()
            } else {
                Vec::new()
            };
            let aligned = AlignedVec::<f64>::from_vec(sliced);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        TypedArrayData::Bool(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<u8> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::Bool(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::I8(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<i8> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::I8(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::I16(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<i16> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::I16(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::I32(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<i32> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::I32(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::U8(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<u8> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::U8(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::U16(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<u16> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::U16(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::U32(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<u32> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::U32(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::U64(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<u64> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::U64(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::F32(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<f32> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            Ok(Arc::new(TypedArrayData::F32(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::String(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<Arc<String>> = if s < e {
                buf.data[s..e].to_vec()
            } else {
                Vec::new()
            };
            Ok(Arc::new(TypedArrayData::String(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::HeapValue(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced = if s < e {
                buf.data[s..e].to_vec()
            } else {
                Vec::new()
            };
            Ok(Arc::new(TypedArrayData::HeapValue(Arc::new(
                TypedBuffer::from_vec(sliced),
            ))))
        }
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        } => {
            let total = *len as i64;
            let off = *offset as usize;
            let (s, e) = clamp_range(start, end, total);
            let sliced: Vec<f64> = if s < e {
                parent.data.as_slice()[off + s..off + e].to_vec()
            } else {
                Vec::new()
            };
            let aligned = AlignedVec::<f64>::from_vec(sliced);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        TypedArrayData::Matrix(_) => Err(VMError::NotImplemented(
            "slice: Matrix variant — Phase-2c reentry. Slicing a row-major \
             matrix into a flat array needs a reshape contract that is not \
             yet specified."
                .to_string(),
        )),
    }
}

/// Concat two `TypedArrayData`s of the **same variant** into a fresh
/// `Arc<TypedArrayData>`. Cross-variant concat is rejected per strict-typing
/// rules (CLAUDE.md "No runtime coercion").
fn concat_typed_array(
    a: &TypedArrayData,
    b: &TypedArrayData,
) -> Result<Arc<TypedArrayData>, VMError> {
    match (a, b) {
        (TypedArrayData::I64(la), TypedArrayData::I64(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I64(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::F64(la), TypedArrayData::F64(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(la.data.as_slice());
            out.extend_from_slice(lb.data.as_slice());
            let aligned = AlignedVec::<f64>::from_vec(out);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        (TypedArrayData::Bool(la), TypedArrayData::Bool(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::Bool(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::I8(la), TypedArrayData::I8(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I8(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::I16(la), TypedArrayData::I16(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I16(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::I32(la), TypedArrayData::I32(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::I32(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::U8(la), TypedArrayData::U8(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U8(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::U16(la), TypedArrayData::U16(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U16(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::U32(la), TypedArrayData::U32(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U32(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::U64(la), TypedArrayData::U64(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::U64(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::F32(la), TypedArrayData::F32(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::F32(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::String(la), TypedArrayData::String(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::String(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        (TypedArrayData::HeapValue(la), TypedArrayData::HeapValue(lb)) => {
            let mut out = Vec::with_capacity(la.data.len() + lb.data.len());
            out.extend_from_slice(&la.data);
            out.extend_from_slice(&lb.data);
            Ok(Arc::new(TypedArrayData::HeapValue(Arc::new(
                TypedBuffer::from_vec(out),
            ))))
        }
        // FloatSlice is a view into a parent matrix's F64 region; both
        // arms below materialize to a flat F64 result. Same-side and
        // cross-side combinations with F64 are admissible (both ultimately
        // float). Cross-variant with non-float surfaces.
        (
            TypedArrayData::FloatSlice {
                parent: lp,
                offset: loff,
                len: llen,
            },
            TypedArrayData::FloatSlice {
                parent: rp,
                offset: roff,
                len: rlen,
            },
        ) => {
            let l_off = *loff as usize;
            let l_n = *llen as usize;
            let r_off = *roff as usize;
            let r_n = *rlen as usize;
            let mut out = Vec::with_capacity(l_n + r_n);
            out.extend_from_slice(&lp.data.as_slice()[l_off..l_off + l_n]);
            out.extend_from_slice(&rp.data.as_slice()[r_off..r_off + r_n]);
            let aligned = AlignedVec::<f64>::from_vec(out);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        (
            TypedArrayData::FloatSlice {
                parent,
                offset,
                len,
            },
            TypedArrayData::F64(rb),
        ) => {
            let off = *offset as usize;
            let n = *len as usize;
            let mut out = Vec::with_capacity(n + rb.data.len());
            out.extend_from_slice(&parent.data.as_slice()[off..off + n]);
            out.extend_from_slice(rb.data.as_slice());
            let aligned = AlignedVec::<f64>::from_vec(out);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        (
            TypedArrayData::F64(la),
            TypedArrayData::FloatSlice {
                parent,
                offset,
                len,
            },
        ) => {
            let off = *offset as usize;
            let n = *len as usize;
            let mut out = Vec::with_capacity(la.data.len() + n);
            out.extend_from_slice(la.data.as_slice());
            out.extend_from_slice(&parent.data.as_slice()[off..off + n]);
            let aligned = AlignedVec::<f64>::from_vec(out);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        (TypedArrayData::Matrix(_), _) | (_, TypedArrayData::Matrix(_)) => {
            Err(VMError::NotImplemented(
                "concat: Matrix variant — Phase-2c reentry. Concatenating a \
                 row-major matrix with another array shape needs a reshape \
                 contract that is not yet specified."
                    .to_string(),
            ))
        }
        (a, b) => Err(VMError::NotImplemented(format!(
            "concat: cross-variant {} + {} — SURFACE: strict-typing \
             precludes implicit numeric promotion (CLAUDE.md \"No runtime \
             coercion\"); only same-variant concat is admissible. The \
             pre-Wave-6.5 body coerced through the deleted nb_to_string_coerce \
             / extract_number_coerce helpers (forbidden §2.7.7 #7).",
            a.type_name(),
            b.type_name()
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Closure-callback helpers (W9 — playbook §1 recipe)
// ═══════════════════════════════════════════════════════════════════════════

/// Build an owned `KindedSlot` carrier from element index `idx` of `arr`.
/// For inline-scalar arms (Int/Float/Bool) the carrier carries the raw
/// payload. For heap-bearing arms (`String`, `HeapValue`) the carrier
/// owns a freshly-cloned `Arc` share — the caller hands the carrier to
/// `call_value_immediate_nb` which transfers the share into the new
/// frame's local slot via `stack_write_kinded` (ADR-006 §2.7.7 WB2.4 —
/// stack-slot-owns-share invariant).
///
/// Per W9 playbook §4 the `Matrix` and `FloatSlice` arms project to a
/// `Float64` scalar payload (matrix elements are `f64` row-major data;
/// FloatSlice is a view into one such region). The `HeapValue` arm
/// projects each element's `Arc<HeapValue>` to a `KindedSlot` whose
/// `kind` is dispatched per `HeapValue::kind()` — this is where the
/// existing flatten/groupBy SURFACE notes flagged the per-element kind
/// metadata gap.
fn element_kinded(arr: &TypedArrayData, idx: usize) -> Result<KindedSlot, VMError> {
    match arr {
        TypedArrayData::I64(buf) => Ok(KindedSlot::from_int(buf.data[idx])),
        TypedArrayData::F64(buf) => Ok(KindedSlot::from_number(buf.data.as_slice()[idx])),
        TypedArrayData::Bool(buf) => Ok(KindedSlot::from_bool(buf.data[idx] != 0)),
        TypedArrayData::I8(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::I16(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::I32(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U8(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U16(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U32(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::U64(buf) => Ok(KindedSlot::from_int(buf.data[idx] as i64)),
        TypedArrayData::F32(buf) => Ok(KindedSlot::from_number(buf.data[idx] as f64)),
        TypedArrayData::String(buf) => {
            // String elements: the buffer owns one share per slot; bump
            // the share so the carrier owns an independent count that
            // transfers cleanly through `call_value_immediate_nb`.
            Ok(KindedSlot::from_string_arc(Arc::clone(&buf.data[idx])))
        }
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        } => {
            let off = *offset as usize;
            let n = *len as usize;
            debug_assert!(idx < n, "FloatSlice element_kinded: idx out of bounds");
            Ok(KindedSlot::from_number(parent.data.as_slice()[off + idx]))
        }
        TypedArrayData::Matrix(m) => Ok(KindedSlot::from_number(m.data.as_slice()[idx])),
        TypedArrayData::HeapValue(_) => Err(VMError::NotImplemented(
            "closure-callback over HeapValue array — SURFACE: each \
             element's NativeKind needs to be sourced from a \
             per-element parallel-kind track that does not yet exist on \
             `TypedArrayData::HeapValue` (ADR-006 §2.7.4 — same gap that \
             surfaces in `flatten`'s HeapValue arm). The closure-callback \
             ABI is live; the blocker is the missing per-element kind \
             metadata. Wave-10 / Phase-2c reentry."
                .to_string(),
        )),
    }
}

/// Stringify a `KindedSlot` for `groupBy` bucket keys. Dispatches on
/// `KindedSlot.kind` per ADR-006 §2.7.6 / Q8 heterogeneous-kind body
/// pattern. Replaces the deleted `nb_to_string_coerce` (forbidden
/// §2.7.7 #7).
fn kinded_to_bucket_key(slot: &KindedSlot) -> Result<String, VMError> {
    match slot.kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => Ok(format!("{}", slot.slot.raw() as i64)),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => Ok(format!("{}", slot.slot.raw())),
        NativeKind::Float64 => {
            let v = slot.slot.as_f64();
            if v == v.trunc() && v.abs() < 1e15 {
                Ok(format!("{}", v as i64))
            } else {
                Ok(format!("{}", v))
            }
        }
        NativeKind::Bool => Ok(format!("{}", slot.slot.as_bool())),
        NativeKind::String => slot
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| VMError::RuntimeError("groupBy: empty string-key slot".into())),
        other => Err(VMError::NotImplemented(format!(
            "groupBy: bucket-key stringification for kind {:?} — SURFACE: \
             only inline-scalar / String key kinds dispatched in W9. \
             Heap-typed keys (HeapValue::Decimal, BigInt, ...) need an \
             ADR-006 §2.7.6 / Q8 per-kind formatter table; Wave-10 / \
             Phase-2c reentry.",
            other
        ))),
    }
}

/// Build a fresh same-variant `Arc<TypedArrayData>` from indices `keep`
/// of the receiver (filter projection).
fn project_indices(arr: &TypedArrayData, keep: &[usize]) -> Result<Arc<TypedArrayData>, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let v: Vec<i64> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::I64(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::F64(buf) => {
            let v: Vec<f64> = keep.iter().map(|&i| buf.data.as_slice()[i]).collect();
            let aligned = AlignedVec::<f64>::from_vec(v);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        TypedArrayData::Bool(buf) => {
            let v: Vec<u8> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::Bool(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::I8(buf) => {
            let v: Vec<i8> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::I8(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::I16(buf) => {
            let v: Vec<i16> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::I16(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::I32(buf) => {
            let v: Vec<i32> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::I32(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U8(buf) => {
            let v: Vec<u8> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::U8(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U16(buf) => {
            let v: Vec<u16> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::U16(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U32(buf) => {
            let v: Vec<u32> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::U32(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U64(buf) => {
            let v: Vec<u64> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::U64(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::F32(buf) => {
            let v: Vec<f32> = keep.iter().map(|&i| buf.data[i]).collect();
            Ok(Arc::new(TypedArrayData::F32(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::String(buf) => {
            let v: Vec<Arc<String>> = keep.iter().map(|&i| Arc::clone(&buf.data[i])).collect();
            Ok(Arc::new(TypedArrayData::String(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len: _,
        } => {
            let off = *offset as usize;
            let v: Vec<f64> = keep
                .iter()
                .map(|&i| parent.data.as_slice()[off + i])
                .collect();
            let aligned = AlignedVec::<f64>::from_vec(v);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        TypedArrayData::HeapValue(_) | TypedArrayData::Matrix(_) => {
            Err(VMError::NotImplemented(format!(
                "filter: {} variant — SURFACE: per-element projection on \
                 HeapValue / Matrix needs the same per-element kind \
                 metadata gap that `flatten` flags (ADR-006 §2.7.4). \
                 Wave-10 / Phase-2c reentry.",
                arr.type_name()
            )))
        }
    }
}

/// Build a fresh `Arc<TypedArrayData>` from a homogeneous-kind result
/// vector. Used by `map` to materialize the per-element closure-callback
/// outputs into a single typed array. Cross-kind result vectors surface
/// (no implicit promotion under strict typing — CLAUDE.md "No runtime
/// coercion").
fn collect_homogeneous_results(
    results: Vec<KindedSlot>,
) -> Result<Arc<TypedArrayData>, VMError> {
    if results.is_empty() {
        // Empty result — pick a stable default. I64 is the natural
        // empty-array element type used elsewhere in the runtime
        // (matches the empty-vec construction in `slice_typed_array`).
        return Ok(Arc::new(TypedArrayData::I64(Arc::new(
            TypedBuffer::from_vec(Vec::<i64>::new()),
        ))));
    }
    let head_kind = results[0].kind;
    if !results.iter().all(|r| r.kind == head_kind) {
        return Err(VMError::NotImplemented(
            "map: heterogeneous closure-result kinds — SURFACE: strict \
             typing precludes implicit promotion (CLAUDE.md \"No runtime \
             coercion\"); the homogeneous-result fast-paths cover the \
             monomorphic stdlib usage. The heterogeneous fall-back \
             (HeapValue::TypedArray with per-element kind metadata) needs \
             the same per-element kind track flagged on `flatten` \
             (ADR-006 §2.7.4). Wave-10 / Phase-2c reentry."
                .to_string(),
        ));
    }
    match head_kind {
        NativeKind::Int64 => {
            let v: Vec<i64> = results.iter().map(|r| r.slot.as_i64()).collect();
            Ok(Arc::new(TypedArrayData::I64(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        NativeKind::Float64 => {
            let v: Vec<f64> = results.iter().map(|r| r.slot.as_f64()).collect();
            let aligned = AlignedVec::<f64>::from_vec(v);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        NativeKind::Bool => {
            let v: Vec<u8> = results
                .iter()
                .map(|r| if r.slot.as_bool() { 1u8 } else { 0u8 })
                .collect();
            Ok(Arc::new(TypedArrayData::Bool(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        NativeKind::String => {
            // Each result owns one Arc<String> share; we move those shares
            // into the output buffer via Arc::clone (the source carriers
            // Drop on scope exit, releasing their shares — net zero).
            let v: Vec<Arc<String>> = results
                .iter()
                .map(|r| {
                    let bits = r.slot.raw();
                    // SAFETY: kind == String → bits is `Arc::into_raw(Arc<String>)`.
                    // Bump and reconstruct into the buffer.
                    unsafe { Arc::increment_strong_count(bits as *const String) };
                    let arc = unsafe { Arc::<String>::from_raw(bits as *const String) };
                    arc
                })
                .collect();
            Ok(Arc::new(TypedArrayData::String(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        other => Err(VMError::NotImplemented(format!(
            "map: closure-result kind {:?} — SURFACE: only Int64 / Float64 / \
             Bool / String homogeneous results are materialized in W9. \
             Heap-typed result kinds (HeapValue::TypedArray, TypedObject, \
             Decimal, ...) need a per-kind output-buffer factory aligned \
             with the missing per-element kind track (ADR-006 §2.7.4). \
             Wave-10 / Phase-2c reentry.",
            other
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — kinded carrier slice in/out
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.map(|x| ...)` — per-element closure callback. The closure-callback
/// dispatch path (`call_value_immediate_nb`) landed in W7 Round-2
/// (`06cdfce`) per ADR-006 §2.7.11 / Q12; this body issues the per-
/// element call via that entry-point, collecting kinded results into a
/// homogeneous output buffer.
///
/// Heterogeneous-kind result fallback + heap-element receivers surface
/// per `collect_homogeneous_results` / `element_kinded` (ADR-006 §2.7.4
/// per-element kind metadata gap on `TypedArrayData::HeapValue`).
pub(crate) fn handle_map_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "map: expected (array, closure)".to_string(),
        ));
    }
    if args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(VMError::RuntimeError(format!(
            "map: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    let closure = &args[1];

    // Borrow the receiver arc without disturbing its share, take a
    // local copy for indexed access — we cannot hold the borrow across
    // a `&mut self` re-entry into the VM through `call_value_immediate_nb`.
    let receiver_arc: Arc<TypedArrayData> = match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            // Reconstruct + clone share + restore — same precedent as
            // `with_typed_array` but yields an owning clone for the
            // body's lifetime (the VM re-entry borrow rules force this
            // shape; we cannot pass a borrow into the inner call).
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            cloned
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "map: expected Array receiver, got kind {:?}",
                other
            )));
        }
    };
    let len = typed_array_len(&receiver_arc);

    let mut results: Vec<KindedSlot> = Vec::with_capacity(len);
    for i in 0..len {
        let elem = element_kinded(&receiver_arc, i)?;
        let result = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        results.push(result);
    }
    let out = collect_homogeneous_results(results)?;
    Ok(KindedSlot::from_typed_array(out))
}

/// `arr.filter(|x| ...)` — per-element predicate keep-mask. Predicate
/// closure is expected to return `Bool`; non-bool results surface as a
/// runtime type error.
pub(crate) fn handle_filter_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "filter: expected (array, predicate)".to_string(),
        ));
    }
    if args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(VMError::RuntimeError(format!(
            "filter: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    let closure = &args[1];

    let receiver_arc: Arc<TypedArrayData> = match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            cloned
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "filter: expected Array receiver, got kind {:?}",
                other
            )));
        }
    };
    let len = typed_array_len(&receiver_arc);

    let mut keep: Vec<usize> = Vec::with_capacity(len);
    for i in 0..len {
        let elem = element_kinded(&receiver_arc, i)?;
        let result = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        match result.kind {
            NativeKind::Bool => {
                if result.slot.as_bool() {
                    keep.push(i);
                }
            }
            other => {
                return Err(VMError::RuntimeError(format!(
                    "filter: predicate must return Bool, got kind {:?}",
                    other
                )));
            }
        }
    }
    let out = project_indices(&receiver_arc, &keep)?;
    Ok(KindedSlot::from_typed_array(out))
}

/// `arr.sort()` / `arr.sort(|a, b| ...)` — out-of-place sort. The arity-0
/// form does a natural-order sort dispatched per-variant. The arity-1
/// form invokes the comparator closure once per pair-comparison; the
/// closure is expected to return an integer (negative / zero / positive
/// for less / equal / greater).
pub(crate) fn handle_sort_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError("sort: missing receiver".to_string()));
    }
    let receiver_arc: Arc<TypedArrayData> = match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            cloned
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "sort: expected Array receiver, got kind {:?}",
                other
            )));
        }
    };

    // Arity-1 form: comparator closure.
    if args.len() >= 2 {
        if args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
            return Err(VMError::RuntimeError(format!(
                "sort: comparator must be a closure, got kind {:?}",
                args[1].kind
            )));
        }
        let closure = &args[1];
        let len = typed_array_len(&receiver_arc);
        // Build an index permutation and sort it via comparator-driven
        // bubble sort. Bubble sort keeps the comparator-call count
        // bounded by O(n^2) without a stable-sort closure-recursion
        // contract; same shape as the pre-Wave-6 comparator path.
        let mut idx: Vec<usize> = (0..len).collect();
        // Use stdlib stable sort via comparator that issues a closure
        // call. Closure errors propagate via a sticky `Result` shadow —
        // `slice::sort_by` cannot signal failure, so we capture the
        // first error and short-circuit subsequent comparisons.
        let mut cmp_err: Option<VMError> = None;
        idx.sort_by(|&a, &b| {
            if cmp_err.is_some() {
                return std::cmp::Ordering::Equal;
            }
            let elem_a = match element_kinded(&receiver_arc, a) {
                Ok(s) => s,
                Err(e) => {
                    cmp_err = Some(e);
                    return std::cmp::Ordering::Equal;
                }
            };
            let elem_b = match element_kinded(&receiver_arc, b) {
                Ok(s) => s,
                Err(e) => {
                    cmp_err = Some(e);
                    return std::cmp::Ordering::Equal;
                }
            };
            let result =
                match vm.call_value_immediate_nb(closure, &[elem_a, elem_b], ctx.as_deref_mut()) {
                    Ok(r) => r,
                    Err(e) => {
                        cmp_err = Some(e);
                        return std::cmp::Ordering::Equal;
                    }
                };
            // Comparator return: negative / zero / positive integer.
            let cmp_int: i64 = match result.kind {
                NativeKind::Int8
                | NativeKind::Int16
                | NativeKind::Int32
                | NativeKind::Int64
                | NativeKind::IntSize => result.slot.raw() as i64,
                NativeKind::Float64 => {
                    let v = result.slot.as_f64();
                    if v < 0.0 {
                        -1
                    } else if v > 0.0 {
                        1
                    } else {
                        0
                    }
                }
                other => {
                    cmp_err = Some(VMError::RuntimeError(format!(
                        "sort: comparator must return integer or number, got kind {:?}",
                        other
                    )));
                    return std::cmp::Ordering::Equal;
                }
            };
            if cmp_int < 0 {
                std::cmp::Ordering::Less
            } else if cmp_int > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        if let Some(e) = cmp_err {
            return Err(e);
        }
        let out = project_indices(&receiver_arc, &idx)?;
        return Ok(KindedSlot::from_typed_array(out));
    }

    // Arity-0 form: natural-order per-variant sort.
    let out = sort_natural(&receiver_arc)?;
    Ok(KindedSlot::from_typed_array(out))
}

/// Per-variant natural-order sort. Numeric variants use total order;
/// `f64` / `f32` use `total_cmp` (NaN-safe). Strings sort by lexical
/// `Ord`. `Bool` sorts false-before-true.
fn sort_natural(arr: &TypedArrayData) -> Result<Arc<TypedArrayData>, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::I64(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::F64(buf) => {
            let mut v = buf.data.as_slice().to_vec();
            v.sort_by(|a, b| a.total_cmp(b));
            let aligned = AlignedVec::<f64>::from_vec(v);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        TypedArrayData::Bool(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::Bool(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::I8(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::I8(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::I16(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::I16(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::I32(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::I32(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U8(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::U8(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U16(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::U16(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U32(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::U32(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::U64(buf) => {
            let mut v = buf.data.to_vec();
            v.sort();
            Ok(Arc::new(TypedArrayData::U64(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::F32(buf) => {
            let mut v = buf.data.to_vec();
            v.sort_by(|a, b| a.total_cmp(b));
            Ok(Arc::new(TypedArrayData::F32(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::String(buf) => {
            let mut v: Vec<Arc<String>> = buf.data.iter().map(Arc::clone).collect();
            v.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Arc::new(TypedArrayData::String(Arc::new(
                TypedBuffer::from_vec(v),
            ))))
        }
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        } => {
            let off = *offset as usize;
            let n = *len as usize;
            let mut v: Vec<f64> = parent.data.as_slice()[off..off + n].to_vec();
            v.sort_by(|a, b| a.total_cmp(b));
            let aligned = AlignedVec::<f64>::from_vec(v);
            Ok(Arc::new(TypedArrayData::F64(Arc::new(
                AlignedTypedBuffer::from_aligned(aligned),
            ))))
        }
        TypedArrayData::Matrix(_) | TypedArrayData::HeapValue(_) => {
            Err(VMError::NotImplemented(format!(
                "sort: {} variant — SURFACE: row-major matrix and \
                 HeapValue arrays need the per-element kind metadata gap \
                 flagged on `flatten` (ADR-006 §2.7.4) for an order \
                 contract. Wave-10 / Phase-2c reentry.",
                arr.type_name()
            )))
        }
    }
}

/// `arr.slice(start, end)` — Python-style range slicing, negative
/// indices count from the end. Receiver kind preserved (same-variant
/// slice).
pub(crate) fn handle_slice_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "slice: expected 2 arguments (start, end)".to_string(),
        ));
    }
    let start = read_int_arg(&args[1], "slice")?;
    let end = read_int_arg(&args[2], "slice")?;
    with_typed_array(args, "slice", |arr| {
        let result = slice_typed_array(arr, start, end)?;
        Ok(KindedSlot::from_typed_array(result))
    })
}

/// `arr.concat(other)` — same-variant concatenation. Cross-variant
/// surfaces with a SURFACE error (strict-typing rule per CLAUDE.md
/// "No runtime coercion").
pub(crate) fn handle_concat_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "concat: expected 1 argument (other array)".to_string(),
        ));
    }
    // Both receiver and `other` must be Arrays. Reconstruct both shares
    // borrow-only, project, then `Arc::into_raw` to restore.
    let (a_kind, a_bits) = (args[0].kind, args[0].slot.raw());
    let (b_kind, b_bits) = (args[1].kind, args[1].slot.raw());
    if a_kind != NativeKind::Ptr(HeapKind::TypedArray)
        || b_kind != NativeKind::Ptr(HeapKind::TypedArray)
    {
        return Err(VMError::RuntimeError(format!(
            "concat: expected (Array, Array), got ({:?}, {:?})",
            a_kind, b_kind
        )));
    }
    let arc_a = unsafe { Arc::<TypedArrayData>::from_raw(a_bits as *const TypedArrayData) };
    let arc_b = unsafe { Arc::<TypedArrayData>::from_raw(b_bits as *const TypedArrayData) };
    let result = concat_typed_array(&arc_a, &arc_b);
    let _ = Arc::into_raw(arc_a);
    let _ = Arc::into_raw(arc_b);
    Ok(KindedSlot::from_typed_array(result?))
}

/// `arr.take(n)` — first `n` elements (clamped at array length). `n < 0`
/// is treated as 0 (consistent with the slice clamp).
pub(crate) fn handle_take_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "take: expected 1 argument (count)".to_string(),
        ));
    }
    let n = read_int_arg(&args[1], "take")?;
    with_typed_array(args, "take", |arr| {
        let result = slice_typed_array(arr, 0, n.max(0))?;
        Ok(KindedSlot::from_typed_array(result))
    })
}

/// `arr.drop(n)` / `arr.skip(n)` — drop the first `n` elements (clamped).
pub(crate) fn handle_drop_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "drop: expected 1 argument (count)".to_string(),
        ));
    }
    let n = read_int_arg(&args[1], "drop")?;
    with_typed_array(args, "drop", |arr| {
        let len = typed_array_len(arr) as i64;
        let result = slice_typed_array(arr, n.max(0), len)?;
        Ok(KindedSlot::from_typed_array(result))
    })
}

/// `arr.skip(n)` — alias of `drop(n)`.
pub(crate) fn handle_skip_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    handle_drop_v2(vm, args, ctx)
}

/// `arr.flatten()` — single-level flatten. Implemented for the
/// `FloatSlice` fast-path (re-materialize the parent's float region into
/// a fresh `F64`). The general case (HeapValue array of nested arrays)
/// needs `TypedArrayData::HeapValue` per-element kind metadata to
/// reclassify each entry; surface that path explicitly.
pub(crate) fn handle_flatten_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    with_typed_array(args, "flatten", |arr| match arr {
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        } => {
            let off = *offset as usize;
            let n = *len as usize;
            let flat: Vec<f64> = parent.data.as_slice()[off..off + n].to_vec();
            let aligned = AlignedVec::<f64>::from_vec(flat);
            Ok(KindedSlot::from_typed_array(Arc::new(TypedArrayData::F64(
                Arc::new(AlignedTypedBuffer::from_aligned(aligned)),
            ))))
        }
        TypedArrayData::Matrix(m) => {
            // A Matrix in flat form is the row-major data; flatten() is
            // the natural projection to a 1-D F64 array.
            let flat: Vec<f64> = m.data.as_slice().to_vec();
            let aligned = AlignedVec::<f64>::from_vec(flat);
            Ok(KindedSlot::from_typed_array(Arc::new(TypedArrayData::F64(
                Arc::new(AlignedTypedBuffer::from_aligned(aligned)),
            ))))
        }
        // I64/F64/Bool/I*/U*/F32/String already 1-D — flatten is identity.
        TypedArrayData::I64(_)
        | TypedArrayData::F64(_)
        | TypedArrayData::Bool(_)
        | TypedArrayData::I8(_)
        | TypedArrayData::I16(_)
        | TypedArrayData::I32(_)
        | TypedArrayData::U8(_)
        | TypedArrayData::U16(_)
        | TypedArrayData::U32(_)
        | TypedArrayData::U64(_)
        | TypedArrayData::F32(_)
        | TypedArrayData::String(_) => {
            // Identity: clone the receiver share into a fresh KindedSlot
            // (caller still owns the original). Cloning the Arc through
            // `Arc::increment_strong_count` keeps refcount discipline.
            let bits = args[0].slot.raw();
            unsafe {
                Arc::increment_strong_count(bits as *const TypedArrayData);
            }
            Ok(KindedSlot::new(
                args[0].slot,
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        TypedArrayData::HeapValue(_) => Err(VMError::NotImplemented(
            "flatten: HeapValue array variant — SURFACE: nested-array \
             reclassification needs TypedArrayData::HeapValue per-element \
             kind metadata to dispatch element-by-element on \
             scalar-or-nested-array shape. Wave-10 / Phase-2c reentry."
                .to_string(),
        )),
    })
}

/// `arr.flatMap(|x| ...)` — per-element closure callback returns an array;
/// the body concats all per-element result arrays into a single
/// homogeneous output buffer.
pub(crate) fn handle_flat_map_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "flatMap: expected (array, closure)".to_string(),
        ));
    }
    if args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(VMError::RuntimeError(format!(
            "flatMap: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    let closure = &args[1];

    let receiver_arc: Arc<TypedArrayData> = match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            cloned
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "flatMap: expected Array receiver, got kind {:?}",
                other
            )));
        }
    };
    let len = typed_array_len(&receiver_arc);

    // Each per-element call returns an `Arc<TypedArrayData>`; collect
    // them and concat pairwise via the same-variant `concat_typed_array`
    // helper. Cross-variant returns surface (strict-typing).
    let mut accum: Option<Arc<TypedArrayData>> = None;
    for i in 0..len {
        let elem = element_kinded(&receiver_arc, i)?;
        let result = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        if result.kind != NativeKind::Ptr(HeapKind::TypedArray) {
            return Err(VMError::RuntimeError(format!(
                "flatMap: closure must return Array, got kind {:?}",
                result.kind
            )));
        }
        // Recover the result's `Arc<TypedArrayData>` via the typed-Arc
        // round-trip pattern (ADR-005 §1 single-discriminator + ADR-006
        // §2.4 typed-pointer slot ABI). `Ptr(HeapKind::TypedArray)`
        // slots store `Arc::into_raw(Arc<TypedArrayData>)` directly —
        // reconstruct, clone, restore the share.
        let result_arc: Arc<TypedArrayData> = unsafe {
            let arc =
                Arc::<TypedArrayData>::from_raw(result.slot.raw() as *const TypedArrayData);
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            cloned
        };
        accum = Some(match accum.take() {
            None => result_arc,
            Some(prev) => concat_typed_array(&prev, &result_arc)?,
        });
        // `result` Drop releases the outer Arc<TypedArrayData> share.
    }
    let out = accum.unwrap_or_else(|| {
        Arc::new(TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(
            Vec::<i64>::new(),
        ))))
    });
    Ok(KindedSlot::from_typed_array(out))
}

/// `arr.groupBy(|x| key)` — bucket each element under `key(elem)` and
/// return a `HashMap<String, Array>`. The kind-aware bucket-key
/// stringifier (`kinded_to_bucket_key`) replaces the deleted
/// `nb_to_string_coerce` (forbidden §2.7.7 #7) by dispatching on
/// `KindedSlot.kind` per ADR-006 §2.7.6 / Q8 heterogeneous-kind body
/// pattern. Heap-typed key kinds surface per the helper's contract.
pub(crate) fn handle_group_by_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "groupBy: expected (array, key_fn)".to_string(),
        ));
    }
    if args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(VMError::RuntimeError(format!(
            "groupBy: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    let closure = &args[1];

    let receiver_arc: Arc<TypedArrayData> = match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let cloned = Arc::clone(&arc);
            let _ = Arc::into_raw(arc);
            cloned
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "groupBy: expected Array receiver, got kind {:?}",
                other
            )));
        }
    };
    let len = typed_array_len(&receiver_arc);

    // Bucket the receiver's element indices by stringified key,
    // preserving insertion order via a `Vec<(key, indices)>` buffer.
    let mut buckets: Vec<(String, Vec<usize>)> = Vec::new();
    for i in 0..len {
        let elem = element_kinded(&receiver_arc, i)?;
        let key_slot = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        let key = kinded_to_bucket_key(&key_slot)?;
        if let Some(existing) = buckets.iter_mut().find(|(k, _)| *k == key) {
            existing.1.push(i);
        } else {
            buckets.push((key, vec![i]));
        }
    }

    // Materialize each bucket as a same-variant Arc<TypedArrayData>,
    // wrap in HeapValue::TypedArray, and compose into a HashMapData
    // via `from_pairs`.
    let mut keys: Vec<Arc<String>> = Vec::with_capacity(buckets.len());
    let mut values: Vec<Arc<HeapValue>> = Vec::with_capacity(buckets.len());
    for (key, indices) in buckets {
        let bucket_arr = project_indices(&receiver_arc, &indices)?;
        keys.push(Arc::new(key));
        values.push(Arc::new(HeapValue::TypedArray(bucket_arr)));
    }
    let map = HashMapData::from_pairs(keys, values);
    Ok(KindedSlot::from_hashmap(Arc::new(map)))
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests removed during stub period; concrete tests for slice/take/drop/skip/
// concat/flatten attach to `op_call_method` once the dispatch shell rebuild
// lands (currently SURFACE in `objects/mod.rs:343`). Per-helper unit tests
// for `slice_typed_array` / `concat_typed_array` / `clamp_range` are
// reachable directly — keep this scaffold here for the rebuild.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_range_negative_indices_count_from_end() {
        // [0, 1, 2, 3, 4, 5] → slice(-3, -1) = [3, 4]
        assert_eq!(clamp_range(-3, -1, 6), (3, 5));
        // slice(-10, 100) saturates to (0, len)
        assert_eq!(clamp_range(-10, 100, 6), (0, 6));
        // start > end after clamp → e == s
        assert_eq!(clamp_range(5, 2, 6), (5, 5));
    }

    #[test]
    fn slice_typed_array_i64_basic() {
        let buf = TypedBuffer::from_vec(vec![10i64, 20, 30, 40, 50]);
        let arr = TypedArrayData::I64(Arc::new(buf));
        let result = slice_typed_array(&arr, 1, 4).unwrap();
        match &*result {
            TypedArrayData::I64(b) => assert_eq!(&b.data, &[20, 30, 40]),
            other => panic!("expected I64, got {}", other.type_name()),
        }
    }

    #[test]
    fn slice_typed_array_negative_indices() {
        let buf = TypedBuffer::from_vec(vec![1i64, 2, 3, 4, 5]);
        let arr = TypedArrayData::I64(Arc::new(buf));
        // slice(-2, 5) → last two elements
        let result = slice_typed_array(&arr, -2, 5).unwrap();
        match &*result {
            TypedArrayData::I64(b) => assert_eq!(&b.data, &[4, 5]),
            other => panic!("expected I64, got {}", other.type_name()),
        }
    }

    #[test]
    fn concat_typed_array_same_variant_i64() {
        let a = TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(vec![1i64, 2])));
        let b = TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(vec![3i64, 4, 5])));
        let result = concat_typed_array(&a, &b).unwrap();
        match &*result {
            TypedArrayData::I64(buf) => assert_eq!(&buf.data, &[1, 2, 3, 4, 5]),
            other => panic!("expected I64, got {}", other.type_name()),
        }
    }

    #[test]
    fn concat_typed_array_cross_variant_surfaces() {
        let a = TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(vec![1i64])));
        let b = TypedArrayData::F64(Arc::new(AlignedTypedBuffer::from_aligned(
            AlignedVec::<f64>::from_vec(vec![2.0]),
        )));
        let result = concat_typed_array(&a, &b);
        assert!(result.is_err(), "cross-variant concat must surface");
    }

}
