//! Focused proofs for the typed SharedCell allocator ABI.

use super::closure::{jit_alloc_shared_cell, jit_arc_shared_release, jit_arc_shared_retain};
use super::shared_cell_payload::{jit_read_shared_cell_ptr, jit_write_shared_cell_ptr};
use crate::ffi::stack_kind_code;
use shape_value::heap_value::{MatrixData, MatrixSliceData};
use shape_value::v2::closure_layout::SharedCell;
use shape_value::v2::heap_element::HeapElement;
use shape_value::v2::string_obj::StringObj;
use shape_value::{HeapKind, NativeKind};
use std::sync::Arc;

fn assert_zero_shared_payload_lifecycle(kind: NativeKind) {
    let code = stack_kind_code::encode(kind);
    assert_eq!(
        stack_kind_code::decode(code),
        Some(kind),
        "{kind:?}: encoded SharedCell kind must be decodable"
    );
    let cell_ptr = unsafe { jit_alloc_shared_cell(0, code) };
    assert_ne!(
        cell_ptr, 0,
        "{kind:?}: a valid code must allocate a non-null typed empty cell"
    );
    assert_eq!(
        unsafe { jit_read_shared_cell_ptr(cell_ptr as i64) },
        0,
        "{kind:?}: reading an empty payload must not fabricate a share"
    );
    unsafe {
        jit_write_shared_cell_ptr(cell_ptr as i64, 0);
        jit_arc_shared_release(cell_ptr);
    }
}

fn assert_arc_payload_lifecycle<T>(kind: NativeKind, initial: Arc<T>, replacement: Arc<T>) {
    let initial_weak = Arc::downgrade(&initial);
    let initial_bits = Arc::into_raw(initial) as u64;
    let cell_ptr = unsafe { jit_alloc_shared_cell(initial_bits, stack_kind_code::encode(kind)) };
    assert_ne!(cell_ptr, 0, "{kind:?}: valid allocation must be non-null");

    // Keep an observer share so the cell and its immutable kind remain
    // inspectable after the production outer-cell share is released.
    let observer = unsafe {
        Arc::increment_strong_count(cell_ptr as *const SharedCell);
        Arc::from_raw(cell_ptr as *const SharedCell)
    };
    let cell_weak = Arc::downgrade(&observer);
    assert_eq!(Arc::strong_count(&observer), 2);
    assert_eq!(initial_weak.strong_count(), 1, "cell owns initial payload");

    let read_bits = unsafe { jit_read_shared_cell_ptr(cell_ptr as i64) } as u64;
    assert_eq!(read_bits, initial_bits, "read preserves payload identity");
    assert_eq!(
        initial_weak.strong_count(),
        2,
        "canonical kinded read must mint one typed payload share"
    );
    unsafe { drop(Arc::from_raw(read_bits as *const T)) };
    assert_eq!(initial_weak.strong_count(), 1);

    let replacement_weak = Arc::downgrade(&replacement);
    let replacement_bits = Arc::into_raw(replacement) as u64;
    unsafe { jit_write_shared_cell_ptr(cell_ptr as i64, replacement_bits as i64) };
    assert_eq!(
        initial_weak.strong_count(),
        0,
        "canonical kinded replacement must retire the displaced share"
    );
    assert_eq!(
        replacement_weak.strong_count(),
        1,
        "the same cell must own the transferred replacement share"
    );
    assert_eq!(
        unsafe { *observer.value.get() },
        replacement_bits,
        "replacement changes payload, never SharedCell identity"
    );

    let replacement_read = unsafe { jit_read_shared_cell_ptr(cell_ptr as i64) } as u64;
    assert_eq!(replacement_read, replacement_bits);
    assert_eq!(replacement_weak.strong_count(), 2);
    unsafe { drop(Arc::from_raw(replacement_read as *const T)) };
    assert_eq!(replacement_weak.strong_count(), 1);

    unsafe { jit_arc_shared_release(cell_ptr) };
    assert_eq!(Arc::strong_count(&observer), 1);
    assert_eq!(replacement_weak.strong_count(), 1);
    drop(observer);
    assert_eq!(cell_weak.strong_count(), 0, "final cell share must retire");
    assert_eq!(
        replacement_weak.strong_count(),
        0,
        "final cell drop must retire the replacement payload"
    );
}

