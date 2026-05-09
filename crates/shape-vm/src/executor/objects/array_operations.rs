//! Array operations (ArrayPush, ArrayPushLocal, ArrayPop, SliceAccess)
//!
//! Handles array manipulation and slicing for arrays, series, and strings.
//!
//! ## Wave 6.5 substep-2 Wave-α `D-array-ops` migration (playbook §10)
//!
//! Receiver kind for the array operand is
//! `NativeKind::Ptr(HeapKind::TypedArray)`; bits =
//! `Arc::into_raw(Arc<TypedArrayData>)`. Element kind is captured once at op
//! entry from `TypedArrayData::*` variant. Heap dispatch goes through
//! `Arc::<TypedArrayData>::from_raw` reconstruction (the cluster C precedent
//! in `executor/v2_handlers/typed_array_elem.rs`); the `HeapValue` enum is
//! NOT an intermediate (per-FieldType `Arc<T>` is the post-§2.7.8 canonical
//! storage shape per ADR-006 §2.4 / Q6).
//!
//! ## Out-of-territory surfaces
//!
//! Several legacy code paths in this file pre-existed in
//! `ValueWord` / `ValueBits` / `tag_bits::*` / `UnifiedArray` /
//! `as_v2_typed_array` / `as_any_array_mut` shape — all forbidden patterns
//! per ADR-006 §2.7.7 / playbook §4. Those were never aliasing-safe under
//! the substep-1 deletion of the transitional shims and they cross
//! cluster-boundary lines:
//!
//! - The `op_array_push_local` body depends on `read_ref_target` /
//!   `write_ref_target` in `executor/variables/mod.rs` (cluster B
//!   `B6-variables-loadptr` territory; still on the legacy ValueWord API).
//!   The §2.7.8 / Q10 parallel-kinds track on `module_bindings` is now
//!   live (Wave-γ `G-module-bindings-kind`, alongside Wave-α B7/B8/B9
//!   for ClosureCell / SharedCell / CallFrame.closure_heap_kind), but
//!   the `read_ref_target` / `write_ref_target` consumer migration is
//!   B6-round-2 territory. Surface per playbook §8 until B6 lands.
//! - The `as_v2_typed_array` / `push_element` / `pop_element` helpers in
//!   `executor/v2_handlers/v2_array_detect.rs` are themselves
//!   forbidden-helper carriers (cluster D `D-v2-array-detect` territory)
//!   that have not yet migrated off `ValueWord`. Surface per playbook §8.
//! - The `HeapValue::Array(Arc<Vec<ValueWord>>)` "generic VW array" arm is
//!   ValueWord-flavoured by definition; the kinded API surface on the
//!   receiver is `Ptr(HeapKind::TypedArray)`, not `Ptr(HeapKind::*)` for
//!   a VW array. Generic VW arrays remain a Phase-2c reentry surface.
//! - String slicing — the receiver kind for string-receiver `SliceAccess`
//!   is `NativeKind::String`, not `Ptr(HeapKind::TypedArray)`. The
//!   compiler currently does not partition `SliceAccess` by receiver
//!   kind; surface per playbook §8 with explicit Phase-2c reentry note.
//!
//! All migrated paths use `push_kinded` / `pop_kinded` / `drop_with_kind`
//! directly per ADR-006 §2.7.7; no `push_raw_u64` / `pop_raw_u64` /
//! `stack_*_raw` / `binding_*_raw` shims remain (substep-1 deleted those).

use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::{NativeKind, VMError};
use std::sync::Arc;

