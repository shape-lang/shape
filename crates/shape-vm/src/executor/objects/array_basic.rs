//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse, clone, zip
//!
//! ## Wave-δ `MR-array-basic-query` body migration (playbook §10 / §7 REVISED)
//!
//! Wave-γ `G-method-fn-v2-abi` (merge `5091cba`) flipped `MethodFnV2` to the
//! kinded ABI per ADR-006 §2.7.10 / Q11:
//!
//! ```ignore
//! pub type MethodFnV2 = fn(
//!     &mut VirtualMachine,
//!     args: &[KindedSlot],
//!     Option<&mut ExecutionContext>,
//! ) -> Result<KindedSlot, VMError>;
//! ```
//!
//! `args[0]` is the receiver; `args[0].kind` is sourced by the dispatch
//! shell from the §2.7.7 stack parallel-Vec<NativeKind> track (no
//! fabrication, no tag_bits decode). For Array methods the receiver
//! kind is `NativeKind::Ptr(HeapKind::TypedArray)` and the bits are
//! `Arc::into_raw(Arc<TypedArrayData>)`.
//!
//! Reference templates:
//! - `executor/objects/array_operations.rs` (Wave-α D-array-ops) for the
//!   in-place mutation pattern via `Arc::increment_strong_count` +
//!   `Arc::from_raw` + `Arc::make_mut`.
//! - `executor/v2_handlers/typed_array_elem.rs` for the typed-array
//!   per-variant element kind classifier.
//!
//! ## Receiver-share discipline
//!
//! The dispatch shell owns one strong-count share per `args[i]`. The
//! handler borrows the slice; the carrier's `Drop` releases the share
//! after the handler returns. Inside the handler:
//!
//! - **Read-only access** (`len`, `first`, `last`, `clone`): borrow
//!   `&TypedArrayData` directly via `&*(bits as *const TypedArrayData)`.
//!   Do NOT reconstruct an Arc — that would consume the dispatch
//!   shell's share.
//! - **Mutable access** (`reverse`, `push`, `pop`): bump the refcount
//!   via `Arc::increment_strong_count`, reconstruct an owned Arc with
//!   `Arc::from_raw`, then `Arc::make_mut` — produces a fresh Arc that
//!   we own and can return as the result. The dispatch shell's share
//!   is independent and released by the carrier's Drop on return.
//! - **Result heap construction** (`reverse`, `push`, `clone`): build
//!   a fresh `Arc<TypedArrayData>` and call `KindedSlot::from_typed_array`
//!   per playbook §3.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers (kinded-API only; no shim usage)
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow the `&TypedArrayData` referenced by a `Ptr(HeapKind::TypedArray)`-
/// kinded receiver. The dispatch shell owns the strong-count share, so the
/// referent is alive for the borrow's lifetime.
#[inline]
fn typed_array_ref<'a>(slot: &'a KindedSlot) -> Result<&'a TypedArrayData, VMError> {
    match slot.kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let bits = slot.slot.raw();
            // SAFETY: per the kinded-ABI contract, when `kind ==
            // Ptr(HeapKind::TypedArray)` the bits are
            // `Arc::into_raw::<TypedArrayData>` and the dispatch shell
            // holds one strong-count share. Borrow the inner T for the
            // lifetime of `slot`.
            Ok(unsafe { &*(bits as *const TypedArrayData) })
        }
        _ => Err(VMError::TypeError {
            expected: "Array",
            got: "non-array",
        }),
    }
}

/// Bump the refcount on a `Ptr(HeapKind::TypedArray)`-kinded slot and
/// reconstruct an owned `Arc<TypedArrayData>`. Use this when the handler
/// needs to mutate via `Arc::make_mut` or transfer ownership of a fresh
/// share onto the result stack.
#[inline]
fn owned_typed_array_clone(slot: &KindedSlot) -> Result<Arc<TypedArrayData>, VMError> {
    match slot.kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let bits = slot.slot.raw();
            // SAFETY: bump the strong-count, then reconstruct an Arc that
            // owns the freshly bumped share. The dispatch shell's share is
            // independent.
            unsafe {
                Arc::increment_strong_count(bits as *const TypedArrayData);
                Ok(Arc::<TypedArrayData>::from_raw(bits as *const TypedArrayData))
            }
        }
        _ => Err(VMError::TypeError {
            expected: "Array",
            got: "non-array",
        }),
    }
}

