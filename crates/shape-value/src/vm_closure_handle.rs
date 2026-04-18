//! `VmClosureHandle` — a read-only shim over closure backing storage.
//!
//! # Closure spec §14.2 (H6.1)
//!
//! Closure state is currently stored in `HeapValue::Closure { function_id,
//! upvalues }` (an `Arc<HeapValue>` enum variant). The H6.5 migration will
//! swap the producer side to emit raw `*const TypedClosureHeader` blocks,
//! matching the JIT's Phase H1 memory layout exactly. Between now and
//! H6.6 (where the enum variant is finally deleted), VM and tooling
//! consumers need a stable API that can read closure state regardless of
//! which backing is in use.
//!
//! `VmClosureHandle` is that API. H6.1 introduces it additively — no
//! consumer has migrated yet. H6.2–H6.4 migrate readers, H6.5 swaps the
//! producer, H6.6 deletes the legacy variant.
//!
//! # API surface
//!
//! ```ignore
//! handle.function_id()          // u32 function table index
//! handle.type_id()              // u32 ClosureTypeId (0 in Legacy backing)
//! handle.capture_count()
//! handle.capture_as_value(i)    // widen capture i to ValueWord
//! handle.captures_as_values()   // Vec<ValueWord> of all captures
//! handle.refcount()             // best-effort refcount
//! ```
//!
//! # Backings
//!
//! * `Legacy` — backed by a borrowed `&[Upvalue]` slice and a stored
//!   `function_id`, constructed via [`VmClosureHandle::legacy`]. Used
//!   while `HeapValue::Closure` remains the primary closure variant.
//! * `Raw` — backed by a `*const TypedClosureHeader` + `&ClosureLayout`,
//!   constructed via [`VmClosureHandle::raw`]. Used once H6.5 swaps the
//!   producer and, today, by tests that exercise the raw allocator
//!   directly.

use crate::v2::closure_layout::{ClosureLayout, TypedClosureHeader};
use crate::v2::closure_raw::{
    read_capture_as_value_bits, typed_closure_function_id, typed_closure_refcount,
    typed_closure_type_id,
};
use crate::value::Upvalue;
use crate::value_word::{ValueWord, ValueWordExt};

/// Storage backing for a `VmClosureHandle`.
///
/// Both variants borrow their respective storage — construction is cheap
/// and the handle itself is a thin read-only facade.
enum ClosureBacking<'a> {
    /// Pre-H6.5: closure state lives inside `HeapValue::Closure {
    /// function_id, upvalues }`.
    Legacy {
        function_id: u32,
        upvalues: &'a [Upvalue],
    },
    /// Post-H6.5: closure state lives in a raw `TypedClosureHeader` block
    /// whose capture layout is described by `layout`.
    ///
    /// # Safety invariant
    ///
    /// `ptr` must have been returned by
    /// [`crate::v2::closure_raw::alloc_typed_closure`] paired with the
    /// exact `layout` reference carried here, and must still be live for
    /// the borrow `'a`.
    Raw {
        ptr: *const TypedClosureHeader,
        layout: &'a ClosureLayout,
    },
}

/// Read-only handle to a closure's function id, type id, and captures.
///
/// Construction is a cheap reborrow over the backing storage — no
/// allocation, no refcount traffic.
pub struct VmClosureHandle<'a> {
    backing: ClosureBacking<'a>,
}

impl<'a> VmClosureHandle<'a> {
    /// Construct a handle over the legacy `HeapValue::Closure` backing.
    ///
    /// The legacy backing stores `function_id` as a `u16`; it is widened
    /// to `u32` here so the handle's public API is stable across the
    /// H6.5 swap (the raw backing uses `u32` throughout).
    #[inline]
    pub fn legacy(function_id: u16, upvalues: &'a [Upvalue]) -> Self {
        VmClosureHandle {
            backing: ClosureBacking::Legacy {
                function_id: function_id as u32,
                upvalues,
            },
        }
    }

    /// Construct a handle over a raw `TypedClosureHeader` block.
    ///
    /// # Safety
    ///
    /// * `ptr` must have been allocated by
    ///   [`crate::v2::closure_raw::alloc_typed_closure`] with the same
    ///   `layout` that is passed in here.
    /// * `ptr` must remain live for the duration of the borrow `'a`.
    /// * The block's capture slots must be initialised — each typed
    ///   width from `layout` must contain a value that
    ///   [`read_capture_as_value_bits`] can safely decode.
    #[inline]
    pub unsafe fn raw(ptr: *const TypedClosureHeader, layout: &'a ClosureLayout) -> Self {
        VmClosureHandle {
            backing: ClosureBacking::Raw { ptr, layout },
        }
    }