impl VirtualMachine {
    pub(in crate::executor) fn op_array_push(&mut self) -> Result<(), VMError> {
        // Stack discipline: ArrayPush expects [array, value] with `value` at
        // top. The expression result is the (mutated) array — push it back.
        let (value_bits, value_kind) = self.pop_kinded()?;
        let (array_bits, array_kind) = self.pop_kinded()?;

        match array_kind {
            NativeKind::Ptr(HeapKind::TypedArray) => {
                // Reconstruct the Arc share to dispatch on the inner
                // `TypedArrayData` variant. Take ownership: we'll either
                // re-stash it (push the array back) or drop it (error).
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(array_bits as *const TypedArrayData)
                };

                // Element kind captured once at op entry per playbook §2
                // (loop-iteration / typed-array element-source rule).
                let elem_kind = element_kind_of(&arc);

                // `push_into_typed_array` retires the value share on every
                // error path (including the variant-not-supported arm), so
                // we do NOT touch `value_bits` again after this call.
                let result = push_into_typed_array(&mut arc, value_bits, value_kind, elem_kind);

                match result {
                    Ok(()) => {
                        // Re-stash the (possibly newly-cloned via
                        // `Arc::make_mut`) Arc and push the array back.
                        let new_bits = Arc::into_raw(arc) as u64;
                        self.push_kinded(new_bits, NativeKind::Ptr(HeapKind::TypedArray))
                    }
                    Err(e) => {
                        // The Arc share is owned by `arc` (we extracted it
                        // via `from_raw` at the top). Drop it to retire the
                        // share — `array_bits` is no longer the owner.
                        // The value share was already retired by
                        // `push_into_typed_array`'s error path.
                        drop(arc);
                        let _ = (array_bits, array_kind);
                        Err(e)
                    }
                }
            }
            _ => {
                // The op was emitted against a non-typed-array receiver
                // shape. Generic-VW-array, unified-array, and the deleted
                // tag_bits dispatch were the pre-existing cover for this
                // — all forbidden per ADR-006 §2.7.7. Surface per
                // playbook §8.
                drop_with_kind(value_bits, value_kind);
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(format!(
                    "ArrayPush: receiver kind {:?} — Phase-2c reentry. \
                     The legacy ValueWord/UnifiedArray/HeapValue::Array \
                     dispatch was forbidden-pattern (ADR-006 §2.7.7); \
                     post-§2.7.8 the only kinded receiver shape is \
                     Ptr(HeapKind::TypedArray)",
                    array_kind
                )))
            }
        }
    }

    /// Push a value into an array stored in a local or module_binding variable slot,
    /// mutating in-place.
    ///
    /// Wave 6.5 §10 status: surface-and-stop. The pre-Wave-6.5 body
    /// depended on:
    ///
    /// 1. `read_ref_target` / `write_ref_target` in
    ///    `executor/variables/mod.rs` (cluster B `B6-variables-loadptr`
    ///    territory; still ValueWord-flavoured pending Wave-β).
    /// 2. The parallel-kinds track on `module_bindings` / closure cells /
    ///    shared cells / call-frame closure-heap (Wave-α B7/B8/B9 +
    ///    Wave-γ G-module-bindings-kind: ALL LANDED). The kind-source
    ///    for module-binding slots is now `module_binding_read_kinded_raw`
    ///    on `VirtualMachine`.
    /// 3. `binding_take_raw` / `binding_write_raw` / `stack_take_raw` /
    ///    `stack_write_raw` / `stack_peek_raw` — substep-1 DELETED.
    ///
    /// Remaining blocker is item #1 (B6-round-2). The cross-cluster
    /// cascade rule in playbook §8 forbids editing `variables/mod.rs`
    /// from this territory. Surface per §8 until B6 lands.
    pub(in crate::executor) fn op_array_push_local(
        &mut self,
        _instruction: &crate::bytecode::Instruction,
    ) -> Result<(), VMError> {
        // Drop the value operand to keep refcount discipline before
        // returning the surface error.
        let (value_bits, value_kind) = self.pop_kinded()?;
        drop_with_kind(value_bits, value_kind);
        Err(VMError::NotImplemented(
            "ArrayPushLocal: depends on cluster B `B6-variables-loadptr` \
             (read_ref_target / write_ref_target ValueWord migration). \
             The §2.7.8 parallel-kinds tracks on module_bindings / \
             closure cells / shared cells / call frames are live (Wave- \
             α B7/B8/B9 + Wave-γ G-module-bindings-kind), so the kind- \
             source side is unblocked. Wave-β reentry — see playbook \
             §10."
                .to_string(),
        ))
    }

    pub(in crate::executor) fn op_array_pop(&mut self) -> Result<(), VMError> {
        let (array_bits, array_kind) = self.pop_kinded()?;

        match array_kind {
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(array_bits as *const TypedArrayData)
                };
                let result = pop_from_typed_array(&mut arc);
                let new_bits = Arc::into_raw(arc) as u64;

                match result {
                    Ok((val_bits, val_kind)) => {
                        // Re-stash the (possibly newly-cloned) Arc with
                        // its kind preserved on the parallel kind track.
                        // ArrayPop in pre-Wave-6.5 form pushed only the
                        // popped value (the array was consumed). Preserve
                        // that contract — drop the array share.
                        drop_with_kind(new_bits, NativeKind::Ptr(HeapKind::TypedArray));
                        self.push_kinded(val_bits, val_kind)
                    }
                    Err(e) => {
                        drop_with_kind(new_bits, NativeKind::Ptr(HeapKind::TypedArray));
                        Err(e)
                    }
                }
            }
            _ => {
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(format!(
                    "ArrayPop: receiver kind {:?} — Phase-2c reentry. \
                     Only Ptr(HeapKind::TypedArray) is supported post-\
                     §2.7.8; legacy generic-VW-array / unified-array \
                     dispatch was forbidden-pattern.",
                    array_kind
                )))
            }
        }
    }

    pub(in crate::executor) fn op_slice_access(&mut self) -> Result<(), VMError> {
        // SliceAccess: [array, start, end] -> [slice]
        let (end_bits, end_kind) = self.pop_kinded()?;
        let (start_bits, start_kind) = self.pop_kinded()?;
        let (array_bits, array_kind) = self.pop_kinded()?;

        // Indices: integer kinds carry a raw i64; nothing else is valid.
        // Cross-domain numeric coercion was explicitly removed by the
        // CLAUDE.md "No runtime coercion" rule.
        let start = match index_from_kinded(start_bits, start_kind) {
            Ok(i) => i,
            Err(e) => {
                drop_with_kind(end_bits, end_kind);
                drop_with_kind(start_bits, start_kind);
                drop_with_kind(array_bits, array_kind);
                return Err(e);
            }
        };
        let end = match index_from_kinded(end_bits, end_kind) {
            Ok(i) => i,
            Err(e) => {
                drop_with_kind(end_bits, end_kind);
                drop_with_kind(array_bits, array_kind);
                return Err(e);
            }
        };
        // Index kinds are inline scalars; no shares to retire.
        let _ = (start_bits, end_bits);

        match array_kind {
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(array_bits as *const TypedArrayData)
                };
                let result = slice_typed_array(&arc, start, end);
                // We read by reference; restore the share.
                let _ = Arc::into_raw(arc);
                match result {
                    Ok((slice_arc, kind)) => {
                        // Drop the original array share (the stack is no
                        // longer holding it; we have exclusively the
                        // freshly-built slice).
                        drop_with_kind(array_bits, array_kind);
                        let bits = Arc::into_raw(slice_arc) as u64;
                        self.push_kinded(bits, kind)
                    }
                    Err(e) => {
                        drop_with_kind(array_bits, array_kind);
                        Err(e)
                    }
                }
            }
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                // String slicing — cluster D-array-ops surfaces this;
                // SliceAccess is not partitioned by receiver kind in the
                // current bytecode, so we'd need to also dispatch on
                // String here. That's a Phase-2c surface (the legacy
                // body did this via `as_heap_ref` + `HeapValue::String`
                // — both forbidden patterns).
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(
                    "SliceAccess: string receiver — Phase-2c reentry. \
                     The legacy as_heap_ref + HeapValue::String dispatch \
                     was forbidden-pattern; post-§2.7.8 the SliceAccess \
                     opcode needs partitioning by receiver kind."
                        .to_string(),
                ))
            }
            _ => {
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(format!(
                    "SliceAccess: receiver kind {:?} — Phase-2c reentry. \
                     Only Ptr(HeapKind::TypedArray) is supported post-\
                     §2.7.8; legacy generic-VW-array / unified-array / \
                     string dispatch was forbidden-pattern.",
                    array_kind
                )))
            }
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Local helpers (no shim usage; pure dispatch on TypedArrayData variants)
// ───────────────────────────────────────────────────────────────────────────