#[test]
fn typed_allocator_preserves_payload_kind_and_balances_cell_shares() {
    // The payload's sole Arc share is transferred into the cell. Observing its
    // Weak count proves `SharedCell::drop` used the allocator's String kind to
    // retire that exact share when the final cell share disappears.
    let payload = Arc::new(String::from("shape"));
    let payload_weak = Arc::downgrade(&payload);
    let payload_bits = Arc::into_raw(payload) as u64;

    let cell_ptr =
        unsafe { jit_alloc_shared_cell(payload_bits, stack_kind_code::encode(NativeKind::String)) };
    assert_ne!(cell_ptr, 0, "typed allocator must return a live cell");
    assert_eq!(cell_ptr % 8, 0, "SharedCell pointer must stay aligned");

    let capture_ptr = unsafe { jit_arc_shared_retain(cell_ptr) };
    assert_eq!(capture_ptr, cell_ptr, "retain must preserve cell identity");

    // Take one explicit observer share so strong-count transitions can be
    // checked without reconstructing either production-owned raw share.
    let observer = unsafe {
        Arc::increment_strong_count(cell_ptr as *const SharedCell);
        Arc::from_raw(cell_ptr as *const SharedCell)
    };
    let cell_weak = Arc::downgrade(&observer);
    assert_eq!(Arc::strong_count(&observer), 3);
    assert_eq!(
        unsafe { *observer.value.get() },
        payload_bits,
        "allocator must preserve the payload bits paired with the kind"
    );

    unsafe {
        jit_arc_shared_release(cell_ptr);
        jit_arc_shared_release(capture_ptr);
    }
    assert_eq!(
        Arc::strong_count(&observer),
        1,
        "outer and capture releases must leave only the observer"
    );
    assert_eq!(
        payload_weak.strong_count(),
        1,
        "the live cell must still own the payload share"
    );

    drop(observer);
    assert_eq!(cell_weak.strong_count(), 0, "final cell share must retire");
    assert_eq!(
        payload_weak.strong_count(),
        0,
        "typed cell drop must retire the String payload share"
    );
}

#[test]
fn invalid_kind_code_returns_null_without_consuming_payload_share() {
    let payload = Arc::new(String::from("still-owned"));
    let raw_share = Arc::into_raw(Arc::clone(&payload));
    assert_eq!(Arc::strong_count(&payload), 2);

    let cell_ptr = unsafe { jit_alloc_shared_cell(raw_share as u64, stack_kind_code::SENTINEL) };
    assert_eq!(
        cell_ptr, 0,
        "invalid kind evidence must not allocate a cell"
    );
    assert_eq!(
        Arc::strong_count(&payload),
        2,
        "a rejected (bits, kind) pair must leave payload ownership with the caller"
    );

    unsafe {
        drop(Arc::from_raw(raw_share));
    }
    assert_eq!(Arc::strong_count(&payload), 1);
}

#[test]
fn nonzero_native_scalar_is_rejected_without_allocation_or_consumption() {
    let bits = 0x0123_4567_89ab_cdef;
    let kind = NativeKind::Ptr(HeapKind::NativeScalar);
    assert_eq!(
        unsafe { jit_alloc_shared_cell(bits, stack_kind_code::encode(kind)) },
        0,
        "carrier-less NativeScalar metadata must fail before allocating a cell"
    );
}

#[test]
fn every_ptr_kind_is_either_supported_or_explicitly_rejected() {
    let mut checked = Vec::new();
    let mut rejected = Vec::new();
    let scalar_refcounted = [
        NativeKind::String,
        NativeKind::StringV2,
        NativeKind::DecimalV2,
    ];
    for kind in scalar_refcounted {
        assert_zero_shared_payload_lifecycle(kind);
        checked.push(kind);
    }
    for heap_kind in HeapKind::ALL {
        let kind = NativeKind::Ptr(heap_kind);
        if heap_kind.has_kinded_slot_carrier() {
            assert_zero_shared_payload_lifecycle(kind);
            checked.push(kind);
        } else {
            assert_eq!(
                unsafe { jit_alloc_shared_cell(1, stack_kind_code::encode(kind)) },
                0
            );
            rejected.push(kind);
        }
    }

    assert_eq!(
        checked.len() + rejected.len(),
        scalar_refcounted.len() + HeapKind::ALL.len()
    );
    assert_eq!(rejected, [NativeKind::Ptr(HeapKind::NativeScalar)]);
    assert!(checked.contains(&NativeKind::Ptr(HeapKind::Matrix)));
    assert!(checked.contains(&NativeKind::Ptr(HeapKind::MatrixSlice)));
}

#[test]
fn nonzero_inline_ptr_payloads_copy_without_fabricated_ownership() {
    for (kind, initial, replacement) in [
        (NativeKind::Ptr(HeapKind::Char), 'a' as u64, 'z' as u64),
        (NativeKind::Ptr(HeapKind::Future), 41, 42),
        (NativeKind::Ptr(HeapKind::ModuleFn), 7, 9),
    ] {
        let cell = unsafe { jit_alloc_shared_cell(initial, stack_kind_code::encode(kind)) };
        assert_ne!(cell, 0);
        assert_eq!(
            unsafe { jit_read_shared_cell_ptr(cell as i64) } as u64,
            initial
        );
        unsafe { jit_write_shared_cell_ptr(cell as i64, replacement as i64) };
        assert_eq!(
            unsafe { jit_read_shared_cell_ptr(cell as i64) } as u64,
            replacement
        );
        unsafe { jit_arc_shared_release(cell) };
    }
}

