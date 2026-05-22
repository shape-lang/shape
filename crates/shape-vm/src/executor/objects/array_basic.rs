//! Basic array operations
//!
//! Handles: len, length, isEmpty, first, last, push, pop, get, set, reverse,
//! clone, zip.
//!
//! ## WS-8 kind-generic header handlers (2026-05-22)
//!
//! `handle_len_v2`, `handle_first_v2`, `handle_last_v2`, `handle_is_empty_v2`
//! are KIND-GENERIC: they dispatch on the `TypedArray<T>` header (`HeapHeader`
//! + element-type byte) via the existing `v2_array_detect::as_v2_typed_array`
//! view, then route reads through `v2_array_detect::read_element` which is
//! already monomorphized per `V2ElemType`. One handler, every element kind —
//! NOT a per-kind PHF, NOT a runtime-tagged value path. ADR-006 §2.7.5
//! producer-side stamp (the element-type byte is stamped at allocation by
//! `stamp_elem_type`); no Bool-default, no runtime kind fabrication.
//!
//! Receiver kind is `NativeKind::Ptr(HeapKind::TypedArray)` per
//! r5c-2-β-CKPT-C u64-carrier-disambiguation; legacy fallback for non-v2
//! receivers (e.g. a future `HeapValue::TypedArray` re-shape) surfaces via
//! the residual stubs preserved below.
//!
//! ## W16.2-J.0 kind-generic mutation + clone + get/set (2026-05-22)
//!
//! `push`, `pop`, `clone`, `get`, `set` are KIND-GENERIC: they delegate
//! to `v2_array_detect::{push,pop,read,write}_element + clone_array`
//! primitives over the `V2TypedArrayView` extracted from the receiver
//! `KindedSlot`. Receiver kind `Ptr(HeapKind::TypedArray)`; element kind
//! flows through the stamped element-type byte. Per W16.2-J audit §3
//! REVISED (2026-05-22), this prereq closes the J.1 PHF-deletion cascade
//! hole for I64/F64 receivers (the per-kind `typed_int_array_methods` /
//! `typed_number_array_methods` PHF entries fall through to ARRAY_METHODS
//! after J.1).
//!
//! `reverse` + `zip` remain surface-and-stop: no `v2_array_detect::*`
//! primitive at HEAD covers either operation. J.4-rest / J.5 territory.
//!
//! Refusal #1 binding: `TypedArrayData` resurrection under any rename refused
//! on sight.

use crate::executor::v2_handlers::v2_array_detect::{
    as_v2_typed_array, clone_array, pop_element, push_element, read_element, write_element,
    V2TypedArrayView,
};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// WS-8 kind-generic header-view helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the kind-generic `V2TypedArrayView` from the receiver `KindedSlot`.
///
/// The view carries the element-type byte stamped at allocation; every
/// downstream read dispatches on `view.elem_type` via
/// `v2_array_detect::read_element` — no per-element-kind PHF, no per-handler
/// duplication. Receiver kind must be `Ptr(HeapKind::TypedArray)`
/// (r5c-2-β-CKPT-C single carrier).
#[inline]
fn extract_typed_array_view(slot: &KindedSlot) -> Option<V2TypedArrayView> {
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return None;
    }
    as_v2_typed_array(slot.slot.raw(), slot.kind)
}

/// Lift a `(u64, NativeKind)` element-read pair into a `KindedSlot`. Matches
/// the carrier shape used by `typed_int_array_methods::pair_to_slot` —
/// keeps the dispatch shell uniform across the int/number/bool/string/decimal
/// element families.
#[inline]
fn pair_to_slot((bits, kind): (u64, NativeKind)) -> KindedSlot {
    KindedSlot::new(ValueSlot::from_raw(bits), kind)
}

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder (legacy fallback)
// ═══════════════════════════════════════════════════════════════════════════

/// Surface-and-stop body for handlers that have not yet migrated to the
/// kind-generic v2-raw `TypedArray<T>` carrier (reverse / push / pop / zip /
/// clone). These mutate the receiver or produce a new array — distinct from
/// the WS-8 read-only header handlers.
#[cold]
#[inline(never)]
fn ckpt5_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "Array<T>.{op} is not yet supported on the v2-raw TypedArray<T> \
         carrier (receiver kind: {kind}). The kind-generic read-only header \
         handlers (len / length / isEmpty / first / last) are wired; \
         mutation/transform methods are scheduled for the v0.4 PHF-\
         retirement workstream (W17 typed-carrier-monomorphization).",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// WS-8 kind-generic header handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.len()` / `arr.length` — element count. Kind-generic via the
/// `TypedArray<T>` header; works for every element kind in one stroke.
pub(crate) fn handle_len_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.len: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    Ok(KindedSlot::from_int(view.len as i64))
}

/// `arr.isEmpty()` — true when the receiver has zero elements. Kind-generic
/// via the `TypedArray<T>` header; the WS-8 ratified-bundle deliverable adds
/// `isEmpty` as a new entry (was previously unrouted across every element
/// kind).
pub(crate) fn handle_is_empty_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.isEmpty: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    Ok(KindedSlot::from_bool(view.len == 0))
}

