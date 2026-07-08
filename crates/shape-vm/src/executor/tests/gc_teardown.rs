//! GC Phase 4 — end-of-program teardown sweep (Finding #82 /
//! real-gc-cycle-collection.md §5 / Phase 4).
//!
//! `VirtualMachine::drop` releases every VM-owned external root (shared
//! module-binding `Arc<SharedCell>` shares, the live stack window, the kinded
//! module-binding slots) and then runs one final `CollectCycles` sweep so a
//! module-scope self-referential cycle — live for the whole program via its
//! module binding, hence never garbage at a mid-run safepoint — is reclaimed at
//! teardown instead of leaked (identical to a Rust `Rc` cycle at program end).
//! The sweep is **memory-only** per ratification §0 (no user `Drop` runs on
//! cycle members), and must never free a value that is still reachable.
//!
//! These tests build the exact carrier topologies with the production
//! allocators, install them into a real `VirtualMachine`, and drop it — so the
//! Phase-4 teardown path in `executor/mod.rs::Drop` is exercised end to end.
//! Run the module under valgrind (`--error-exitcode=99`): a reclaimed cycle
//! leaks nothing; a prematurely-freed value shows a use-after-free.

#![cfg(feature = "gc")]

use crate::executor::{VMConfig, VirtualMachine};
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout, SharedCell};
use shape_value::v2::closure_raw::{alloc_typed_closure, write_capture_raw_u64, OwnedClosureBlock};
use shape_value::v2::concrete_type::ConcreteType;
use shape_value::v2::typed_array::{
    stamp_elem_type, CallableArrayElem, CallableArrayElemKind, TypedArray, ELEM_TYPE_CALLABLE,
};
use shape_value::{HeapKind, HeapValue, NativeKind, TypedObjectStorage, V2HeapHeader};
use std::sync::{Arc, Mutex, MutexGuard};

/// Serializes the teardown-reclaim tests. The end-of-program sweep runs under
/// the process-global GC stop-the-world coordinator (`collect_under_stop`);
/// two teardown sweeps racing on separate test threads would collide on the
/// coordinator's `collector_lock` (one wins, the other no-ops), leaving the
/// loser's cycle unreclaimed. Holding this lock makes each teardown sweep the
/// sole initiator, matching the real "one VM tears down at program end"
/// single-thread scenario the sweep targets. (Reclamation is best-effort by
/// design — memory-only, may abort under a live cross-thread rendezvous — so
/// the *assertion* of reclamation is what needs the serial guard, not the fix.)
static TEARDOWN_SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    TEARDOWN_SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

// ── Raw-carrier witnesses (read a strong count / downgrade without perturbing
//    it) ────────────────────────────────────────────────────────────────────

/// The four raw carriers of a real Finding-#31 / #82 closure-capture cycle
/// (`var arr = []; arr.push(|| arr.len())`).
struct ClosureCycle {
    /// A — `*mut TypedArray<CallableArrayElem>` (v2 header carrier, CALLABLE).
    arr: *mut TypedArray<CallableArrayElem>,
    /// B — `Arc::into_raw(Arc<HeapValue::ClosureRaw>)` (header-less std-Arc).
    closure_value: *const HeapValue,
    /// D — `Arc::into_raw(Arc<SharedCell>)` (header-less std-Arc).
    cell: *const SharedCell,
}

/// Build the authentic closure-capture cycle with the PRODUCTION allocators:
/// a CALLABLE `TypedArray` (A) whose element owns an `Arc<HeapValue::ClosureRaw>`
/// (B); the closure block (C) has one `Shared` capture owning an
/// `Arc<SharedCell>` (D) whose payload back-edges to A. Steady state: A.rc = 2
/// (external root + cell payload); B/C/D each strong 1.
unsafe fn build_closure_cycle() -> ClosureCycle {
    unsafe {
        // A — CALLABLE TypedArray (external root share = 1).
        let arr = TypedArray::<CallableArrayElem>::with_capacity(1);
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_CALLABLE);

        // D — SharedCell whose payload back-edges to A (one A share moves in).
        shape_value::v2::typed_array::retain_v2_typed_array(arr as *mut u8);
        let cell_arc = Arc::new(SharedCell::new(arr as u64, NativeKind::Ptr(HeapKind::TypedArray)));
        let cell = Arc::into_raw(cell_arc); // D strong = 1

        // C — closure block with one Shared capture holding D.
        let layout_arc = Arc::new(ClosureLayout::from_capture_types(
            &[ConcreteType::Array(Box::new(ConcreteType::I64))],
            &[CaptureKind::Shared],
        ));
        let block = alloc_typed_closure(0, 0, &layout_arc); // C.rc = 1
        write_capture_raw_u64(block, &layout_arc, 0, cell as u64); // C owns D

        // B — Arc<HeapValue::ClosureRaw> owning block C.
        let owned = OwnedClosureBlock::from_raw(block as *const u8, Arc::clone(&layout_arc));
        let hv_arc = Arc::new(HeapValue::ClosureRaw(owned));
        let closure_value = Arc::into_raw(hv_arc); // B strong = 1

        // A's element owns B's share (the array push).
        TypedArray::<CallableArrayElem>::push(
            arr,
            CallableArrayElem {
                bits: closure_value as u64,
                kind: CallableArrayElemKind::Closure,
            },
        );

        ClosureCycle {
            arr,
            closure_value,
            cell,
        }
    }
}

