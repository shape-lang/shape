//! Array operations (ArrayPush, ArrayPushLocal, ArrayPop, SliceAccess)
//!
//! Handles array manipulation and slicing for arrays, series, and strings.
//!
//! ## Receiver carrier shapes (ADR-006 §2.7.6 / §2.7.7)
//!
//! Two array carrier shapes flow through these opcodes:
//!
//! 1. **`NativeKind::Ptr(HeapKind::TypedArray)`** — legacy Arc-boxed
//!    `Arc<TypedArrayData>` carrier. Bits = `Arc::into_raw(Arc<TypedArrayData>)`.
//!    Element kind is captured once at op entry from `TypedArrayData::*`
//!    variant. Heap dispatch goes through `Arc::<TypedArrayData>::from_raw`
//!    reconstruction (the cluster C precedent in
//!    `executor/v2_handlers/typed_array_elem.rs`); the `HeapValue` enum is
//!    NOT an intermediate (per-FieldType `Arc<T>` is the post-§2.7.8 canonical
//!    storage shape per ADR-006 §2.4 / Q6).
//!
//! 2. **`NativeKind::UInt64`** — v2 raw `*mut TypedArray<T>` pointer
//!    (`v2_handlers/array.rs` allocation path). No Arc, no refcount; bits
//!    are the raw pointer, kind is `UInt64`. Element type byte stamped in
//!    the heap header lets `v2_array_detect::as_v2_typed_array` recover an
//!    element-typed view from the `(bits, UInt64)` pair. In-place push
//!    uses `realloc` which preserves the outer struct pointer, so the
//!    slot doesn't need rewriting on growth (W17-array-typed-receiver
//!    invariant — the data-pointer aliasing class is tracked separately
//!    as "v2-raw-heap-audit", out of this sub-cluster).
//!
//! ## Phase 2d W17-array-typed-receiver
//!
//! This sub-cluster fills the bodies for non-`Ptr(HeapKind::TypedArray)`
//! receivers. The pre-W17 form short-circuited every non-Arc receiver kind
//! to `NotImplemented(SURFACE)`. The actual current shape for plain array
//! literals (`let mut xs = [1, 2, 3]; xs.push(4)`) is the v2 raw-pointer
//! path (NewTypedArrayI64 → UInt64 carrier), so ArrayPushLocal's UInt64
//! branch is what unblocks the smoke target.
//!
//! ## Still-surfaced shapes
//!
//! - **String receiver for SliceAccess** (`NativeKind::String` /
//!   `Ptr(HeapKind::String)`): the SliceAccess opcode is not partitioned
//!   by receiver kind in the current bytecode emission; string slicing
//!   needs a dedicated bytecode shape. Out of W17-array-typed-receiver
//!   territory (the territory line is *array* receivers).
//! - **Legacy generic-VW-array** (`HeapValue::Array(Arc<Vec<ValueWord>>)`):
//!   forbidden under ADR-006 §2.7.7 (ValueWord deletion). Programs that
//!   would have hit this path now lower to v2 typed-array (UInt64) for
//!   homogeneous int/number/bool literals, or to NewArray + the cluster
//!   W17-typed-carrier-monomorphization territory for heterogeneous /
//!   string / object element types.
//! - **String-element typed arrays** (`TypedArrayData::String` /
//!   `the-deleted-heterogeneous-element-carrier`): the §2.7.24 Q25 typed-carrier
//!   monomorphization sub-cluster's territory; surface in the Ptr branch.
//! - **Sub-i64 integer widths and F32**: still surface — the kind track
//!   needs to partition int-family pushes by width before in-place
//!   mutation is sound (out-of-territory).
//!
//! All migrated paths use `push_kinded` / `pop_kinded` / `drop_with_kind`
//! directly per ADR-006 §2.7.7; no `push_raw_u64` / `pop_raw_u64` /
//! `stack_*_raw` / `binding_*_raw` shims remain.