/// Capture-once element-kind classifier per playbook §2 (loop iteration / typed
/// array element source rule). Used to decide whether the incoming value's
/// kind is admissible for in-place push.
fn element_kind_of(arc: &Arc<TypedArrayData>) -> NativeKind {
    match &**arc {
        TypedArrayData::I64(_) => NativeKind::Int64,
        TypedArrayData::F64(_) => NativeKind::Float64,
        TypedArrayData::Bool(_) => NativeKind::Bool,
        TypedArrayData::I8(_) => NativeKind::Int8,
        TypedArrayData::I16(_) => NativeKind::Int16,
        TypedArrayData::I32(_) => NativeKind::Int32,
        TypedArrayData::U8(_) => NativeKind::UInt8,
        TypedArrayData::U16(_) => NativeKind::UInt16,
        TypedArrayData::U32(_) => NativeKind::UInt32,
        TypedArrayData::U64(_) => NativeKind::UInt64,
        TypedArrayData::F32(_) => NativeKind::Float64, // narrowed at push site
        TypedArrayData::String(_) => NativeKind::String,
        TypedArrayData::HeapValue(_) => NativeKind::Ptr(HeapKind::TypedObject),
        TypedArrayData::Matrix(_) => NativeKind::Float64,
        TypedArrayData::FloatSlice { .. } => NativeKind::Float64,
    }
}

