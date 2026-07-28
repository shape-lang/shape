//! The opaque foreign-reference carrier.
//!
//! ADR-019 §3 / R25 (POLY-FOREIGN-REF, issue #200); ADR-006 §2.7.32 / Q26.
//!
//! A [`ForeignRefData`] is Shape's handle on an object that lives inside a
//! foreign runtime — a Python object, a JavaScript value. Shape never sees the
//! object's representation: it holds an extension-minted `handle`, the origin
//! facts needed to name the value in a diagnostic, and the disposal authority
//! that returns the object to its owner when the last Shape share goes away.
//!
//! # Why the disposer is injected rather than looked up
//!
//! `shape-value` sits below the plugin layer: it cannot reach a
//! `PluginLanguageRuntime`, an extension vtable, or the async worker pool, and
//! ADR-006's dependency direction says it must not learn to. But drop runs
//! *here* — in the `HeapKind` dispatch tables — with no VM in scope. So the
//! carrier binds its disposal authority at construction, as an owned
//! [`ForeignRefDisposer`], and `Drop` calls it. The layer that knows how to
//! reach the owning instance (`shape-vm`'s foreign-call path) is the layer that
//! supplies it.
//!
//! That is also what makes thread-affine disposal expressible: a foreign object
//! minted inside a dedicated worker's V8 isolate belongs to *that worker's*
//! instance, and the disposer it was built with is the one that routes back to
//! it. The carrier does not need to know which case it is in.
//!
//! # Lifecycle
//!
//! Slot bits are `Arc::into_raw(Arc<ForeignRefData>) as u64`, labelled
//! `NativeKind::Ptr(HeapKind::ForeignRef)`. This is the pure-discriminator
//! shape ADR-006 §2.7.9 established for `HeapKind::FilterExpr`: this kind has
//! no `HeapValue` arm at all, `as_heap_value()` is unsound on
//! ForeignRef-labelled bits, and every retain/release goes through
//! `Arc::increment_strong_count::<ForeignRefData>` /
//! `Arc::decrement_strong_count::<ForeignRefData>` in the kind-dispatch tables.
//!
//! Disposal happens exactly once, when the last `Arc` share is retired, through
//! ordinary `Drop`. Per ADR-019 §3 dispose is synchronous and infallible in v1:
//! a disposer that could fail or suspend is a later design under ADR-010 §6's
//! finalization contract, not a v1 surface.

use std::sync::Arc;

/// The disposal authority for one foreign object.
///
/// Implemented by the layer that can reach the owning extension instance. The
/// carrier owns one of these and calls [`dispose`](Self::dispose) exactly once,
/// from `Drop`.
///
/// # Contract
///
/// - **Synchronous.** When `dispose` returns, the owning runtime has been told
///   to release the object. ADR-019 §3 fixes this for v1 so that a Shape scope
///   exit is a real release point rather than a hint; an implementation that
///   must reach another thread blocks until that thread has taken the request.
/// - **Infallible.** There is no way to report a disposal failure from `Drop`,
///   and inventing one would make teardown partial. An implementation that
///   cannot reach its instance — because the owning worker is already gone, and
///   with it the whole foreign heap the handle pointed into — has nothing left
///   to release and returns.
/// - **Reentrancy-free.** `dispose` must not run Shape code, take VM locks, or
///   drop another `ForeignRefData`.
pub trait ForeignRefDisposer: Send + Sync + std::fmt::Debug {
    /// Release the foreign object behind `handle`.
    fn dispose(&self, handle: u64);
}

/// Where a foreign reference came from.
///
/// Carried on every reference because ADR-019 §3 requires refusals to *name the
/// value and its origin*: a snapshot that hits a live foreign ref, and a remote
/// artifact that would have to carry one, both have to say which extension
/// minted it and which declaration produced it. Origin facts that are only
/// reconstructible from VM context are not available at those refusal sites, so
/// they travel with the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignRefOrigin {
    /// The extension's language id — `"python"`, `"typescript"`.
    pub language: Arc<str>,
    /// The foreign-side type as the extension described it (`"module"`,
    /// `"numpy.ndarray"`). Free-form: it is diagnostic text minted by the
    /// extension, never a Shape type and never dispatched on.
    pub foreign_type: Arc<str>,
    /// The declared Shape function whose return produced this reference.
    pub produced_by: Arc<str>,
}

impl ForeignRefOrigin {
    /// Render the origin as a diagnostic phrase: `a python module returned by
    /// `load_model``.
    pub fn describe(&self) -> String {
        format!(
            "a {} {} returned by `{}`",
            self.language, self.foreign_type, self.produced_by
        )
    }
}

/// An opaque, refcounted handle on an object owned by a foreign runtime.
///
/// See the module docs for the carrier rules. Construct with [`Self::new`];
/// the value is only ever held behind `Arc`.
#[derive(Debug)]
pub struct ForeignRefData {
    /// The extension's own identifier for the object. Opaque to Shape: it is
    /// minted by the extension, handed back to the same extension at disposal,
    /// and never interpreted, compared across languages, or serialized.
    handle: u64,
    origin: ForeignRefOrigin,
    disposer: Arc<dyn ForeignRefDisposer>,
}