use crate::bytecode::{Instruction, Operand};
use crate::executor::v2_handlers::v2_array_detect::{
    self, ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I32, ELEM_TYPE_I64, V2ElemType,
    V2TypedArrayView,
};
use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::v2::typed_array::TypedArray;
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
            NativeKind::UInt64 => {
                // W17-array-typed-receiver: v2 typed-array carrier shape
                // (raw `*mut TypedArray<T>` with `UInt64` kind, no Arc,
                // no refcount; see v2_handlers/array.rs).
                //
                // Detect → push_element → re-stash the receiver pointer.
                // `push_element` mutates in place; on growth, `realloc`
                // preserves the outer struct pointer, so the slot
                // doesn't need rewriting.
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        // Element-kind admission gates by view kind; the
                        // primitive `push_element` rejects incompatible
                        // (bits, kind) pairs (see `decode_*` in
                        // v2_array_detect).
                        match v2_array_detect::push_element(
                            &view, value_bits, value_kind,
                        ) {
                            Ok(()) => {
                                // Value bits transferred into the array
                                // buffer; UInt64 receiver has no_op
                                // clone/drop, no share to retire. Re-push
                                // the receiver pointer as the expression
                                // result.
                                self.push_kinded(array_bits, NativeKind::UInt64)
                            }
                            Err(msg) => {
                                drop_with_kind(value_bits, value_kind);
                                // UInt64 receiver has no_op drop; nothing
                                // to retire on array_bits.
                                Err(VMError::TypeError {
                                    expected: "v2 typed-array element",
                                    got: msg,
                                })
                            }
                        }
                    }
                    None => {
                        // UInt64-kinded bits that are NOT a v2 typed-array
                        // pointer (foreign raw pointer, FFI scratchpad,
                        // etc.). Surface — fabricating an element kind
                        // would violate ADR-006 §2.7.8 #4.
                        drop_with_kind(value_bits, value_kind);
                        Err(VMError::NotImplemented(
                            "ArrayPush: UInt64 receiver did not resolve to a \
                             v2 typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                             header missing). The op was emitted against a \
                             non-array UInt64-kinded value — compiler bug. \
                             ADR-006 §2.7.6 / §2.7.7."
                                .to_string(),
                        ))
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
                    "ArrayPush: receiver kind {:?} — only \
                     Ptr(HeapKind::TypedArray) (Arc-boxed) and UInt64 \
                     (v2 raw pointer) are supported post-§2.7.7. The \
                     legacy ValueWord/UnifiedArray/HeapValue::Array \
                     dispatch was forbidden-pattern.",
                    array_kind
                )))
            }
        }
    }

    /// Push a value into an array stored in a local or module_binding
    /// variable slot, mutating in-place.
    ///
    /// W17-array-typed-receiver: receiver location comes from the
    /// instruction operand (`Local(idx)` → `stack[base_pointer + idx]`;
    /// `ModuleBinding(idx)` → `module_bindings[idx]`). Both stores have
    /// a §2.7.7 / §2.7.8 parallel-kind track — kind is sourced from the
    /// track, never fabricated.
    ///
    /// Both array carrier shapes are handled:
    ///
    /// - **Ptr(HeapKind::TypedArray)**: take the slot's share via
    ///   `*_take_kinded` (replacing it with the zero/Bool sentinel),
    ///   reconstruct the Arc, dispatch on `TypedArrayData::*`, mutate
    ///   via `Arc::make_mut`, then re-install the (possibly newly
    ///   cloned) Arc bits directly. Slot is currently sentinel, so the
    ///   direct write doesn't double-drop. Net Arc strong-count change:
    ///   0 (slot's share moved through the temporary).
    ///
    /// - **UInt64**: in-place push through
    ///   `v2_array_detect::push_element`. `TypedArray<T>::push`'s grow
    ///   path uses `realloc` which preserves the outer struct pointer,
    ///   so the slot's pointer stays valid — no take/restore needed.
    ///   UInt64 has no_op clone/drop, no refcount accounting.
    pub(in crate::executor) fn op_array_push_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // Pop the value operand first (the receiver lives in a slot,
        // not on the stack).
        let (value_bits, value_kind) = self.pop_kinded()?;

        let receiver_loc = match instruction.operand {
            Some(Operand::Local(idx)) => ReceiverLoc::Local(idx as usize),
            Some(Operand::ModuleBinding(idx)) => ReceiverLoc::ModuleBinding(idx as usize),
            _ => {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::InvalidOperand);
            }
        };

        // Peek the kind to dispatch without disturbing the slot — Arc
        // and raw-pointer paths take different ownership routes.
        let (_peek_bits, array_kind) = self.read_receiver_loc(&receiver_loc);

        match array_kind {
            NativeKind::Ptr(HeapKind::TypedArray) => {
                // Take ownership of the slot's share (slot becomes
                // (0, Bool) sentinel). This avoids the double-decrement
                // hazard of round-tripping `from_raw → into_raw` while
                // leaving the slot live.
                let (array_bits, _) = self.take_receiver_loc(&receiver_loc);
                let mut arc = unsafe {
                    Arc::<TypedArrayData>::from_raw(array_bits as *const TypedArrayData)
                };
                let elem_kind = element_kind_of(&arc);
                let push_result =
                    push_into_typed_array(&mut arc, value_bits, value_kind, elem_kind);
                let new_bits = Arc::into_raw(arc) as u64;

                // Slot is sentinel — install the new Arc bits directly
                // without dropping (there is no prior occupant to drop).
                self.write_sentinel_loc(
                    &receiver_loc,
                    new_bits,
                    NativeKind::Ptr(HeapKind::TypedArray),
                );
                push_result
            }
            NativeKind::UInt64 => {
                let (array_bits, _) = self.read_receiver_loc(&receiver_loc);
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        match v2_array_detect::push_element(
                            &view, value_bits, value_kind,
                        ) {
                            Ok(()) => Ok(()),
                            Err(msg) => {
                                drop_with_kind(value_bits, value_kind);
                                Err(VMError::TypeError {
                                    expected: "v2 typed-array element",
                                    got: msg,
                                })
                            }
                        }
                    }
                    None => {
                        drop_with_kind(value_bits, value_kind);
                        Err(VMError::NotImplemented(
                            "ArrayPushLocal: UInt64 slot did not resolve to a \
                             v2 typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                             header missing). The op was emitted against a \
                             non-array UInt64-kinded slot — compiler bug. \
                             ADR-006 §2.7.6 / §2.7.7."
                                .to_string(),
                        ))
                    }
                }
            }
            _ => {
                drop_with_kind(value_bits, value_kind);
                Err(VMError::NotImplemented(format!(
                    "ArrayPushLocal: slot kind {:?} — only \
                     Ptr(HeapKind::TypedArray) (Arc-boxed) and UInt64 \
                     (v2 raw pointer) are supported post-§2.7.7. The \
                     legacy ValueWord/UnifiedArray/HeapValue::Array \
                     dispatch was forbidden-pattern.",
                    array_kind
                )))
            }
        }
    }

    /// Borrow the receiver `(bits, kind)` from the slot — no refcount
    /// change, slot retains ownership.
    fn read_receiver_loc(&self, loc: &ReceiverLoc) -> (u64, NativeKind) {
        match loc {
            ReceiverLoc::Local(idx) => {
                let bp = self.current_locals_base();
                self.stack_read_kinded_raw(bp + *idx)
            }
            ReceiverLoc::ModuleBinding(idx) => self.module_binding_read_kinded_raw(*idx),
        }
    }

    /// Take ownership of the receiver share, replacing the slot with
    /// the zero/Bool sentinel. The caller now owns the bits.
    fn take_receiver_loc(&mut self, loc: &ReceiverLoc) -> (u64, NativeKind) {
        match loc {
            ReceiverLoc::Local(idx) => {
                let bp = self.current_locals_base();
                self.stack_take_kinded(bp + *idx)
            }
            ReceiverLoc::ModuleBinding(idx) => self.module_binding_take_kinded(*idx),
        }
    }

    /// Install `(bits, kind)` into a slot that currently holds the
    /// zero/Bool sentinel (i.e. just after a `take_receiver_loc`). The
    /// sentinel's `drop_with_kind` is a no-op, so this is safe to call
    /// via the kinded writer; using it ensures the parallel-kind track
    /// stays in lockstep.
    fn write_sentinel_loc(
        &mut self,
        loc: &ReceiverLoc,
        bits: u64,
        kind: NativeKind,
    ) {
        match loc {
            ReceiverLoc::Local(idx) => {
                let bp = self.current_locals_base();
                self.stack_write_kinded(bp + *idx, bits, kind);
            }
            ReceiverLoc::ModuleBinding(idx) => {
                self.module_binding_write_kinded(*idx, bits, kind);
            }
        }
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
            NativeKind::UInt64 => {
                // W17-array-typed-receiver: v2 typed-array carrier.
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        // Pop from the underlying TypedArray<T>. The
                        // receiver pointer is consumed (matches the
                        // pre-Wave-6.5 ArrayPop contract: only the
                        // popped value is pushed back, the array is
                        // not).
                        let result = v2_array_detect::pop_element(&view);
                        // UInt64 has no_op drop — no refcount accounting
                        // needed on `array_bits`.
                        let _ = array_bits;
                        match result {
                            Some((val_bits, val_kind)) => {
                                self.push_kinded(val_bits, val_kind)
                            }
                            None => {
                                // Empty array — push the null/unit
                                // sentinel (matches the legacy
                                // pop-on-empty contract used by typed
                                // array methods, see
                                // `typed_int_array_methods::pop`).
                                self.push_kinded(0u64, NativeKind::Bool)
                            }
                        }
                    }
                    None => Err(VMError::NotImplemented(
                        "ArrayPop: UInt64 receiver did not resolve to a v2 \
                         typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                         header missing). ADR-006 §2.7.6 / §2.7.7."
                            .to_string(),
                    )),
                }
            }
            _ => {
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(format!(
                    "ArrayPop: receiver kind {:?} — only \
                     Ptr(HeapKind::TypedArray) (Arc-boxed) and UInt64 \
                     (v2 raw pointer) are supported post-§2.7.7. \
                     Legacy generic-VW-array / unified-array dispatch \
                     was forbidden-pattern.",
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
            NativeKind::UInt64 => {
                // W17-array-typed-receiver: v2 typed-array carrier.
                // Build a fresh `TypedArray<T>` over the sliced range
                // and push the new pointer back as `UInt64`. The
                // receiver pointer is consumed (matches the
                // Ptr(HeapKind::TypedArray) contract above: the
                // original array share is dropped, slice owns the
                // fresh allocation).
                match v2_array_detect::as_v2_typed_array(array_bits, array_kind) {
                    Some(view) => {
                        let (s, e) = clamp_range(start, end, view.len as i64);
                        let new_ptr = slice_v2_typed_array(&view, s, e);
                        // UInt64 has no_op drop on the source pointer;
                        // the source v2 typed array is reference-free
                        // (lives as long as the slot does). We push the
                        // freshly-allocated slice pointer.
                        let _ = array_bits;
                        self.push_kinded(new_ptr as u64, NativeKind::UInt64)
                    }
                    None => Err(VMError::NotImplemented(
                        "SliceAccess: UInt64 receiver did not resolve to a \
                         v2 typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY \
                         header missing). ADR-006 §2.7.6 / §2.7.7."
                            .to_string(),
                    )),
                }
            }
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                // String slicing — receiver kind is `NativeKind::String`,
                // not an array carrier. SliceAccess is not partitioned by
                // receiver kind in the current bytecode emission; the
                // string-receiver shape needs a dedicated bytecode op
                // (or the SliceAccess op needs a runtime partition by
                // string kind). Out of W17-array-typed-receiver
                // territory — string slicing surfaces a *separate*
                // architectural gap (the legacy `as_heap_ref` +
                // `HeapValue::String` dispatch was forbidden-pattern).
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(
                    "SliceAccess: string receiver — out of \
                     W17-array-typed-receiver territory. SliceAccess is \
                     not partitioned by receiver kind in the current \
                     bytecode emission; string slicing needs a \
                     dedicated op or a compile-time partition. ADR-006 \
                     §2.7.6 / §2.7.7."
                        .to_string(),
                ))
            }
            _ => {
                drop_with_kind(array_bits, array_kind);
                Err(VMError::NotImplemented(format!(
                    "SliceAccess: receiver kind {:?} — only \
                     Ptr(HeapKind::TypedArray) (Arc-boxed) and UInt64 \
                     (v2 raw pointer) are supported post-§2.7.7. \
                     Legacy generic-VW-array / unified-array dispatch \
                     was forbidden-pattern.",
                    array_kind
                )))
            }
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Receiver-location descriptor for `ArrayPushLocal`
// ───────────────────────────────────────────────────────────────────────────

/// Where the `ArrayPushLocal` receiver slot lives. Resolves the
/// instruction operand to the right slot store at op entry.
enum ReceiverLoc {
    Local(usize),
    ModuleBinding(usize),
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
        // ADR-006 §2.7.22 amendment (Round 18 S3): Matrix / FloatSlice
        // exit `TypedArrayData`.
        // W17-typed-carrier-bundle-A checkpoint 3/4: Q25.A specialized arms.
        TypedArrayData::Decimal(_) => NativeKind::Ptr(HeapKind::Decimal),
        TypedArrayData::BigInt(_) => NativeKind::Ptr(HeapKind::BigInt),
        TypedArrayData::DateTime(_)
        | TypedArrayData::Timespan(_)
        | TypedArrayData::Duration(_) => NativeKind::Ptr(HeapKind::Temporal),
        TypedArrayData::Instant(_) => NativeKind::Ptr(HeapKind::Instant),
        TypedArrayData::Char(_) => NativeKind::Ptr(HeapKind::Char),
        TypedArrayData::TypedObject(_) => NativeKind::Ptr(HeapKind::TypedObject),
        TypedArrayData::TraitObject(_) => NativeKind::Ptr(HeapKind::TraitObject),
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
        // ADR-006 §2.7.22 amendment (Round 18 S3): FloatSlice exits
        // `TypedArrayData`. Slicing a MatrixSlice receiver dispatches via
        // its dedicated HeapKind path; this typed-array slice path no
        // longer sees that receiver shape.
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

/// Slice a v2 typed array `[s, e)` into a freshly-allocated
/// `TypedArray<T>` of the same element type. Returns the raw pointer
/// (the slot carrier shape — `NativeKind::UInt64`). Empty/invalid
/// ranges produce an empty typed array of the matching element type
/// (which still carries the element-type stamp so downstream readers
/// can dispatch).
///
/// The element-type byte in the new array's header is set via
/// `stamp_elem_type` (matching the allocation path in
/// `executor/v2_handlers/array.rs`).
fn slice_v2_typed_array(
    view: &V2TypedArrayView,
    s: usize,
    e: usize,
) -> *mut u8 {
    use crate::executor::v2_handlers::v2_array_detect::stamp_elem_type;
    let (s, e) = if s <= e { (s, e) } else { (s, s) };
    match view.elem_type {
        V2ElemType::F64 => unsafe {
            let src = view.ptr as *const TypedArray<f64>;
            let slice: &[f64] = if s < e {
                let data = (*src).data as *const f64;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<f64>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_F64);
            new_ptr as *mut u8
        },
        V2ElemType::I64 => unsafe {
            let src = view.ptr as *const TypedArray<i64>;
            let slice: &[i64] = if s < e {
                let data = (*src).data as *const i64;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<i64>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_I64);
            new_ptr as *mut u8
        },
        V2ElemType::I32 => unsafe {
            let src = view.ptr as *const TypedArray<i32>;
            let slice: &[i32] = if s < e {
                let data = (*src).data as *const i32;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<i32>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_I32);
            new_ptr as *mut u8
        },
        V2ElemType::Bool => unsafe {
            let src = view.ptr as *const TypedArray<u8>;
            let slice: &[u8] = if s < e {
                let data = (*src).data as *const u8;
                std::slice::from_raw_parts(data.add(s), e - s)
            } else {
                &[]
            };
            let new_ptr = TypedArray::<u8>::from_slice(slice);
            stamp_elem_type(new_ptr as *mut u8, ELEM_TYPE_BOOL);
            new_ptr as *mut u8
        },
    }
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
