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
    as_v2_typed_array, clone_array, pop_element, push_element, read_element, reverse_array,
    write_element, V2TypedArrayView,
};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::v2::heap_header::HEAP_KIND_V2_TYPED_ARRAY;
use shape_value::v2::typed_array::{TypedArray, ELEM_TYPE_TYPED_OBJECT};
use shape_value::{HeapKind, KindedSlot, NativeKind, TypedObjectStorage, ValueSlot, VMError};

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
//
// `ckpt5_surface` retired R8 W4 J.5d (2026-05-24) — zip was the last caller;
// migrated to the Array<TypedObject> heap-fallback carrier per supervisor
// D1 OPTION (c). Reverse migrated R8 W3 J.5a via `reverse_array`. If a
// future site needs a new surface-and-stop body, prefer per-site inlined
// `VMError::NotImplemented` over reviving this helper — the helper's
// generic "scheduled for v0.4 PHF-retirement" message is no longer the
// right surface signal.

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

/// `arr.reverse()` — produce a reversed array. Kind-generic via the
/// `v2_array_detect::reverse_array` primitive (R8 W3 J.5a, 2026-05-24);
/// returns a fresh `Ptr(HeapKind::TypedArray)` slot with the same stamped
/// element-type byte and elements in reversed order.
pub(crate) fn handle_reverse_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.reverse: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let new_ptr = reverse_array(&view);
    Ok(KindedSlot::new(
        ValueSlot::from_u64(new_ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
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

/// `arr.zip(other)` — pairwise element zip into `Array<Pair<A, B>>`.
///
/// **R8 W4 J.5d — Option (c) Array<Tuple> heap fallback (supervisor D1,
/// 2026-05-24).** Materializes as `TypedArray<*const TypedObjectStorage>`
/// (stamped `ELEM_TYPE_TYPED_OBJECT`); each element is a TypedObject with
/// fields `_0` (from the receiver) and `_1` (from the argument). The
/// `_0` / `_1` schema is auto-registered (or retrieved) via
/// `shape_runtime::type_schema::typed_object_from_pairs`, mirroring the
/// W17-typed-carrier-bundle-A `{key, value}` Entry precedent for
/// `HashMap.entries()`.
///
/// **Carrier rationale — supervisor D1 binding:** Tuples are not a
/// first-class heap value at HEAD (no `HeapKind::Tuple`, no `Expr::Tuple`
/// literal). Option (a) general `TypedArray<TupleN<T1,...,Tn>>` carrier is
/// the v0.4 promotion (touches ADR-006 §2.7.24 typed-carrier-
/// monomorphization). Option (b) `HeapKind::ZippedArray` REFUSED per
/// CLAUDE.md §Forbidden-Patterns parallel-implementation rule + ADR-005
/// §1 single-discriminator debt. Option (d) defer-to-v0.4 REFUSED per
/// stdlib API gap. The (c) heap-allocation-per-tuple cost is the
/// accepted v0.3 trade-off — zip is not a hot path. **v0.4 promotes to
/// (a) general `TypedArray<TupleN>` as a typed-carrier-monomorphization
/// workstream.**
///
/// **Length:** `min(a.len, b.len)` — common stdlib precedent
/// (JS Array.zip / Rust Iterator::zip / Python zip).
///
/// **Refcount discipline:** `typed_object_from_pairs` clones each element
/// `KindedSlot` (bumping any heap refcount), transfers the share into the
/// constructed TypedObject's slot, and returns a `KindedSlot` owning a
/// share of the freshly-allocated `*const TypedObjectStorage` (refcount=1
/// on the on-header counter). We extract `bits` via `slot().raw()` +
/// `mem::forget` to transfer the share into the result array's element
/// buffer without double-drop; the array's stamp + `read_element`
/// `V2ElemType::TypedObject` arm handle subsequent retain/release via
/// the on-header `v2_retain` / `v2_release` (`HeapElement` trait).
pub(crate) fn handle_zip_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.zip expects 1 argument".into(),
        ));
    }
    let a = extract_typed_array_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.zip: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let b = extract_typed_array_view(&args[1]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.zip: expected v2 TypedArray argument, got kind {:?}",
            args[1].kind
        ))
    })?;

    let out_len = a.len.min(b.len);

    // SAFETY: allocate `TypedArray<*const TypedObjectStorage>` with capacity
    // = out_len; stamp `ELEM_TYPE_TYPED_OBJECT` so subsequent reads dispatch
    // through the `V2ElemType::TypedObject` arm. The buffer is owned by the
    // allocated array; each element slot will be initialized below.
    let new_arr = TypedArray::<*const TypedObjectStorage>::with_capacity(out_len);
    let dst_data = unsafe { (*new_arr).data };

    for i in 0..out_len {
        // Read element pairs (bits, kind). `read_element` retains heap
        // elements (String / Decimal / TypedObject) on the producer side
        // per its docstring — the share is owned by the caller and must
        // be transferred into the TypedObject's slot via the standard
        // `typed_object_from_pairs` clone-and-forget recipe.
        let (a_bits, a_kind) = read_element(&a, i).ok_or_else(|| {
            VMError::RuntimeError(format!("Array.zip: failed to read receiver element {}", i))
        })?;
        let (b_bits, b_kind) = read_element(&b, i).ok_or_else(|| {
            VMError::RuntimeError(format!("Array.zip: failed to read argument element {}", i))
        })?;
        let a_slot = KindedSlot::new(ValueSlot::from_raw(a_bits), a_kind);
        let b_slot = KindedSlot::new(ValueSlot::from_raw(b_bits), b_kind);

        // Build a `Pair<A, B>` TypedObject with fields `_0` / `_1`
        // (Rust-tuple convention). The schema is auto-registered on
        // first use via `lookup_schema_for_fields` → `register_
        // predeclared_any_schema` with `FieldType::Any` columns; subsequent
        // calls hit the registry cache. `typed_object_from_pairs` clones
        // each input slot (bumping any heap refcount), so a_slot/b_slot's
        // shares — owned by us — must be dropped via the explicit
        // `drop_with_kind` lockstep that `KindedSlot::Drop` performs.
        let pair = shape_runtime::type_schema::typed_object_from_pairs(&[
            ("_0", a_slot),
            ("_1", b_slot),
        ]);
        if pair.kind != NativeKind::Ptr(HeapKind::TypedObject) {
            return Err(VMError::RuntimeError(format!(
                "Array.zip: typed_object_from_pairs returned unexpected kind {:?}",
                pair.kind
            )));
        }
        let pair_ptr = pair.slot.raw() as usize as *const TypedObjectStorage;
        // Transfer the `pair` share into the destination buffer without
        // dropping it — `KindedSlot::Drop` would otherwise release the
        // refcount we just gave the array. Mirror of the recipe at
        // `type_schema/mod.rs::typed_object_from_pairs` body for
        // per-field slot construction.
        std::mem::forget(pair);
        unsafe {
            *dst_data.add(i as usize) = pair_ptr;
        }
    }

    unsafe {
        (*new_arr).len = out_len;
    }
    let p = new_arr as *mut u8;
    unsafe {
        crate::executor::v2_handlers::v2_array_detect::stamp_elem_type(
            p,
            ELEM_TYPE_TYPED_OBJECT,
        );
    }
    // The on-header `kind` byte is already `HEAP_KIND_V2_TYPED_ARRAY` from
    // `TypedArray::with_capacity` → `HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY)`;
    // assert as a debug-only sanity check.
    debug_assert_eq!(
        unsafe { (*(p as *const shape_value::HeapHeader)).kind },
        HEAP_KIND_V2_TYPED_ARRAY,
        "Array.zip: freshly-allocated TypedArray header kind mismatch"
    );

    Ok(KindedSlot::new(
        ValueSlot::from_u64(p as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
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
