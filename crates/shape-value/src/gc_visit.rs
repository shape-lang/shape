//! Read-only heap-child edge visitor (Phase 1 — real-gc-cycle-collection.md
//! §3.4).
//!
//! This module is **entirely gated behind the default-off `gc` Cargo feature**;
//! with the feature off it compiles to nothing (strict no-op).
//!
//! The Bacon–Rajan cycle collector (Phase 3+) traces a heap object's outgoing
//! edges via a **read-only** child visitor. Phase 1 lands that visitor and, most
//! importantly, proves it enumerates **exactly** the edge set the *destructive*
//! `Drop` path releases — the single most dangerous divergence in the whole
//! design (a missed edge = premature-free or leaked cycle; the same multi-table
//! lockstep hazard the codebase historically fears, §14.1).
//!
//! ## One source of truth (lockstep discipline, §3.4)
//!
//! The visitor does **not** re-derive "which slots are heap children." It calls
//! the *same* enumeration primitive the destructive `Drop` walk calls:
//!
//! - **`TypedObject`** → [`TypedObjectStorage::for_each_heap_child_edge`], the
//!   very function `TypedObjectStorage::drop_fields` now enumerates through.
//!   Read-here / release-there over one primitive ⇒ they cannot drift.
//! - **`TypedArray`** → the element-type discriminant stamped in the header
//!   `_pad` byte (offset 7) + `len`, mirroring `release_v2_typed_array`'s
//!   `drop_array_heap` element walk. POD element arrays own no per-element
//!   shares ⇒ zero edges (matching `drop_array`, which frees the buffer without
//!   releasing any element).
//! - **`Closure`** → [`for_each_closure_heap_child`], which mirrors
//!   `release_typed_closure`'s `heap_capture_mask` (immutable Ptr capture)
//!   branch. Closures are not self-describing from the block pointer alone (the
//!   `ClosureLayout` is looked up by `type_id`), so the closure entry takes the
//!   layout explicitly.
//!
//! ## Dispatch discipline (Forbidden Patterns — CLAUDE.md)
//!
//! Dispatch is on `HeapKind` + the object's own parallel-`NativeKind` /
//! `heap_mask` / `_pad` / capture-layout tracks. There is **no** `is_heap()`
//! probe, **no** tag decode, **no** `ValueWord`, **no** parallel discriminator.
//! No child slot is ever inspected to ask "are these bits a pointer?" — the
//! object's own construction-time kind tracks answer that.

use crate::heap_value::{HeapKind, TypedObjectStorage};
use crate::native_kind::NativeKind;
use crate::v2::closure_layout::ClosureLayout;
use crate::v2::typed_array::{
    self, ELEM_TYPE_DECIMAL, ELEM_TYPE_STRING, ELEM_TYPE_TRAIT_OBJECT, ELEM_TYPE_TYPED_ARRAY,
    ELEM_TYPE_TYPED_OBJECT,
};

/// Read-only visit of every heap-child edge of the object at `ptr` of `kind`.
///
/// Dispatches on `HeapKind` and yields `(child_bits, child_kind)` for each
/// outgoing heap edge — the SAME edge set the destructive `Drop` path releases
/// (design §3.4 lockstep). `child_bits` is the child's raw `Arc<T>` / v2-raw
/// carrier pointer; `child_kind` is the child's proven `NativeKind`.
///
/// `Closure` requires its [`ClosureLayout`] (not reachable from the block
/// pointer alone) — use [`for_each_closure_heap_child`] for that kind; a bare
/// `Closure` here yields nothing. Non-cycle-capable / leaf kinds have no
/// outgoing heap edges and yield nothing.
///
/// # Safety
/// `ptr` must point to a live allocation of the given `kind` (offset 0 = its
/// header for header carriers). The yielded child bits are borrowed views — the
/// visitor performs **no** refcount work (it neither retains nor releases).
pub unsafe fn for_each_heap_child<F>(ptr: *const u8, kind: HeapKind, mut f: F)
where
    F: FnMut(u64, NativeKind),
{
    match kind {
        HeapKind::TypedObject => {
            // SAFETY: caller guarantees `ptr` is a live TypedObjectStorage.
            let obj = unsafe { &*(ptr as *const TypedObjectStorage) };
            // The one shared primitive — identical to the destructive Drop walk.
            unsafe { obj.for_each_heap_child_edge(|_i, bits, k| f(bits, k)) };
        }
        HeapKind::TypedArray => {
            // SAFETY: caller guarantees `ptr` is a live TypedArray header.
            unsafe { for_each_typed_array_heap_child(ptr, f) };
        }
        // Closure needs its layout (see `for_each_closure_heap_child`).
        // All other kinds are leaves / non-cycle-capable: no outgoing heap edges.
        _ => {}
    }
}