/// Downgrade a raw `Arc::into_raw` pointer to a `Weak<T>` without changing the
/// strong count. After the payload is reclaimed the `Weak` keeps the control
/// block alive, so `weak.strong_count() == 0` can be read with no
/// use-after-free on the freed payload.
unsafe fn weak_of<T>(raw: *const T) -> std::sync::Weak<T> {
    unsafe {
        let arc = Arc::from_raw(raw);
        let w = Arc::downgrade(&arc);
        let _ = Arc::into_raw(arc);
        w
    }
}

/// A fresh VM with no loaded program (module_bindings/stack empty).
fn empty_vm() -> VirtualMachine {
    VirtualMachine::new(VMConfig::default())
}

// ── Test 1: the #82 SharedCell-rooted cycle is reclaimed at teardown ─────────

/// **The Finding #82 repro shape is reclaimed at teardown.** `arr` promotes to a
/// module-scope `SharedCell` (D) whose `TypedArray` payload (A) closes the
/// closure-capture cycle A→B→C→D→A. The module `SharedCell` share drops at
/// teardown but the closure's capture keeps D alive, so the cycle is pinned by
/// its own internal edges — a genuine garbage cycle. The Phase-4 teardown sweep
/// (which buffers the cell's interior payload before the raw `Arc` drop, then
/// runs `CollectCycles`) reclaims all four nodes. Proven via `Weak`: B and D
/// reach strong count 0.
#[test]
fn sharedcell_rooted_module_cycle_reclaimed_at_teardown() {
    let _serial = serial();
    shape_value::gc::clear_candidate_buffer();
    unsafe {
        let c = build_closure_cycle();
        // Drop the array's external root share: now A is held ONLY by the cell
        // payload (A.rc = 1) — `arr` is not directly rooted, exactly as in the
        // promoted-module-var repro.
        shape_value::v2::typed_array::release_v2_typed_array(c.arr as *mut u8);
        assert_eq!((*(c.arr as *const V2HeapHeader)).get_refcount(), 1);

        // Give the cell a SECOND strong share to serve as the module binding
        // (D.strong = 2: the closure capture + the module binding).
        let d_module_share = {
            let a = Arc::from_raw(c.cell);
            let dup = Arc::clone(&a);
            let _ = Arc::into_raw(a);
            Arc::into_raw(dup)
        };

        let weak_b = weak_of(c.closure_value);
        let weak_d = weak_of(c.cell);
        assert_eq!(weak_b.strong_count(), 1);
        assert_eq!(weak_d.strong_count(), 2);

        // A prior mid-run safepoint collection: the cycle is live via the module
        // root, so it is restored (freed 0) and its buffered bits are reset —
        // the realistic state in which only the teardown SharedCell buffering
        // can re-catch the cycle at program end.
        let mid = shape_value::gc::collect_cycles();
        assert_eq!(mid, 0, "live module-rooted cycle is NOT collected mid-run");

        // Install D as a shared module binding (idx 0).
        let mut vm = empty_vm();
        vm.module_bindings.push(d_module_share as u64);
        vm.module_binding_kinds
            .push(NativeKind::Ptr(HeapKind::SharedCell));
        vm.shared_module_bindings.insert(0);

        // Teardown: the SharedCell loop buffers A, drops the module D share, then
        // the final sweep reclaims the whole cycle — memory-only.
        drop(vm);

        assert_eq!(
            weak_b.strong_count(),
            0,
            "B (closure value) reclaimed at teardown"
        );
        assert_eq!(weak_d.strong_count(), 0, "D (SharedCell) reclaimed at teardown");
        assert_eq!(
            shape_value::gc::candidate_buffer_len(),
            0,
            "candidate buffer drained after teardown"
        );

        // A and C are freed by the cascade; do not touch c.arr / c.cell /
        // c.closure_value again. Drop the (strong-0) Weaks so the test leaks
        // nothing.
        drop(weak_b);
        drop(weak_d);
    }
    shape_value::gc::clear_candidate_buffer();
}

