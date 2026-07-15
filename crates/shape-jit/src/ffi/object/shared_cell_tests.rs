//! Focused proofs for the typed SharedCell allocator ABI.

use super::closure::{jit_alloc_shared_cell, jit_arc_shared_release, jit_arc_shared_retain};
use super::shared_cell_payload::{jit_read_shared_cell_ptr, jit_write_shared_cell_ptr};
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

#[test]
fn zero_is_a_valid_empty_payload_for_every_refcounted_kind_code() {
    let mut checked = Vec::new();
    for code in 0..stack_kind_code::SENTINEL {
        let Some(kind) = stack_kind_code::decode(code) else {
            continue;
        };
        if !kind.is_refcounted() {
            continue;
        }

        let cell_ptr = unsafe { jit_alloc_shared_cell(0, code) };
        assert_ne!(
            cell_ptr, 0,
            "{kind:?}: zero must allocate a typed empty cell"
        );
        assert_eq!(
            unsafe { jit_read_shared_cell_ptr(cell_ptr as i64) },
            0,
            "{kind:?}: reading an empty refcounted payload must not fabricate a share"
        );
        unsafe {
            jit_write_shared_cell_ptr(cell_ptr as i64, 0);
            jit_arc_shared_release(cell_ptr);
        }
        checked.push(kind);
    }

    assert!(checked.contains(&NativeKind::String));
    assert!(checked.contains(&NativeKind::StringV2));
    assert!(checked.contains(&NativeKind::DecimalV2));
    assert!(
        checked
            .iter()
            .any(|kind| matches!(kind, NativeKind::Ptr(_))),
        "the proof must include the Ptr(HeapKind) family"
    );
}

#[test]
fn refcounted_read_and_replacement_balance_payload_shares() {
    let initial = Arc::new(String::from("initial"));
    let initial_weak = Arc::downgrade(&initial);
    let initial_bits = Arc::into_raw(initial) as u64;
    let cell_ptr =
        unsafe { jit_alloc_shared_cell(initial_bits, stack_kind_code::encode(NativeKind::String)) };

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
        "read must mint one typed payload share"
    );
    unsafe { drop(Arc::from_raw(read_bits as *const String)) };
    assert_eq!(initial_weak.strong_count(), 1);

    let replacement = Arc::new(String::from("replacement"));
    let replacement_weak = Arc::downgrade(&replacement);
    let replacement_bits = Arc::into_raw(replacement) as u64;
    unsafe { jit_write_shared_cell_ptr(cell_ptr as i64, replacement_bits as i64) };
    assert_eq!(
        initial_weak.strong_count(),
        0,
        "replacement must retire the displaced payload share"
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
    unsafe { drop(Arc::from_raw(replacement_read as *const String)) };
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