/// Push `value` into `arr`, dispatching on the inner variant. Returns
/// `Err(NotImplemented(SURFACE))` for variants whose mutation path requires
/// out-of-territory work (Matrix immutable view, FloatSlice immutable view,
/// String/HeapValue retain-on-write semantics).
fn push_into_typed_array(
    arr: &mut Arc<TypedArrayData>,
    value_bits: u64,
    value_kind: NativeKind,
    elem_kind: NativeKind,
) -> Result<(), VMError> {
    // For variants that do support in-place push, also enforce that the
    // incoming value's kind matches the captured element kind. Mismatch is
    // a compile-time bug (CLAUDE.md "No runtime coercion") — surface, do
    // not silently re-encode.
    match Arc::make_mut(arr) {
        TypedArrayData::I64(buf) => {
            if !is_int_kind(value_kind) {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::TypeError {
                    expected: "int element",
                    got: "non-int kind",
                });
            }
            let buf = Arc::make_mut(buf);
            buf.data.push(value_bits as i64);
            // Inline scalar; no share to retire. (Inline-int kinds have
            // `clone_with_kind`/`drop_with_kind` no-ops.)
            let _ = (value_bits, elem_kind);
            Ok(())
        }
        TypedArrayData::F64(buf) => {
            if value_kind != NativeKind::Float64 && value_kind != NativeKind::NullableFloat64 {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::TypeError {
                    expected: "number element",
                    got: "non-number kind",
                });
            }
            let buf = Arc::make_mut(buf);
            buf.data.push(f64::from_bits(value_bits));
            let _ = elem_kind;
            Ok(())
        }
        TypedArrayData::Bool(buf) => {
            if value_kind != NativeKind::Bool {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::TypeError {
                    expected: "bool element",
                    got: "non-bool kind",
                });
            }
            let buf = Arc::make_mut(buf);
            buf.data.push(if value_bits != 0 { 1u8 } else { 0u8 });
            let _ = elem_kind;
            Ok(())
        }
        // The remaining TypedArrayData variants (I8/I16/I32/U8/U16/U32/U64/
        // F32/String/HeapValue/Matrix/FloatSlice) need:
        // - I8..U64: bit-narrowing per element width (Phase-2c surface
        //   while the kind-track does not yet partition int-family pushes).
        // - F32: narrowing f64 -> f32 (also Phase-2c).
        // - String / HeapValue: ownership-transfer Arc<T> push, where the
        //   element-kind matrix is exactly the kinded-API contract.
        // - Matrix / FloatSlice: read-only views; push is undefined.
        //
        // None of these were correctly handled by the pre-Wave-6.5 body
        // either — the legacy code only covered I64/F64/Bool fast-paths
        // and fell through to the forbidden generic-VW-array path
        // otherwise. Surface explicitly rather than inherit the silent
        // fall-through.
        other => {
            drop_with_kind(value_bits, value_kind);
            Err(VMError::NotImplemented(format!(
                "ArrayPush: TypedArrayData variant {} — Phase-2c reentry. \
                 Pre-Wave-6.5 silently fell through to forbidden \
                 generic-VW-array path; post-§2.7.7 this requires \
                 element-kind-aware push (int-width narrowing, \
                 String/HeapValue retain-on-write, etc.).",
                other.type_name()
            )))
        }
    }
}

/// Pop the last element from `arr`. Returns the element bits + its kind.
fn pop_from_typed_array(
    arr: &mut Arc<TypedArrayData>,
) -> Result<(u64, NativeKind), VMError> {
    match Arc::make_mut(arr) {
        TypedArrayData::I64(buf) => {
            let buf = Arc::make_mut(buf);
            match buf.data.pop() {
                Some(v) => Ok((v as u64, NativeKind::Int64)),
                None => Ok((0u64, NativeKind::Bool)),
            }
        }
        TypedArrayData::F64(buf) => {
            let buf = Arc::make_mut(buf);
            match buf.data.pop() {
                Some(v) => Ok((v.to_bits(), NativeKind::Float64)),
                None => Ok((0u64, NativeKind::Bool)),
            }
        }
        TypedArrayData::Bool(buf) => {
            let buf = Arc::make_mut(buf);
            match buf.data.pop() {
                Some(v) => Ok((if v != 0 { 1u64 } else { 0u64 }, NativeKind::Bool)),
                None => Ok((0u64, NativeKind::Bool)),
            }
        }
        other => Err(VMError::NotImplemented(format!(
            "ArrayPop: TypedArrayData variant {} — Phase-2c reentry. \
             Element-kind-aware pop required for int-width / \
             String / HeapValue variants.",
            other.type_name()
        ))),
    }
}

