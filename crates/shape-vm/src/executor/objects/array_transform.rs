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
//! ## Phase-2c surfaces — closure-callback dispatch
//!
//! `map`, `filter`, `sort` (comparator form), `flatMap`, and `groupBy` all
//! need to issue per-element kinded closure callbacks through
//! `op_call_value`. That op is itself a `todo!("phase-2c")` stub in
//! `control_flow/mod.rs::op_call_value` (call_convention.rs:308 —
//! `call_value_immediate_nb` rebuild pending). The MethodFnV2 ABI is
//! kinded post-Wave-γ, but the per-element callback (kinded callee +
//! kinded arg slice + kinded result) cannot be issued until the
//! Phase-2c call-convention rebuild lands per ADR-006 §2.7.4 / §2.7.5.
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

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapKind, TypedArrayData};
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
// MethodFnV2 (native ABI) handlers — kinded carrier slice in/out
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.map(|x| ...)`
///
/// SURFACE: closure-callback dispatch through `op_call_value` is itself
/// a `todo!("phase-2c")` stub in `control_flow/mod.rs::op_call_value`
/// (`call_convention.rs:308` — `call_value_immediate_nb` rebuild
/// pending). Per-element invocation needs kinded callee + 1-arg kinded
/// slice + kinded result. Unblocked once the Phase-2c call-convention
/// rebuild lands per ADR-006 §2.7.4 / §2.7.5.
pub(crate) fn handle_map_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "map — SURFACE: closure-callback dispatch through op_call_value is \
         itself a `todo!(\"phase-2c\")` stub (control_flow/mod.rs::op_call_value, \
         call_convention.rs:308 call_value_immediate_nb rebuild pending). \
         Per-element invocation needs kinded callee + 1-arg kinded slice + \
         kinded result. Unblocked once the Phase-2c call-convention \
         rebuild lands per ADR-006 §2.7.4 / §2.7.5."
            .to_string(),
    ))
}

/// `arr.filter(|x| ...)`
///
/// SURFACE: same closure-callback dispatch gap as `map`.
pub(crate) fn handle_filter_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "filter — SURFACE: closure-callback dispatch through op_call_value is \
         itself a `todo!(\"phase-2c\")` stub (control_flow/mod.rs::op_call_value, \
         call_convention.rs:308 call_value_immediate_nb rebuild pending). \
         Per-element predicate invocation needs kinded callee + 1-arg \
         kinded slice + Bool result. Unblocked once the Phase-2c call- \
         convention rebuild lands per ADR-006 §2.7.4 / §2.7.5."
            .to_string(),
    ))
}

/// `arr.sort()` (no-arg — natural order) / `arr.sort(|a, b| ...)` (comparator)
///
/// SURFACE: the no-arg form needs per-element comparison that crosses
/// variant boundaries through `numeric_domain` dispatch + same-domain
/// total-order; that path is implementable without a closure callback,
/// but the comparator form needs the same closure-callback dispatch
/// gap as `map`. Both forms surface together to keep the dispatch
/// contract consistent — implementing one half now and the other
/// later would split the kind-dispatch table in two and re-create
/// the receiver/closure detection ambiguity the V2 ABI flip just
/// closed.
pub(crate) fn handle_sort_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "sort — SURFACE: comparator form needs closure-callback dispatch \
         through op_call_value, which is itself a `todo!(\"phase-2c\")` \
         stub (control_flow/mod.rs::op_call_value, call_convention.rs:308 \
         call_value_immediate_nb rebuild pending). Both arity-0 and \
         arity-1 forms surface together to keep the dispatch contract \
         consistent — splitting the implementation across the closure-call \
         migration boundary would re-create the receiver/closure detection \
         ambiguity the V2 ABI flip just closed. Unblocked once the \
         Phase-2c call-convention rebuild lands per ADR-006 §2.7.4 / §2.7.5."
            .to_string(),
    ))
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

/// `arr.flatMap(|x| ...)`
///
/// SURFACE: closure-callback dispatch + per-element-result-array
/// flattening. The closure-callback path is itself a `todo!("phase-2c")`
/// stub.
pub(crate) fn handle_flat_map_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "flatMap — SURFACE: closure-callback dispatch through op_call_value \
         is itself a `todo!(\"phase-2c\")` stub (control_flow/mod.rs::op_call_value, \
         call_convention.rs:308 call_value_immediate_nb rebuild pending). \
         Per-element invocation returns an array; result is the concat of \
         all per-element arrays. Unblocked once the Phase-2c call- \
         convention rebuild lands per ADR-006 §2.7.4 / §2.7.5."
            .to_string(),
    ))
}

/// `arr.groupBy(|x| key)`
///
/// SURFACE: closure-callback dispatch (key fn) + kind-aware string
/// stringifier for the bucket key. The deleted `nb_to_string_coerce`
/// (forbidden §2.7.7 #7) was the pre-Wave-6.5 stringifier; the
/// replacement needs to dispatch on `KindedSlot.kind` per §2.7.6 / Q8
/// heterogeneous-kind body pattern, but the closure-callback that
/// produces the bucket keys is the same `todo!("phase-2c")` gap as
/// `map`/`filter`. Surface together.
pub(crate) fn handle_group_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "groupBy — SURFACE: closure-callback dispatch (key fn) through \
         op_call_value is itself a `todo!(\"phase-2c\")` stub \
         (control_flow/mod.rs::op_call_value, call_convention.rs:308 \
         call_value_immediate_nb rebuild pending). Result is \
         HashMapData<Arc<String>, Arc<HeapValue::TypedArray>>; the \
         kind-aware bucket-key stringifier replaces the deleted \
         nb_to_string_coerce (forbidden §2.7.7 #7) by dispatching on \
         `KindedSlot.kind` per §2.7.6 / Q8 heterogeneous-kind body \
         pattern. Unblocked once the Phase-2c call-convention rebuild \
         lands per ADR-006 §2.7.4 / §2.7.5."
            .to_string(),
    ))
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
