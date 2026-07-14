//! Focused proofs for the typed SharedCell allocator ABI.

use super::closure::{jit_alloc_shared_cell, jit_arc_shared_release, jit_arc_shared_retain};
use crate::ffi::stack_kind_code;
use shape_value::NativeKind;
use shape_value::v2::closure_layout::SharedCell;
use std::sync::Arc;

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