impl ForeignRefData {
    /// Bind a foreign handle to the authority that can release it.
    pub fn new(
        handle: u64,
        origin: ForeignRefOrigin,
        disposer: Arc<dyn ForeignRefDisposer>,
    ) -> Self {
        Self {
            handle,
            origin,
            disposer,
        }
    }

    /// The extension's handle. Only the owning extension may interpret it.
    #[inline]
    pub fn handle(&self) -> u64 {
        self.handle
    }

    /// Where this reference came from — for diagnostics and refusals.
    #[inline]
    pub fn origin(&self) -> &ForeignRefOrigin {
        &self.origin
    }
}

/// Disposal at the last share, per ADR-019 §3.
///
/// This is the whole lifecycle: there is no explicit close, so there is no
/// second path that could dispose a handle twice. `Arc` guarantees this runs
/// once, and the `HeapKind::ForeignRef` retain/release tables are what make the
/// share count correct in the first place.
impl Drop for ForeignRefData {
    fn drop(&mut self) {
        self.disposer.dispose(self.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct RecordingDisposer {
        disposed: Mutex<Vec<u64>>,
    }

    impl ForeignRefDisposer for RecordingDisposer {
        fn dispose(&self, handle: u64) {
            self.disposed.lock().unwrap().push(handle);
        }
    }

    fn origin() -> ForeignRefOrigin {
        ForeignRefOrigin {
            language: "python".into(),
            foreign_type: "module".into(),
            produced_by: "load".into(),
        }
    }

    #[test]
    fn last_share_disposes_exactly_once() {
        let recorder = Arc::new(RecordingDisposer::default());
        let first = Arc::new(ForeignRefData::new(7, origin(), recorder.clone()));
        let second = Arc::clone(&first);

        drop(first);
        assert!(
            recorder.disposed.lock().unwrap().is_empty(),
            "a surviving share must keep the foreign object alive"
        );

        drop(second);
        assert_eq!(
            *recorder.disposed.lock().unwrap(),
            vec![7],
            "the last share disposes, once"
        );
    }

    #[test]
    fn origin_describes_value_and_source() {
        assert_eq!(
            origin().describe(),
            "a python module returned by `load`",
            "refusals quote this verbatim (ADR-019 §3)"
        );
    }

    // ── 4-table lockstep, carrier-crate half ───────────────────────────────
    //
    // ADR-006 §2.7.32 / Q26. `HeapKind::ForeignRef` must retain and release
    // through the SAME `Arc<ForeignRefData>` shape in every dispatch table, or
    // one table over-releases and the foreign object is disposed while Shape
    // still holds a reference. These cover the two tables that live in this
    // crate; `shape-vm` covers the stack track and the TypedObject field
    // track, and `scripts/verify-merge.sh` CHECK 6 proves an arm exists in all
    // four.

    /// Table 2 — `KindedSlot` Clone / Drop (ADR-006 §2.7.6 / Q8).
    #[test]
    fn kinded_slot_clone_and_drop_balance_the_disposal() {
        use crate::KindedSlot;

        let recorder = Arc::new(RecordingDisposer::default());
        let slot = KindedSlot::from_foreign_ref(Arc::new(ForeignRefData::new(
            11,
            origin(),
            recorder.clone(),
        )));

        let duplicate = slot.clone();
        drop(slot);
        assert!(
            recorder.disposed.lock().unwrap().is_empty(),
            "the clone still owns the object"
        );

        drop(duplicate);
        assert_eq!(
            *recorder.disposed.lock().unwrap(),
            vec![11],
            "the last slot disposes exactly once — a double-drop would push twice"
        );
    }

    /// Table 3 — `SharedCell::drop` cell storage (ADR-006 §2.7.8 / Q10). The
    /// shape a `var` binding holding a foreign reference takes once captured.
    #[test]
    fn shared_cell_payload_disposes_when_the_cell_dies() {
        use crate::heap_value::HeapKind;
        use crate::native_kind::NativeKind;
        use crate::v2::closure_layout::SharedCell;

        let recorder = Arc::new(RecordingDisposer::default());
        let bits = Arc::into_raw(Arc::new(ForeignRefData::new(
            12,
            origin(),
            recorder.clone(),
        ))) as u64;
        let cell = Arc::new(SharedCell::new(bits, NativeKind::Ptr(HeapKind::ForeignRef)));

        let second_share = Arc::clone(&cell);
        drop(cell);
        assert!(
            recorder.disposed.lock().unwrap().is_empty(),
            "the cell is still alive, so its payload share is still owned"
        );

        drop(second_share);
        assert_eq!(
            *recorder.disposed.lock().unwrap(),
            vec![12],
            "the cell's last share retires the payload share, which disposes"
        );
    }
}
