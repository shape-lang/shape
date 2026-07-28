//! The foreign-reference carrier's VM-side share accounting.
//!
//! ADR-019 §3 / #200 (POLY-FOREIGN-REF); ADR-006 §2.7.32 / Q26.
//!
//! `shape-value`'s own tests cover the `KindedSlot` and `SharedCell` tables.
//! These cover the VM stack's §2.7.7 parallel-kind track — the table a value
//! actually travels through while a Shape program runs — and they observe the
//! thing that makes this carrier different from every other `HeapKind`: the
//! last release does not merely free memory, it disposes an object inside
//! another runtime. A miscounted share here is a foreign-side leak or a
//! use-after-free that no Rust tooling would see.

use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};
use crate::type_tracking::NativeKind;
use shape_value::heap_value::HeapKind;
use shape_value::{ForeignRefData, ForeignRefDisposer, ForeignRefOrigin};
use std::sync::{Arc, Mutex};

/// Records the handles it is asked to dispose, in order.
#[derive(Debug, Default)]
struct RecordingDisposer {
    disposed: Mutex<Vec<u64>>,
}

impl ForeignRefDisposer for RecordingDisposer {
    fn dispose(&self, handle: u64) {
        self.disposed.lock().unwrap().push(handle);
    }
}

impl RecordingDisposer {
    fn handles(&self) -> Vec<u64> {
        self.disposed.lock().unwrap().clone()
    }
}

fn origin(produced_by: &str) -> ForeignRefOrigin {
    ForeignRefOrigin {
        language: "python".into(),
        foreign_type: "object".into(),
        produced_by: produced_by.into(),
    }
}

/// Build a foreign ref and hand back its slot bits, transferring the share.
fn foreign_ref_bits(handle: u64, disposer: Arc<RecordingDisposer>) -> u64 {
    Arc::into_raw(Arc::new(ForeignRefData::new(
        handle,
        origin("fixture"),
        disposer,
    ))) as u64
}

const FOREIGN_REF: NativeKind = NativeKind::Ptr(HeapKind::ForeignRef);

/// Table 1 — `clone_with_kind` / `drop_with_kind` (ADR-006 §2.7.7).
#[test]
fn stack_clone_and_drop_dispose_exactly_once() {
    let recorder = Arc::new(RecordingDisposer::default());
    let bits = foreign_ref_bits(41, recorder.clone());

    // Two stack slots now name the same foreign object.
    clone_with_kind(bits, FOREIGN_REF);
    assert!(
        recorder.handles().is_empty(),
        "duplicating a slot must not dispose"
    );

    drop_with_kind(bits, FOREIGN_REF);
    assert!(
        recorder.handles().is_empty(),
        "one live slot remains, so the foreign object is still owned"
    );

    drop_with_kind(bits, FOREIGN_REF);
    assert_eq!(
        recorder.handles(),
        vec![41],
        "the last stack slot disposes, once — a second disposal here would be \
         a double-free on the foreign side"
    );
}

/// Drop order follows ownership: two independent references are disposed in
/// the order their last shares are retired, and each is disposed once.
///
/// ADR-010 §4 ("observable order follows ownership, not layout") is what makes
/// this assertable at all — the foreign runtime sees releases in the order
/// Shape's scopes end, not in allocation or layout order.
#[test]
fn independent_references_dispose_in_release_order() {
    let recorder = Arc::new(RecordingDisposer::default());
    let first = foreign_ref_bits(1, recorder.clone());
    let second = foreign_ref_bits(2, recorder.clone());

    drop_with_kind(second, FOREIGN_REF);
    drop_with_kind(first, FOREIGN_REF);

    assert_eq!(
        recorder.handles(),
        vec![2, 1],
        "each reference disposes exactly once, in the order its last share went"
    );
}

/// A reference whose share is duplicated across a nested scope outlives the
/// inner scope's release — the escape case ADR-006 §2.7.30 describes for
/// ordinary values, with a foreign disposal attached to the end of it.
#[test]
fn a_duplicated_share_defers_disposal_past_the_inner_release() {
    let recorder = Arc::new(RecordingDisposer::default());
    let bits = foreign_ref_bits(7, recorder.clone());

    // Inner scope takes its own share...
    clone_with_kind(bits, FOREIGN_REF);
    // ...outer scope releases first.
    drop_with_kind(bits, FOREIGN_REF);
    assert!(
        recorder.handles().is_empty(),
        "the inner share keeps the foreign object alive"
    );

    drop_with_kind(bits, FOREIGN_REF);
    assert_eq!(recorder.handles(), vec![7]);
}