/// Read-only element walk of a `TypedArray` header at `ptr`, mirroring
/// `release_v2_typed_array` → `drop_array_heap`.
///
/// Only the heap-element discriminants (String / Decimal / TypedObject /
/// TraitObject / nested TypedArray) carry per-element `Arc`/carrier shares that
/// `Drop` releases; POD element arrays (`f64`/`i64`/…) release nothing, so this
/// yields nothing for them — exactly matching the destructive `drop_array`
/// path. The `Callable` element carrier holds non-uniform descriptors (inline
/// ids vs. `Arc<HeapValue>`) rather than a uniform child pointer and is left to
/// a later increment; it is not enumerated here.
///
/// # Safety
/// `ptr` must point to a live `TypedArray<T>` header whose `_pad` discriminant
/// matches its element monomorphization (producer-side stamp contract).
unsafe fn for_each_typed_array_heap_child<F>(ptr: *const u8, mut f: F)
where
    F: FnMut(u64, NativeKind),
{
    // SAFETY: `ptr` is a live TypedArray header per the caller contract.
    let elem_type = unsafe { typed_array::read_elem_type(ptr) };
    let child_kind = match elem_type {
        ELEM_TYPE_STRING => NativeKind::StringV2,
        ELEM_TYPE_DECIMAL => NativeKind::DecimalV2,
        ELEM_TYPE_TYPED_OBJECT => NativeKind::Ptr(HeapKind::TypedObject),
        ELEM_TYPE_TRAIT_OBJECT => NativeKind::Ptr(HeapKind::TraitObject),
        ELEM_TYPE_TYPED_ARRAY => NativeKind::Ptr(HeapKind::TypedArray),
        // POD element arrays (and the deferred Callable carrier, §3.5) release
        // no per-element share on Drop ⇒ no heap-child edges.
        _ => return,
    };
    // Enumerate the element buffer via the SAME shared primitive the
    // destructive `drop_array_heap` release path walks
    // (`typed_array::for_each_typed_array_elem_ptr`) — one enumeration, so the
    // collector's trace and the Drop walk cannot drift on the array's
    // heap-child edge set (real-gc-cycle-collection.md §3.4). The primitive
    // yields every element in `0..len`; skip null here (the collector must not
    // trace a null child), a read-only per-edge filter that leaves the
    // destructive walk it shares byte-identical.
    // SAFETY: `ptr` is a live heap-element TypedArray per the caller contract.
    unsafe {
        typed_array::for_each_typed_array_elem_ptr(ptr, |elem| {
            let bits = elem as u64;
            if bits != 0 {
                f(bits, child_kind);
            }
        });
    }
}