// ── Test 2: an array held DIRECTLY by a kinded module binding ────────────────

/// **A cycle whose array is held directly by a kinded module binding is
/// reclaimed at teardown.** Here `A` is the module root (kind
/// `Ptr(TypedArray)`); the module-binding decrement barrier buffers it, and the
/// teardown sweep collects the four nodes. Complements test 1 (no `SharedCell`
/// indirection).
#[test]
fn direct_array_rooted_module_cycle_reclaimed_at_teardown() {
    let _serial = serial();
    shape_value::gc::clear_candidate_buffer();
    unsafe {
        let c = build_closure_cycle();
        // A.rc = 2 (module root share + cell payload). The `arr` pointer IS the
        // module root share.
        assert_eq!((*(c.arr as *const V2HeapHeader)).get_refcount(), 2);

        let weak_b = weak_of(c.closure_value);
        let weak_d = weak_of(c.cell);

        let mut vm = empty_vm();
        vm.module_bindings.push(c.arr as u64);
        vm.module_binding_kinds
            .push(NativeKind::Ptr(HeapKind::TypedArray));

        drop(vm);

        assert_eq!(weak_b.strong_count(), 0, "B reclaimed at teardown");
        assert_eq!(weak_d.strong_count(), 0, "D reclaimed at teardown");
        assert_eq!(shape_value::gc::candidate_buffer_len(), 0);

        drop(weak_b);
        drop(weak_d);
    }
    shape_value::gc::clear_candidate_buffer();
}

// ── Test 3: an acyclic module binding is unaffected ──────────────────────────

/// **An acyclic module binding is released by ordinary RC, unaffected by the
/// sweep.** A plain `Arc<String>` module binding hits refcount 0 on teardown and
/// frees immediately (RC fast path — never buffered); the sweep sees an empty
/// buffer and does nothing. The value is freed exactly once (`Weak` → 0), with
/// no double-free.
#[test]
fn acyclic_module_binding_unaffected_by_teardown_sweep() {
    let _serial = serial();
    shape_value::gc::clear_candidate_buffer();
    let s = Arc::new("acyclic-module-value".to_string());
    let weak_s = Arc::downgrade(&s);
    let raw = Arc::into_raw(s); // the module binding's sole share

    let mut vm = empty_vm();
    vm.module_bindings.push(raw as u64);
    vm.module_binding_kinds.push(NativeKind::String);

    assert_eq!(weak_s.strong_count(), 1);
    drop(vm);

    assert_eq!(
        weak_s.strong_count(),
        0,
        "acyclic String binding freed by ordinary RC at teardown"
    );
    assert_eq!(shape_value::gc::candidate_buffer_len(), 0);
    drop(weak_s);
    shape_value::gc::clear_candidate_buffer();
}

// ── Test 4: a still-reachable cycle is NOT prematurely freed ─────────────────

/// **A cycle that is still externally reachable at teardown is NOT freed.** An
/// extra live share of the array survives the VM drop (held by the test). Trial
/// deletion finds A's refcount residue > 0 → `ScanBlack` restores the whole
/// subgraph → nothing is freed. Proven via `Weak`: B and D stay at strong 1
/// across the teardown. The test then releases its extra share and reclaims the
/// now-garbage cycle so it leaks nothing.
#[test]
fn still_reachable_cycle_not_prematurely_freed_at_teardown() {
    let _serial = serial();
    shape_value::gc::clear_candidate_buffer();
    unsafe {
        let c = build_closure_cycle();
        // Take one EXTRA external share of A that the test keeps past VM drop.
        shape_value::v2::typed_array::retain_v2_typed_array(c.arr as *mut u8);
        // A.rc = 3 (module root + cell payload + our extra external share).
        assert_eq!((*(c.arr as *const V2HeapHeader)).get_refcount(), 3);

        let weak_b = weak_of(c.closure_value);
        let weak_d = weak_of(c.cell);

        let mut vm = empty_vm();
        vm.module_bindings.push(c.arr as u64);
        vm.module_binding_kinds
            .push(NativeKind::Ptr(HeapKind::TypedArray));

        drop(vm);

        // The extra external share kept A reachable → the cycle survived.
        assert_eq!(weak_b.strong_count(), 1, "B not prematurely freed");
        assert_eq!(weak_d.strong_count(), 1, "D not prematurely freed");
        assert_eq!(
            (*(c.arr as *const V2HeapHeader)).get_refcount(),
            2,
            "A survived: module root released, extra share + cell payload remain"
        );

        // Now release the extra share and reclaim the (now-garbage) cycle so the
        // test itself leaks nothing.
        shape_value::v2::typed_array::release_v2_typed_array(c.arr as *mut u8);
        shape_value::gc::gc_buffer_possible_root(c.arr as *mut u8, HeapKind::TypedArray);
        let freed = shape_value::gc::collect_cycles();
        assert_eq!(freed, 4, "cycle reclaimed after the external ref is dropped");
        assert_eq!(weak_b.strong_count(), 0);
        assert_eq!(weak_d.strong_count(), 0);

        drop(weak_b);
        drop(weak_d);
    }
    shape_value::gc::clear_candidate_buffer();
}