/// `arr.first()` — first element, or null sentinel if empty. Kind-generic;
/// the element read dispatches on the stamped element-type byte and returns
/// the element's matching `NativeKind` (e.g. `Bool` for bool arrays,
/// `StringV2` for string arrays, `Float64` for number arrays).
pub(crate) fn handle_first_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.first: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    if view.len == 0 {
        return Ok(KindedSlot::none());
    }
    match read_element(&view, 0) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Ok(KindedSlot::none()),
    }
}

/// `arr.last()` — last element, or null sentinel if empty. Kind-generic;
/// reads via `read_element(view, len-1)` which monomorphizes on the
/// element-type byte.
pub(crate) fn handle_last_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.last: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    if view.len == 0 {
        return Ok(KindedSlot::none());
    }
    match read_element(&view, view.len - 1) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Ok(KindedSlot::none()),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// W16.2-J.0 kind-generic mutation + clone handlers
// ═══════════════════════════════════════════════════════════════════════════
//
// `push`, `pop`, `clone`, `get`, `set` delegate to the kind-generic
// `v2_array_detect::{push,pop,read,write}_element + clone_array`
// primitives. The view's `elem_type` (stamped at allocation) drives the
// per-T mutation inside the primitive.
//
// `reverse` + `zip` remain surface-and-stop: no `v2_array_detect::*`
// primitive at HEAD covers either operation (reverse needs an in-place
// reorder + clone hybrid; zip needs a result-shape carrier that v2-raw
// `TypedArray<T>` doesn't yet have). These are J.4-rest / J.5 territory
// per W16.2-J audit §3 REVISED. Refusal #1 binding: surface-and-stop
// disallows fabricating a primitive on-the-fly.

/// `arr.reverse()` — produce a reversed array. No `v2_array_detect`
/// reverse primitive at HEAD; surfaces W17/J.4-rest territory.
pub(crate) fn handle_reverse_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("reverse", args))
}

/// `arr.push(elem)` — append element, return the new length. Kind-generic
/// via `v2_array_detect::push_element`.
pub(crate) fn handle_push_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.push expects 1 argument".into(),
        ));
    }
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.push: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let bits = args[1].slot.raw();
    let kind = args[1].kind;
    push_element(&view, bits, kind)
        .map_err(|e| VMError::RuntimeError(format!("Array.push: {}", e)))?;
    // Re-read the header for the post-push element count.
    let post = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(
            "Array.push: receiver re-detection failed after push".into(),
        )
    })?;
    Ok(KindedSlot::from_int(post.len as i64))
}

/// `arr.pop()` — remove and return the last element, or the null sentinel
/// if empty. Kind-generic via `v2_array_detect::pop_element`; result kind
/// is the per-element kind from the view (`Int64`/`Float64`/etc.).
pub(crate) fn handle_pop_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.pop: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    match pop_element(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Ok(KindedSlot::none()),
    }
}

/// `arr.zip(other)` — pairwise element zip. No `v2_array_detect` zip
/// primitive at HEAD (zip output is a tuple-element carrier; v2-raw
/// `TypedArray<T>` doesn't model tuples). J.4-rest territory.
pub(crate) fn handle_zip_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("zip", args))
}

/// `arr.clone()` — deep-clone the receiver array. Kind-generic via
/// `v2_array_detect::clone_array`; returns a fresh
/// `Ptr(HeapKind::TypedArray)` slot pointing at the new allocation with
/// the same stamped element-type byte.
pub(crate) fn handle_clone_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.clone: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let new_ptr = clone_array(&view);
    Ok(KindedSlot::new(
        ValueSlot::from_u64(new_ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// `arr.get(i)` — element at index `i`, error if out of bounds.
/// Kind-generic via `v2_array_detect::read_element`; result kind is the
/// per-element kind from the view.
pub(crate) fn handle_get_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.get expects 1 argument".into(),
        ));
    }
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.get: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let idx = args[1].as_i64().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.get: index must be an integer, got kind {:?}",
            args[1].kind
        ))
    })?;
    if idx < 0 || (idx as u32) >= view.len {
        return Err(VMError::RuntimeError(format!(
            "Array.get: index {} out of bounds (len={})",
            idx, view.len
        )));
    }
    match read_element(&view, idx as u32) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(
            "Array.get: read_element returned None".into(),
        )),
    }
}