/// Slice `arr` at [start, end) into a fresh `Arc<TypedArrayData>` of the
/// same variant. Returns `(arc, kind)` where `kind` is always
/// `NativeKind::Ptr(HeapKind::TypedArray)`.
fn slice_typed_array(
    arr: &Arc<TypedArrayData>,
    start: i64,
    end: i64,
) -> Result<(Arc<TypedArrayData>, NativeKind), VMError> {
    let kind = NativeKind::Ptr(HeapKind::TypedArray);
    match &**arr {
        TypedArrayData::I64(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<i64> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            let new_buf = shape_value::typed_buffer::TypedBuffer::from_vec(sliced);
            let new_arc = Arc::new(TypedArrayData::I64(Arc::new(new_buf)));
            Ok((new_arc, kind))
        }
        TypedArrayData::F64(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<f64> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            let aligned = shape_value::aligned_vec::AlignedVec::<f64>::from_vec(sliced);
            let new_buf = shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            let new_arc = Arc::new(TypedArrayData::F64(Arc::new(new_buf)));
            Ok((new_arc, kind))
        }
        TypedArrayData::Bool(buf) => {
            let len = buf.data.len() as i64;
            let (s, e) = clamp_range(start, end, len);
            let sliced: Vec<u8> = if s < e { buf.data[s..e].to_vec() } else { Vec::new() };
            let new_buf = shape_value::typed_buffer::TypedBuffer::from_vec(sliced);
            let new_arc = Arc::new(TypedArrayData::Bool(Arc::new(new_buf)));
            Ok((new_arc, kind))
        }
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        } => {
            // Re-slice the parent's float region. The result is an owned
            // F64 typed array, not a nested view (matches the pre-Wave-6.5
            // semantics — the slice operator materializes).
            let total = *len as i64;
            let off = *offset as usize;
            let (s, e) = clamp_range(start, end, total);
            let sliced: Vec<f64> = if s < e {
                parent.data[off + s..off + e].to_vec()
            } else {
                Vec::new()
            };
            let aligned = shape_value::aligned_vec::AlignedVec::<f64>::from_vec(sliced);
            let new_buf = shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            let new_arc = Arc::new(TypedArrayData::F64(Arc::new(new_buf)));
            Ok((new_arc, kind))
        }
        other => Err(VMError::NotImplemented(format!(
            "SliceAccess: TypedArrayData variant {} — Phase-2c reentry. \
             Element-kind-aware slice required for int-width / \
             String / HeapValue variants.",
            other.type_name()
        ))),
    }
}

/// Clamp Python-style negative indices and bound them to `[0, len]`.
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
    (s as usize, e as usize)
}

/// True if `kind` is one of the integer-family `NativeKind`s.
#[inline]
fn is_int_kind(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::Int8
            | NativeKind::Int16
            | NativeKind::Int32
            | NativeKind::Int64
            | NativeKind::IntSize
            | NativeKind::UInt8
            | NativeKind::UInt16
            | NativeKind::UInt32
            | NativeKind::UInt64
            | NativeKind::UIntSize
            | NativeKind::NullableInt8
            | NativeKind::NullableInt16
            | NativeKind::NullableInt32
            | NativeKind::NullableInt64
            | NativeKind::NullableIntSize
            | NativeKind::NullableUInt8
            | NativeKind::NullableUInt16
            | NativeKind::NullableUInt32
            | NativeKind::NullableUInt64
            | NativeKind::NullableUIntSize
    )
}

/// Read a slice index from a kinded slot. Integer kinds carry a raw `i64`;
/// every other kind is a compile-time-prevented case.
#[inline]
fn index_from_kinded(bits: u64, kind: NativeKind) -> Result<i64, VMError> {
    if is_int_kind(kind) {
        Ok(bits as i64)
    } else {
        Err(VMError::TypeError {
            expected: "integer index",
            got: "non-integer kind",
        })
    }
}