// ── Test 5: cycle members SKIP user Drop (memory-only) ───────────────────────

/// A two-field `TypedObjectStorage`: field0 a `Ptr(TypedObject)` heap edge,
/// field1 an owned `Arc<String>` (the observable-`Drop` witness).
unsafe fn mk_obj_with_witness(
    schema: u64,
    edge: *const TypedObjectStorage,
    witness: &Arc<String>,
) -> *mut TypedObjectStorage {
    let kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject), NativeKind::String]);
    TypedObjectStorage::_new(
        schema,
        vec![
            shape_value::slot::ValueSlot::from_typed_object_raw(edge),
            shape_value::slot::ValueSlot::from_string_arc(Arc::clone(witness)),
        ]
        .into_boxed_slice(),
        0b11, // both fields are heap edges
        kinds,
    )
}

/// A one-field `TypedObjectStorage`: field0 a `Ptr(TypedObject)` heap edge.
unsafe fn mk_ref_obj(schema: u64, edge: *const TypedObjectStorage) -> *mut TypedObjectStorage {
    let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject)]);
    TypedObjectStorage::_new(
        schema,
        vec![shape_value::slot::ValueSlot::from_typed_object_raw(edge)].into_boxed_slice(),
        0b1,
        kinds,
    )
}

/// **Cycle members skip user `Drop` at teardown (memory-only reclaim).** A
/// two-node `TypedObject` cycle (A↔B) where A owns an `Arc<String>` witness is
/// installed as a module binding. The teardown sweep reclaims A and B's memory
/// but does NOT run their field `Drop` — so the witness `Arc<String>` share is
/// never released (its strong count is unchanged), exactly as a Rust `Rc` cycle
/// leaks its members' `Drop`. The test then reclaims that leaked witness share.
#[test]
fn cycle_members_skip_user_drop_at_teardown() {
    let _serial = serial();
    shape_value::gc::clear_candidate_buffer();
    unsafe {
        let witness = Arc::new("owned-by-cycle-member".to_string());
        assert_eq!(Arc::strong_count(&witness), 1);

        // A owns the witness (field1) + a heap edge to B (field0). B edges back
        // to A. A is the module root.
        let a = mk_obj_with_witness(301, std::ptr::null(), &witness); // A.rc = 1
        assert_eq!(Arc::strong_count(&witness), 2, "A.field1 owns a witness share");
        // B → A edge: create B with field0 = A, and account the edge's share on
        // A explicitly (mk_ref_obj stores the pointer but does not retain).
        let b = mk_ref_obj(302, a); // B.rc = 1 (its creation share)
        shape_value::v2::refcount::v2_retain(&(*a).header); // A.rc = 2 (module + B.field0)

        // A → B edge: transfer B's sole creation share into the edge (no retain).
        *(*a).slot_cells[0].get() = shape_value::slot::ValueSlot::from_typed_object_raw(b);

        assert_eq!((*a).header.get_refcount(), 2);
        assert_eq!((*b).header.get_refcount(), 1);

        let mut vm = empty_vm();
        vm.module_bindings.push(a as u64);
        vm.module_binding_kinds
            .push(NativeKind::Ptr(HeapKind::TypedObject));

        drop(vm);

        // Memory-only reclaim: A + B freed, but A's owned witness share was NOT
        // released (Drop skipped) — Rust `Rc`-cycle semantics.
        assert_eq!(
            Arc::strong_count(&witness),
            2,
            "cycle member SKIPPED user Drop — owned Arc<String> share not released"
        );
        assert_eq!(shape_value::gc::candidate_buffer_len(), 0);

        // Reclaim the leaked share so the test itself leaks nothing.
        Arc::decrement_strong_count(Arc::as_ptr(&witness));
        assert_eq!(Arc::strong_count(&witness), 1);
    }
    shape_value::gc::clear_candidate_buffer();
}