/// Read-only visit of a closure block's heap-child edges, mirroring the
/// `heap_capture_mask` (immutable Ptr capture) branch of
/// `release_typed_closure`.
///
/// Enumerates the immutable heap captures — the ones `release_typed_closure`
/// retires via `drop_with_kind(bits, kind)`. The `OwnedMutable` (Box cell) and
/// `Shared` (`SharedCell`) capture families are header-less interior-mutable
/// carriers routed through the §3.5 side table in a later increment and are not
/// enumerated here.
///
/// # Safety
/// `ptr` must point to a live `TypedClosureHeader` block whose layout is
/// `layout`; the immutable heap-capture slots hold one `Arc<T>` share each for
/// the `T` matching `layout.capture_native_kind(i)`.
pub unsafe fn for_each_closure_heap_child<F>(ptr: *const u8, layout: &ClosureLayout, mut f: F)
where
    F: FnMut(u64, NativeKind),
{
    for i in 0..layout.capture_count() {
        // Enumerate each capture's heap-child edge via the SAME shared accessor
        // the destructive `release_typed_closure` path consumes
        // (`closure_raw::closure_immutable_heap_capture_edge`) — one accessor,
        // so the collector's trace and the Drop walk cannot drift on the
        // closure's heap-child edge set (real-gc-cycle-collection.md §3.4). The
        // `OwnedMutable` / `Shared` interior captures yield `None` here (their
        // GC edges are deferred to the §3.5 side table). Skip null: a read-only
        // per-edge filter that leaves the destructive walk byte-identical.
        // SAFETY: `ptr` is a live closure block whose layout is `layout`.
        if let Some((bits, kind)) =
            unsafe { crate::v2::closure_raw::closure_immutable_heap_capture_edge(ptr, layout, i) }
            && bits != 0
        {
            f(bits, kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slot::ValueSlot;
    use crate::v2::closure_layout::CaptureKind;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    // ── TypedObject parity ──────────────────────────────────────────────────

    /// The read-only visitor yields exactly the direct heap-field edges, in
    /// slot order, and skips scalar + zero slots — same filter as the Drop
    /// enumeration (they share `for_each_heap_child_edge`).
    #[test]
    fn typed_object_visitor_yields_only_heap_fields() {
        let a: Arc<String> = Arc::new("a".to_string());
        let b: Arc<String> = Arc::new("b".to_string());
        let a_bits = Arc::as_ptr(&a) as u64;
        let b_bits = Arc::as_ptr(&b) as u64;

        // Layout: [string a, int, string b, bool] — heap_mask = 0b0101.
        let slots = vec![
            ValueSlot::from_string_arc(a),
            ValueSlot::from_int(99),
            ValueSlot::from_string_arc(b),
            ValueSlot::from_bool(true),
        ]
        .into_boxed_slice();
        let kinds: Arc<[NativeKind]> = Arc::from(vec![
            NativeKind::String,
            NativeKind::Int64,
            NativeKind::String,
            NativeKind::Bool,
        ]);
        let storage = TypedObjectStorage::new(1, slots, 0b0101, kinds);

        let mut edges: Vec<(u64, NativeKind)> = Vec::new();
        // SAFETY: `storage` is live.
        unsafe {
            for_each_heap_child(
                &storage as *const TypedObjectStorage as *const u8,
                HeapKind::TypedObject,
                |bits, kind| edges.push((bits, kind)),
            );
        }

        assert_eq!(
            edges,
            vec![(a_bits, NativeKind::String), (b_bits, NativeKind::String),],
            "visitor must yield exactly the two string fields, in slot order"
        );
        // `storage` drops here; its two shares are released — parity below.
    }

    /// **The parity gate.** The set of children the read-only visitor yields is
    /// exactly the set the destructive `Drop` releases: hold a witness share on
    /// each child, visit to collect the edge set, then drop the parent and
    /// assert every visited child's refcount fell by exactly one (and nothing
    /// else did). Read-set == release-set.
    #[test]
    fn typed_object_visited_edges_equal_dropped_edges() {
        let child0: Arc<String> = Arc::new("child0".to_string());
        let child1: Arc<String> = Arc::new("child1".to_string());
        // Witness clones so we can observe the parent's release.
        let w0 = Arc::clone(&child0);
        let w1 = Arc::clone(&child1);
        assert_eq!(Arc::strong_count(&w0), 2);
        assert_eq!(Arc::strong_count(&w1), 2);

        let slots = vec![
            ValueSlot::from_string_arc(child0),
            ValueSlot::from_int(7),
            ValueSlot::from_string_arc(child1),
        ]
        .into_boxed_slice();
        let kinds: Arc<[NativeKind]> = Arc::from(vec![
            NativeKind::String,
            NativeKind::Int64,
            NativeKind::String,
        ]);
        let storage = TypedObjectStorage::new(2, slots, 0b101, kinds);

        // Collect the visitor's edge set (read-only: refcounts unchanged).
        let mut visited: Vec<u64> = Vec::new();
        unsafe {
            for_each_heap_child(
                &storage as *const TypedObjectStorage as *const u8,
                HeapKind::TypedObject,
                |bits, _kind| visited.push(bits),
            );
        }
        assert_eq!(visited.len(), 2, "two heap children visited");
        assert_eq!(Arc::strong_count(&w0), 2, "visit must not retain");
        assert_eq!(Arc::strong_count(&w1), 2, "visit must not retain");
        assert!(visited.contains(&(Arc::as_ptr(&w0) as u64)));
        assert!(visited.contains(&(Arc::as_ptr(&w1) as u64)));

        // Now the destructive path.
        drop(storage);

        // Exactly the visited children were released — one share each.
        assert_eq!(
            Arc::strong_count(&w0),
            1,
            "dropped edge set must equal visited edge set (child0)"
        );
        assert_eq!(
            Arc::strong_count(&w1),
            1,
            "dropped edge set must equal visited edge set (child1)"
        );
    }

    /// A TypedObject holding a nested v2-raw `TypedObject` field: the visitor
    /// yields that one edge and the drop releases exactly it.
    #[test]
    fn typed_object_nested_object_edge_parity() {
        unsafe {
            let inner_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
            let inner_ptr = TypedObjectStorage::_new(
                10,
                vec![ValueSlot::from_int(7)].into_boxed_slice(),
                0,
                inner_kinds,
            );
            // Witness share so we can observe the outer's release.
            crate::v2::refcount::v2_retain(&(*inner_ptr).header);
            assert_eq!((*inner_ptr).header.refcount.load(Ordering::SeqCst), 2);

            let outer_kinds: Arc<[NativeKind]> =
                Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject)]);
            let outer = TypedObjectStorage::new(
                11,
                vec![ValueSlot::from_typed_object_raw(inner_ptr)].into_boxed_slice(),
                0b1,
                outer_kinds,
            );

            let mut visited: Vec<(u64, NativeKind)> = Vec::new();
            for_each_heap_child(
                &outer as *const TypedObjectStorage as *const u8,
                HeapKind::TypedObject,
                |bits, kind| visited.push((bits, kind)),
            );
            assert_eq!(
                visited,
                vec![(inner_ptr as u64, NativeKind::Ptr(HeapKind::TypedObject))]
            );
            // Visit did not touch the refcount.
            assert_eq!((*inner_ptr).header.refcount.load(Ordering::SeqCst), 2);

            drop(outer);
            // Outer's Drop released exactly the visited edge.
            assert_eq!((*inner_ptr).header.refcount.load(Ordering::SeqCst), 1);

            // Clean up the witness share.
            crate::v2::refcount::v2_release(&(*inner_ptr).header);
        }
    }

    // ── TypedArray parity ───────────────────────────────────────────────────

    /// POD element arrays own no per-element share ⇒ the visitor yields nothing
    /// (matching `drop_array`, which frees the buffer without releasing
    /// elements).
    #[test]
    fn typed_array_pod_yields_no_edges() {
        use crate::v2::typed_array::{ELEM_TYPE_F64, TypedArray, stamp_elem_type};
        unsafe {
            let arr = TypedArray::<f64>::with_capacity(3);
            TypedArray::<f64>::push(arr, 1.0);
            TypedArray::<f64>::push(arr, 2.0);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);

            let mut count = 0usize;
            for_each_heap_child(arr as *const u8, HeapKind::TypedArray, |_b, _k| count += 1);
            assert_eq!(count, 0, "POD array has no heap-child edges");

            typed_array::release_v2_typed_array(arr as *mut u8);
        }
    }

    /// Heap-element (`TypedObject` element) array: the visitor yields exactly
    /// one edge per element, and the destructive release retires exactly those
    /// shares.
    #[test]
    fn typed_array_typed_object_elements_edge_parity() {
        use crate::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};
        unsafe {
            // Two inner v2-raw TypedObjects, each with a witness share.
            let mk_inner = |schema: u64| {
                let k: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
                TypedObjectStorage::_new(
                    schema,
                    vec![ValueSlot::from_int(0)].into_boxed_slice(),
                    0,
                    k,
                )
            };
            let e0 = mk_inner(20);
            let e1 = mk_inner(21);
            crate::v2::refcount::v2_retain(&(*e0).header); // witness
            crate::v2::refcount::v2_retain(&(*e1).header); // witness
            assert_eq!((*e0).header.refcount.load(Ordering::SeqCst), 2);

            // Build a TypedArray<*const TypedObjectStorage> owning one share each.
            let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(2);
            TypedArray::<*const TypedObjectStorage>::push(arr, e0 as *const _);
            TypedArray::<*const TypedObjectStorage>::push(arr, e1 as *const _);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);

            let mut visited: Vec<(u64, NativeKind)> = Vec::new();
            for_each_heap_child(arr as *const u8, HeapKind::TypedArray, |bits, kind| {
                visited.push((bits, kind))
            });
            assert_eq!(
                visited,
                vec![
                    (e0 as u64, NativeKind::Ptr(HeapKind::TypedObject)),
                    (e1 as u64, NativeKind::Ptr(HeapKind::TypedObject)),
                ]
            );
            // Read-only: element shares intact.
            assert_eq!((*e0).header.refcount.load(Ordering::SeqCst), 2);
            assert_eq!((*e1).header.refcount.load(Ordering::SeqCst), 2);

            // Destructive release retires exactly the visited element shares.
            typed_array::release_v2_typed_array(arr as *mut u8);
            assert_eq!((*e0).header.refcount.load(Ordering::SeqCst), 1);
            assert_eq!((*e1).header.refcount.load(Ordering::SeqCst), 1);

            // Clean up witnesses.
            crate::v2::refcount::v2_release(&(*e0).header);
            crate::v2::refcount::v2_release(&(*e1).header);
        }
    }

    /// A heap-element (`StringV2`) array: the read-only visitor's edge set (via
    /// the shared `for_each_typed_array_elem_ptr` primitive) equals the set the
    /// destructive `release_v2_typed_array` retires — one share per element,
    /// exactly the visited pointers. Second carrier proving the array walk and
    /// the Drop walk share one enumeration and cannot drift (§3.4).
    #[test]
    fn typed_array_string_elements_edge_parity() {
        use crate::v2::string_obj::StringObj;
        use crate::v2::typed_array::{ELEM_TYPE_STRING, TypedArray, stamp_elem_type};
        unsafe {
            // Two StringObj carriers, each with a witness share held here.
            let s0 = StringObj::new("child0");
            let s1 = StringObj::new("child1");
            crate::v2::refcount::v2_retain(&(*s0).header); // witness
            crate::v2::refcount::v2_retain(&(*s1).header); // witness
            assert_eq!((*s0).header.refcount.load(Ordering::SeqCst), 2);
            assert_eq!((*s1).header.refcount.load(Ordering::SeqCst), 2);

            // TypedArray<*const StringObj> owning one share each.
            let arr = TypedArray::<*const StringObj>::with_capacity(2);
            TypedArray::<*const StringObj>::push(arr, s0 as *const _);
            TypedArray::<*const StringObj>::push(arr, s1 as *const _);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);

            // Collect the visitor's edge set (read-only: refcounts unchanged).
            let mut visited: Vec<(u64, NativeKind)> = Vec::new();
            for_each_heap_child(arr as *const u8, HeapKind::TypedArray, |bits, kind| {
                visited.push((bits, kind))
            });
            assert_eq!(
                visited,
                vec![
                    (s0 as u64, NativeKind::StringV2),
                    (s1 as u64, NativeKind::StringV2),
                ]
            );
            assert_eq!((*s0).header.refcount.load(Ordering::SeqCst), 2);
            assert_eq!((*s1).header.refcount.load(Ordering::SeqCst), 2);

            // Destructive release retires exactly the visited element shares.
            typed_array::release_v2_typed_array(arr as *mut u8);
            assert_eq!((*s0).header.refcount.load(Ordering::SeqCst), 1);
            assert_eq!((*s1).header.refcount.load(Ordering::SeqCst), 1);

            // Clean up witnesses.
            StringObj::drop(s0 as *mut StringObj);
            StringObj::drop(s1 as *mut StringObj);
        }
    }

    // ── Closure parity ──────────────────────────────────────────────────────

    /// A closure with an immutable heap (String) capture: the read-only closure
    /// visitor yields exactly that one edge and `release_typed_closure` retires
    /// exactly its share. This is the flagship cycle carrier (Finding #31 is a
    /// closure captured into a mutable array).
    #[test]
    fn closure_immutable_heap_capture_edge_parity() {
        use crate::v2::closure_raw::{
            alloc_typed_closure, release_typed_closure, write_capture_typed,
        };
        use crate::v2::concrete_type::ConcreteType;

        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::String], &[CaptureKind::Immutable]);

        let s: Arc<String> = Arc::new("cap".to_string());
        let witness = Arc::clone(&s);
        assert_eq!(Arc::strong_count(&witness), 2);
        // Transfer one strong-count share into the capture slot.
        let bits = Arc::into_raw(s) as u64;

        // SAFETY: alloc + write are paired; the block is released once below.
        unsafe {
            let ptr = alloc_typed_closure(0, 0, &layout);
            write_capture_typed(ptr, &layout, 0, bits);

            let mut visited: Vec<(u64, NativeKind)> = Vec::new();
            for_each_closure_heap_child(ptr as *const u8, &layout, |b, k| visited.push((b, k)));
            assert_eq!(visited, vec![(bits, NativeKind::String)]);
            // Read-only: the captured share is untouched.
            assert_eq!(Arc::strong_count(&witness), 2);

            // Destructive release retires exactly the visited edge, then frees
            // the block.
            release_typed_closure(ptr, &layout);
            assert_eq!(Arc::strong_count(&witness), 1);
        }
    }
}