    /// Function table index for the closure body.
    ///
    /// The legacy backing stores this as `u16`; the raw backing stores
    /// it as `u32`. Both are widened to `u32` here.
    #[inline]
    pub fn function_id(&self) -> u32 {
        match self.backing {
            ClosureBacking::Legacy { function_id, .. } => function_id,
            ClosureBacking::Raw { ptr, .. } => {
                // SAFETY: the Raw constructor requires `ptr` to be a live
                // `TypedClosureHeader`; reading the 4-byte `function_id`
                // at offset 8 is in-bounds.
                unsafe { typed_closure_function_id(ptr as *const u8) as u32 }
            }
        }
    }

    /// `ClosureTypeId` for the closure's capture layout, or `0` for
    /// Legacy-backed handles (which do not carry a per-closure layout id
    /// — the layout is implicit in the `upvalues` slice's length and per-
    /// element dynamic types).
    #[inline]
    pub fn type_id(&self) -> u32 {
        match self.backing {
            ClosureBacking::Legacy { .. } => 0,
            ClosureBacking::Raw { ptr, .. } => {
                // SAFETY: see `function_id` above — same live-block
                // invariant covers the 4-byte read at offset 12.
                unsafe { typed_closure_type_id(ptr as *const u8) }
            }
        }
    }

    /// Number of captures.
    #[inline]
    pub fn capture_count(&self) -> usize {
        match self.backing {
            ClosureBacking::Legacy { upvalues, .. } => upvalues.len(),
            ClosureBacking::Raw { layout, .. } => layout.capture_count(),
        }
    }

    /// Read capture `i` as a `ValueWord`.
    ///
    /// For the Legacy backing, this delegates to `Upvalue::get()` which
    /// auto-deref's through a `HeapValue::SharedCell` wrapper when
    /// present (mutable-shared captures).
    ///
    /// For the Raw backing, the capture's typed native width is read and
    /// widened to a `ValueWord` bit pattern via
    /// [`read_capture_as_value_bits`].
    ///
    /// # Panics
    ///
    /// Panics if `i >= self.capture_count()`.
    #[inline]
    pub fn capture_as_value(&self, i: usize) -> ValueWord {
        match self.backing {
            ClosureBacking::Legacy { upvalues, .. } => upvalues[i].get(),
            ClosureBacking::Raw { ptr, layout } => {
                // SAFETY: the Raw constructor guarantees `ptr` + `layout`
                // match and that the block is live. `i < capture_count`
                // is upheld by the caller (panics on overflow above).
                let bits = unsafe { read_capture_as_value_bits(ptr as *const u8, layout, i) };
                ValueWord::from_raw_bits(bits)
            }
        }
    }