/// Read the element at `idx` from a `TypedArrayData`, returning a kinded
/// result slot whose kind matches the variant's element kind. Per-variant
/// the bits encoding is the same as `array_operations.rs::pop_from_typed_array`.
fn read_element_at(arr: &TypedArrayData, idx: usize) -> Result<KindedSlot, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let v = buf.data.get(idx).copied().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_int(v))
        }
        TypedArrayData::F64(buf) => {
            let v = buf.data.get(idx).copied().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_number(v))
        }
        TypedArrayData::Bool(buf) => {
            let v = buf.data.get(idx).copied().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_bool(v != 0))
        }
        TypedArrayData::String(buf) => {
            let v = buf.data.get(idx).cloned().ok_or(VMError::IndexOutOfBounds {
                index: idx as i32,
                length: buf.data.len(),
            })?;
            Ok(KindedSlot::from_string_arc(v))
        }
        // Element-kind-aware reads for narrow-int / float-narrowed / matrix
        // / heap-value-backed / float-slice arrays remain Wave-γ-followup
        // territory (D-array-basic-class). Pre-Wave-6.5 the legacy body
        // also did not handle these uniformly; surface explicitly.
        other => Err(VMError::NotImplemented(format!(
            "array element read: TypedArrayData variant {} — Wave-γ-followup. \
             Element-kind-aware read for narrow-int / matrix / heap-value-\
             backed / float-slice arrays needs the §2.7.6 / Q8 per-variant \
             constructor matrix completed.",
            other.type_name()
        ))),
    }
}