#[test]
fn nonzero_v2_header_payload_balances_retain_replace_and_drop() {
    let initial = StringObj::new("initial");
    let replacement = StringObj::new("replacement");
    unsafe {
        shape_value::v2::refcount::v2_retain(&(*initial).header);
        shape_value::v2::refcount::v2_retain(&(*replacement).header);
    }
    let kind = NativeKind::StringV2;
    let cell = unsafe { jit_alloc_shared_cell(initial as u64, stack_kind_code::encode(kind)) };
    assert_ne!(cell, 0);
    let read = unsafe { jit_read_shared_cell_ptr(cell as i64) } as *const StringObj;
    assert_eq!(read, initial);
    assert_eq!(unsafe { (*initial).header.get_refcount() }, 3);
    unsafe { StringObj::release_elem(read) };
    unsafe { jit_write_shared_cell_ptr(cell as i64, replacement as i64) };
    assert_eq!(unsafe { (*initial).header.get_refcount() }, 1);
    assert_eq!(unsafe { (*replacement).header.get_refcount() }, 2);
    unsafe { jit_arc_shared_release(cell) };
    assert_eq!(unsafe { (*replacement).header.get_refcount() }, 1);
    unsafe {
        StringObj::release_elem(initial);
        StringObj::release_elem(replacement);
    }
}

#[test]
fn refcounted_read_and_replacement_balance_payload_shares() {
    assert_arc_payload_lifecycle(
        NativeKind::String,
        Arc::new(String::from("initial")),
        Arc::new(String::from("replacement")),
    );
}

#[test]
fn matrix_zero_read_write_and_drop_use_canonical_typed_arc_dispatch() {
    let kind = NativeKind::Ptr(HeapKind::Matrix);
    assert_zero_shared_payload_lifecycle(kind);
    assert_arc_payload_lifecycle(
        kind,
        Arc::new(MatrixData::new(1, 2)),
        Arc::new(MatrixData::new(2, 1)),
    );
}

#[test]
fn matrix_slice_zero_read_write_and_drop_use_canonical_typed_arc_dispatch() {
    let kind = NativeKind::Ptr(HeapKind::MatrixSlice);
    assert_zero_shared_payload_lifecycle(kind);
    let initial_parent = Arc::new(MatrixData::new(1, 2));
    let replacement_parent = Arc::new(MatrixData::new(2, 1));
    assert_arc_payload_lifecycle(
        kind,
        Arc::new(MatrixSliceData::new(initial_parent, 0, 2)),
        Arc::new(MatrixSliceData::new(replacement_parent, 0, 2)),
    );
}

#[test]
fn nested_recapture_retains_one_cell_identity_and_one_payload_owner() {
    let payload = Arc::new(String::from("nested"));
    let payload_weak = Arc::downgrade(&payload);
    let payload_bits = Arc::into_raw(payload) as u64;
    let outer_slot =
        unsafe { jit_alloc_shared_cell(payload_bits, stack_kind_code::encode(NativeKind::String)) };
    let outer_capture = unsafe { jit_arc_shared_retain(outer_slot) };
    let inner_capture = unsafe { jit_arc_shared_retain(outer_capture) };

    assert_eq!(outer_capture, outer_slot);
    assert_eq!(inner_capture, outer_slot);
    let observer = unsafe {
        Arc::increment_strong_count(outer_slot as *const SharedCell);
        Arc::from_raw(outer_slot as *const SharedCell)
    };
    let cell_weak = Arc::downgrade(&observer);
    assert_eq!(
        Arc::strong_count(&observer),
        4,
        "outer slot, outer closure, inner closure, and observer own one cell each"
    );
    assert_eq!(
        payload_weak.strong_count(),
        1,
        "recapturing the cell must not duplicate its payload-owned share"
    );

    unsafe {
        jit_arc_shared_release(outer_slot);
        jit_arc_shared_release(outer_capture);
        jit_arc_shared_release(inner_capture);
    }
    assert_eq!(Arc::strong_count(&observer), 1);
    assert_eq!(
        payload_weak.strong_count(),
        1,
        "payload remains live until the last cell share retires"
    );
    drop(observer);
    assert_eq!(cell_weak.strong_count(), 0);
    assert_eq!(
        payload_weak.strong_count(),
        0,
        "SharedCell::drop retires the payload exactly once"
    );
}