    /// Read every capture as a `ValueWord`, in declaration order.
    #[inline]
    pub fn captures_as_values(&self) -> Vec<ValueWord> {
        let n = self.capture_count();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(self.capture_as_value(i));
        }
        out
    }

    /// Best-effort refcount.
    ///
    /// * Raw backing: returns the `HeapHeader.refcount` — the exact number
    ///   of live shares on the `TypedClosureHeader` block.
    /// * Legacy backing: **always returns `0`**. The meaningful refcount
    ///   for a legacy closure lives on the enclosing `Arc<HeapValue>`,
    ///   which this handle does not hold. Callers that need the Arc-level
    ///   refcount must consult the owning `Arc` directly (via
    ///   `Arc::strong_count`). This caveat goes away after H6.5, when all
    ///   handles are Raw-backed.
    #[inline]
    pub fn refcount(&self) -> u32 {
        match self.backing {
            ClosureBacking::Legacy { .. } => 0,
            ClosureBacking::Raw { ptr, .. } => {
                // SAFETY: live-block invariant from the Raw constructor
                // covers the 4-byte atomic load at offset 0.
                unsafe { typed_closure_refcount(ptr as *const u8) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap_value::HeapValue;
    use crate::v2::closure_layout::ClosureLayout;
    use crate::v2::closure_raw::{
        alloc_typed_closure, release_typed_closure, write_capture_typed,
    };
    use crate::v2::concrete_type::ConcreteType;
    use crate::value_word::{ValueWord, ValueWordExt};

    // ── Legacy backing ─────────────────────────────────────────────────

    fn legacy_closure(function_id: u16, captures: Vec<ValueWord>) -> HeapValue {
        HeapValue::Closure {
            function_id,
            upvalues: captures.into_iter().map(Upvalue::new).collect(),
        }
    }

    #[test]
    fn test_legacy_function_id_read() {
        let hv = legacy_closure(42, vec![]);
        let handle = hv.as_closure_handle().expect("closure handle");
        assert_eq!(handle.function_id(), 42);
        // Legacy backing has no per-closure layout id.
        assert_eq!(handle.type_id(), 0);
    }

    #[test]
    fn test_legacy_capture_count_read() {
        let hv = legacy_closure(
            0,
            vec![
                ValueWord::from_i64(1),
                ValueWord::from_i64(2),
                ValueWord::from_i64(3),
            ],
        );
        let handle = hv.as_closure_handle().expect("closure handle");
        assert_eq!(handle.capture_count(), 3);
    }

    #[test]
    fn test_legacy_capture_as_value_f64() {
        let hv = legacy_closure(0, vec![ValueWord::from_f64(3.14)]);
        let handle = hv.as_closure_handle().expect("closure handle");
        let v = handle.capture_as_value(0);
        assert_eq!(v.as_f64(), Some(3.14));
    }

    #[test]
    fn test_legacy_capture_as_value_i64() {
        let hv = legacy_closure(0, vec![ValueWord::from_i64(-9001)]);
        let handle = hv.as_closure_handle().expect("closure handle");
        let v = handle.capture_as_value(0);
        assert_eq!(v.as_i64(), Some(-9001));
    }

    #[test]
    fn test_legacy_capture_as_value_bool() {
        let hv = legacy_closure(0, vec![ValueWord::from_bool(true)]);
        let handle = hv.as_closure_handle().expect("closure handle");
        let v = handle.capture_as_value(0);
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn test_legacy_captures_as_values_roundtrip() {
        let expected: Vec<ValueWord> = vec![
            ValueWord::from_f64(1.5),
            ValueWord::from_i64(42),
            ValueWord::from_bool(false),
        ];
        let hv = legacy_closure(7, expected.clone());
        let handle = hv.as_closure_handle().expect("closure handle");
        assert_eq!(handle.function_id(), 7);
        let actual = handle.captures_as_values();
        assert_eq!(actual.len(), expected.len());
        assert_eq!(actual[0].as_f64(), Some(1.5));
        assert_eq!(actual[1].as_i64(), Some(42));
        assert_eq!(actual[2].as_bool(), Some(false));
    }

    #[test]
    fn test_legacy_refcount_is_sentinel_zero() {
        // Documented caveat: Legacy backing's `refcount()` returns 0; the
        // meaningful refcount lives on the enclosing `Arc<HeapValue>`.
        let hv = legacy_closure(0, vec![]);
        let handle = hv.as_closure_handle().expect("closure handle");
        assert_eq!(handle.refcount(), 0);
    }

    #[test]
    fn test_value_word_as_closure_handle() {
        // The ValueWord accessor delegates through the HeapValue path.
        let hv = legacy_closure(99, vec![ValueWord::from_i64(7)]);
        let vw = ValueWord::from_heap_value(hv);
        let handle = vw.as_closure_handle().expect("closure handle via ValueWord");
        assert_eq!(handle.function_id(), 99);
        assert_eq!(handle.capture_count(), 1);
        assert_eq!(handle.capture_as_value(0).as_i64(), Some(7));
    }

    #[test]
    fn test_value_word_non_closure_returns_none() {
        let vw = ValueWord::from_i64(5);
        assert!(vw.as_closure_handle().is_none());
    }

    // ── Raw backing ────────────────────────────────────────────────────

    #[test]
    fn test_raw_function_id_read() {
        let layout = ClosureLayout::from_capture_types(&[]);
        // SAFETY: alloc_typed_closure returns a live block; release_typed_closure
        // frees it once at the end of the test.
        unsafe {
            let ptr = alloc_typed_closure(123, 55, &layout);
            let handle =
                VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            assert_eq!(handle.function_id(), 123);
            assert_eq!(handle.type_id(), 55);
            assert_eq!(handle.capture_count(), 0);
            // Fresh block has refcount 1.
            assert_eq!(handle.refcount(), 1);
            drop(handle);
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn test_raw_capture_as_value_typed() {
        // Mixed F64 + I64 + Bool captures, exercising the typed-width
        // read path in read_capture_as_value_bits.
        let layout = ClosureLayout::from_capture_types(&[
            ConcreteType::F64,
            ConcreteType::I64,
            ConcreteType::Bool,
        ]);
        // SAFETY: alloc + writes + reads all go through the well-formed
        // layout; release_typed_closure reclaims the block.
        unsafe {
            let ptr = alloc_typed_closure(1, 2, &layout);
            write_capture_typed(ptr, &layout, 0, ValueWord::from_f64(2.5).into_raw_bits());
            write_capture_typed(ptr, &layout, 1, ValueWord::from_i64(-17).into_raw_bits());
            write_capture_typed(ptr, &layout, 2, ValueWord::from_bool(true).into_raw_bits());

            let handle =
                VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            assert_eq!(handle.function_id(), 1);
            assert_eq!(handle.type_id(), 2);
            assert_eq!(handle.capture_count(), 3);

            let vs = handle.captures_as_values();
            assert_eq!(vs[0].as_f64(), Some(2.5));
            assert_eq!(vs[1].as_i64(), Some(-17));
            assert_eq!(vs[2].as_bool(), Some(true));

            // Individual accessor matches the batched one.
            assert_eq!(handle.capture_as_value(0).as_f64(), Some(2.5));
            assert_eq!(handle.capture_as_value(1).as_i64(), Some(-17));
            assert_eq!(handle.capture_as_value(2).as_bool(), Some(true));

            drop(handle);
            release_typed_closure(ptr, &layout);
        }
    }
}