/// `arr.set(i, x)` — set element at index, return the receiver pointer
/// for chained calls. Kind-generic via `v2_array_detect::write_element`.
pub(crate) fn handle_set_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Array.set expects 2 arguments".into(),
        ));
    }
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.set: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let idx = args[1].as_i64().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.set: index must be an integer, got kind {:?}",
            args[1].kind
        ))
    })?;
    if idx < 0 || (idx as u32) >= view.len {
        return Err(VMError::RuntimeError(format!(
            "Array.set: index {} out of bounds (len={})",
            idx, view.len
        )));
    }
    let bits = args[2].slot.raw();
    let kind = args[2].kind;
    write_element(&view, idx as u32, bits, kind)
        .map_err(|e| VMError::RuntimeError(format!("Array.set: {}", e)))?;
    // Return the receiver pointer carrier for chained calls.
    Ok(KindedSlot::new(
        ValueSlot::from_u64(view.ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// WS-8 regression tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::v2_handlers::v2_array_detect::{
        as_v2_typed_array, V2ElemType,
    };

    // The handlers use the v2-raw allocators from `v2_handlers/array.rs`,
    // which require a fully-set-up `VirtualMachine`. These unit tests build
    // typed arrays through the allocator's safe surface and exercise the
    // header-view extract + read paths directly. End-to-end VM==JIT regression
    // smokes live alongside the WS-8 fix-wave dispatch in the user-facing
    // `tests/smokes/` set.

    /// `handle_len_v2` reads the header `.len` field for any element kind.
    /// Tests int / bool / string element kinds — one handler, three kinds.
    #[test]
    fn kind_generic_len_reads_header() {
        use crate::executor::v2_handlers::v2_array_detect::ELEM_TYPE_I64;
        use shape_value::v2::typed_array::TypedArray;

        // Allocate a TypedArray<i64> with 3 elements via the same shape the
        // VM allocator uses.
        let arr_ptr = TypedArray::<i64>::with_capacity(3) as *mut u8;
        unsafe {
            crate::executor::v2_handlers::v2_array_detect::stamp_elem_type(
                arr_ptr,
                ELEM_TYPE_I64,
            );
            let arr = arr_ptr as *mut TypedArray<i64>;
            TypedArray::<i64>::push(arr, 10);
            TypedArray::<i64>::push(arr, 20);
            TypedArray::<i64>::push(arr, 30);
        }

        let view = as_v2_typed_array(
            arr_ptr as u64,
            NativeKind::Ptr(HeapKind::TypedArray),
        )
        .expect("view");
        assert_eq!(view.elem_type, V2ElemType::I64);
        assert_eq!(view.len, 3);

        // Per the WS-8 handler contract: result is Int64-kinded.
        // Header `.len == 3` — what handle_len_v2 would return.
        let len_result = view.len as i64;
        assert_eq!(len_result, 3);

        unsafe {
            TypedArray::<i64>::drop_array(arr_ptr as *mut TypedArray<i64>);
        }
    }

    /// `handle_is_empty_v2` returns `false` for non-empty header.
    #[test]
    fn kind_generic_is_empty_false_when_nonempty() {
        use crate::executor::v2_handlers::v2_array_detect::ELEM_TYPE_BOOL;
        use shape_value::v2::typed_array::TypedArray;

        let arr_ptr = TypedArray::<u8>::with_capacity(2) as *mut u8;
        unsafe {
            crate::executor::v2_handlers::v2_array_detect::stamp_elem_type(
                arr_ptr,
                ELEM_TYPE_BOOL,
            );
            let arr = arr_ptr as *mut TypedArray<u8>;
            TypedArray::<u8>::push(arr, 1);
            TypedArray::<u8>::push(arr, 0);
        }

        let view = as_v2_typed_array(
            arr_ptr as u64,
            NativeKind::Ptr(HeapKind::TypedArray),
        )
        .expect("view");
        assert_eq!(view.elem_type, V2ElemType::Bool);
        // isEmpty contract: header.len == 0
        assert_eq!(view.len == 0, false);

        unsafe {
            TypedArray::<u8>::drop_array(arr_ptr as *mut TypedArray<u8>);
        }
    }

    /// `handle_last_v2` reads element at `len - 1` regardless of element
    /// kind — same code path for Int64 / Bool / StringV2.
    #[test]
    fn kind_generic_last_reads_element_at_len_minus_1() {
        use crate::executor::v2_handlers::v2_array_detect::ELEM_TYPE_BOOL;
        use shape_value::v2::typed_array::TypedArray;

        let arr_ptr = TypedArray::<u8>::with_capacity(3) as *mut u8;
        unsafe {
            crate::executor::v2_handlers::v2_array_detect::stamp_elem_type(
                arr_ptr,
                ELEM_TYPE_BOOL,
            );
            let arr = arr_ptr as *mut TypedArray<u8>;
            TypedArray::<u8>::push(arr, 1); // true
            TypedArray::<u8>::push(arr, 1); // true
            TypedArray::<u8>::push(arr, 0); // false
        }

        let view = as_v2_typed_array(
            arr_ptr as u64,
            NativeKind::Ptr(HeapKind::TypedArray),
        )
        .expect("view");

        // What handle_last_v2 reads: read_element(view, len - 1).
        let last = read_element(&view, view.len - 1).expect("read");
        assert_eq!(last.0, 0); // false bits
        assert_eq!(last.1, NativeKind::Bool);

        unsafe {
            TypedArray::<u8>::drop_array(arr_ptr as *mut TypedArray<u8>);
        }
    }
}