/// Element count for a `TypedArrayData`, dispatching on the variant.
#[inline]
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
        // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A specialized arms.
        TypedArrayData::Decimal(b) => b.data.len(),
        TypedArrayData::BigInt(b) => b.data.len(),
        TypedArrayData::DateTime(b) => b.data.len(),
        TypedArrayData::Timespan(b) => b.data.len(),
        TypedArrayData::Duration(b) => b.data.len(),
        TypedArrayData::Instant(b) => b.data.len(),
        TypedArrayData::Char(b) => b.data.len(),
        TypedArrayData::TypedObject(b) => b.data.len(),
        TypedArrayData::TraitObject(b) => b.data.len(),
        TypedArrayData::Matrix(m) => m.data.len(),
        TypedArrayData::FloatSlice { len, .. } => *len as usize,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (kinded ABI per ADR-006 §2.7.10 / Q11) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.len()` / `arr.length()` — return element count as Int64.
pub(crate) fn handle_len_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = typed_array_ref(&args[0])?;
    Ok(KindedSlot::from_int(typed_array_len(arr) as i64))
}

/// `arr.first()` — first element, or empty-array error.
pub(crate) fn handle_first_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    if len == 0 {
        return Err(VMError::IndexOutOfBounds {
            index: 0,
            length: 0,
        });
    }
    read_element_at(arr, 0)
}

/// `arr.last()` — last element, or empty-array error.
pub(crate) fn handle_last_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = typed_array_ref(&args[0])?;
    let len = typed_array_len(arr);
    if len == 0 {
        return Err(VMError::IndexOutOfBounds {
            index: 0,
            length: 0,
        });
    }
    read_element_at(arr, len - 1)
}

/// `arr.reverse()` — return a new array with elements in reversed order.
///
/// Per the pre-Wave-6.5 contract `reverse` produces a fresh array (the
/// receiver is not mutated in-place from the caller's perspective; see
/// the JS `Array.prototype.reverse` semantics divergence noted in the
/// stdlib tests). We build a new Arc by cloning the receiver share and
/// `Arc::make_mut`-ing it, then reverse the underlying buffer in place.
pub(crate) fn handle_reverse_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let mut arc = owned_typed_array_clone(&args[0])?;
    match Arc::make_mut(&mut arc) {
        TypedArrayData::I64(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data.reverse();
        }
        TypedArrayData::F64(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data.reverse();
        }
        TypedArrayData::Bool(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data.reverse();
        }
        TypedArrayData::String(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data.reverse();
        }
        // Narrow-int / matrix / float-slice / heap-value variants need
        // element-kind-aware reverse (matrix is row-major; float-slice is
        // a view into a parent buffer, reversing requires materializing).
        // Pre-Wave-6.5 the legacy code only covered I64/F64/Bool/String
        // fast paths and fell through to forbidden generic-VW-array
        // behavior. Surface the gap explicitly per playbook §7 REVISED.
        other => {
            return Err(VMError::NotImplemented(format!(
                "Array.reverse: TypedArrayData variant {} — Wave-γ-followup. \
                 Per-variant reverse needs element-kind partitioning for \
                 narrow-int widths / matrix row-major layout / float-slice \
                 view materialization.",
                other.type_name()
            )));
        }
    }
    Ok(KindedSlot::from_typed_array(arc))
}

/// `arr.push(x)` — return a new array with `x` appended.
///
/// Like the pre-Wave-6.5 contract, this returns the (possibly newly-cloned
/// via `Arc::make_mut`) array. Receiver-side aliasing follows the
/// `array_operations.rs::push_into_typed_array` pattern: per-variant
/// element-kind validation, refuse cross-domain coercion (CLAUDE.md
/// "No runtime coercion").
pub(crate) fn handle_push_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::argument_count_error("push", 1, 0));
    }
    let mut arc = owned_typed_array_clone(&args[0])?;
    let value = &args[1];
    match Arc::make_mut(&mut arc) {
        TypedArrayData::I64(buf) => {
            let v = value.as_i64().ok_or(VMError::TypeError {
                expected: "int",
                got: "non-int element",
            })?;
            let buf = Arc::make_mut(buf);
            buf.data.push(v);
        }
        TypedArrayData::F64(buf) => {
            let v = value.as_f64().ok_or(VMError::TypeError {
                expected: "number",
                got: "non-number element",
            })?;
            let buf = Arc::make_mut(buf);
            buf.data.push(v);
        }
        TypedArrayData::Bool(buf) => {
            let v = value.as_bool().ok_or(VMError::TypeError {
                expected: "bool",
                got: "non-bool element",
            })?;
            let buf = Arc::make_mut(buf);
            buf.data.push(if v { 1u8 } else { 0u8 });
        }
        TypedArrayData::String(buf) => {
            // String element push: bump the receiver's `Arc<String>` share
            // (the `KindedSlot` we're reading is borrowed; we need our own
            // share to push into the buffer).
            let s_arc = match value.kind {
                NativeKind::String => {
                    let bits = value.slot.raw();
                    if bits == 0 {
                        return Err(VMError::TypeError {
                            expected: "string",
                            got: "null",
                        });
                    }
                    // SAFETY: `String`-kind slot bits are
                    // `Arc::into_raw::<String>`. Bump the share so the
                    // buffer push gets its own Arc; the carrier's Drop
                    // still releases the borrowed share independently.
                    unsafe {
                        Arc::increment_strong_count(bits as *const String);
                        Arc::<String>::from_raw(bits as *const String)
                    }
                }
                _ => {
                    return Err(VMError::TypeError {
                        expected: "string",
                        got: "non-string element",
                    });
                }
            };
            let buf = Arc::make_mut(buf);
            buf.data.push(s_arc);
        }
        // Narrow-int / float-narrowed / matrix / heap-value-backed /
        // float-slice push paths need element-kind partitioning the
        // pre-Wave-6.5 body never implemented end-to-end. Surface per
        // playbook §7 REVISED rather than silently accept a forbidden
        // fall-through.
        other => {
            return Err(VMError::NotImplemented(format!(
                "Array.push: TypedArrayData variant {} — Wave-γ-followup. \
                 Element-kind-aware push needs narrow-int width narrowing, \
                 matrix shape preservation, heap-value-backed retain-on-\
                 write, and float-slice view materialization.",
                other.type_name()
            )));
        }
    }
    Ok(KindedSlot::from_typed_array(arc))
}

/// `arr.pop()` — return the last element after popping it from the array.
///
/// The pre-Wave-6.5 contract returns just the popped element (the array
/// itself is consumed by the call). Empty array errors with
/// `IndexOutOfBounds`.
pub(crate) fn handle_pop_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let mut arc = owned_typed_array_clone(&args[0])?;
    match Arc::make_mut(&mut arc) {
        TypedArrayData::I64(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data
                .pop()
                .map(KindedSlot::from_int)
                .ok_or(VMError::IndexOutOfBounds {
                    index: 0,
                    length: 0,
                })
        }
        TypedArrayData::F64(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data
                .pop()
                .map(KindedSlot::from_number)
                .ok_or(VMError::IndexOutOfBounds {
                    index: 0,
                    length: 0,
                })
        }
        TypedArrayData::Bool(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data
                .pop()
                .map(|v| KindedSlot::from_bool(v != 0))
                .ok_or(VMError::IndexOutOfBounds {
                    index: 0,
                    length: 0,
                })
        }
        TypedArrayData::String(buf) => {
            let buf = Arc::make_mut(buf);
            buf.data
                .pop()
                .map(KindedSlot::from_string_arc)
                .ok_or(VMError::IndexOutOfBounds {
                    index: 0,
                    length: 0,
                })
        }
        other => Err(VMError::NotImplemented(format!(
            "Array.pop: TypedArrayData variant {} — Wave-γ-followup. \
             Element-kind-aware pop needs narrow-int width, heap-value-\
             backed retain-on-write, matrix shape preservation, and \
             float-slice view materialization.",
            other.type_name()
        ))),
    }
}

/// `a.zip(b)` — return an array of pair-shaped elements drawn from `a` and
/// `b` in lockstep. The result element type is heterogeneous (one drawn
/// from `a`'s element kind, one from `b`'s); the only `TypedArrayData`
/// variant that can carry heterogeneous payloads is `HeapValue`, which
/// requires wrapping each pair in `Arc<HeapValue::TypedArray(Arc<...>)>`
/// or similar — and the per-pair construction matrix needs the §2.7.6 /
/// Q8 per-variant constructor matrix completed for non-I64/F64/Bool/String
/// element kinds. Surface to Wave-γ-followup per playbook §7 REVISED.
pub(crate) fn handle_zip_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // Validate receiver kind so we surface a clean type-error for non-
    // array receivers rather than NotImplemented; the architectural gap
    // is the heterogeneous-pair construction, not the receiver.
    let _left = typed_array_ref(&args[0])?;
    if args.len() < 2 {
        return Err(VMError::argument_count_error("zip", 1, 0));
    }
    let _right = typed_array_ref(&args[1])?;
    Err(VMError::NotImplemented(
        "Array.zip — SURFACE: Wave-γ-followup (D-array-basic-class). \
         Pair construction requires heterogeneous element-kind handling: \
         the-deleted-heterogeneous-element-carrier is the only variant that admits mixed \
         per-element kinds, and each pair's two `KindedSlot`s need to be \
         wrapped as Arc<HeapValue::TypedArray(...)> per the §2.7.6 / Q8 \
         per-variant constructor matrix. The pre-Wave-6.5 body materialized \
         pairs through the deleted `vmarray_from_vec` + `ValueWord::pair` \
         helpers (forbidden #1)."
            .to_string(),
    ))
}

/// `arr.clone()` — return a fresh `KindedSlot` that bumps the receiver's
/// strong-count. Both the result and the dispatch-shell-owned receiver
/// share are independently released by their carriers' Drop impls.
pub(crate) fn handle_clone_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let bits = args[0].slot.raw();
            // SAFETY: bump the share so the result carrier and the
            // dispatch-shell's receiver carrier each own one
            // independent strong-count.
            unsafe {
                Arc::increment_strong_count(bits as *const TypedArrayData);
            }
            Ok(KindedSlot::new(
                ValueSlot::from_raw(bits),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        _ => Err(VMError::TypeError {
            expected: "Array",
            got: "non-array",
        }),
    }
}
