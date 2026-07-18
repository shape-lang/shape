//! ADR-009 C1 (slice 3) — **teardown balance for every DECLARED capture mode.**
//!
//! The declared path changes which `CaptureKind` reaches the emitted
//! `ClosureLayout`, and the `CaptureKind` is exactly what the runtime's
//! ownership discipline dispatches on:
//!
//!   * `Immutable` + `Ptr` → the closure owns ONE heap share, retired by
//!     `release_typed_closure`'s `drop_with_kind` edge walk;
//!   * `Immutable` + scalar → the closure owns NOTHING (`heap_capture_mask`
//!     bit clear — the mask derives from the capture's TYPE, never from the
//!     declared mode);
//!   * `OwnedMutable` → the closure owns a `Box` cell, reclaimed by
//!     `drop_owned_mutable_capture` (which also retires the interior heap share
//!     when the cell's payload is a pointer);
//!   * `Shared` → the closure owns an `Arc<SharedCell>` share, retired by
//!     `drop_shared_capture` (interior payload first, then the cell).
//!
//! A declared mode that flipped the layout mask without the emitter installing
//! the matching share would either LEAK (mask says "no share to retire" but one
//! was installed) or DOUBLE-FREE (mask says "retire a share" but none was
//! installed). Both are silent at the `capture_storage_kind` assertion level —
//! which is why these tests exist.
//!
//! **The layouts under test are the EMITTED ones.** Each test compiles the real
//! declared-capture program through `BytecodeCompiler`, pulls
//! `program.closure_function_layouts[fid]` out of the artifact, and drives the
//! production allocators + `release_typed_closure` against *that*. A model that
//! disagreed with the artifact could not make these pass.
//!
//! Every mode is re-run on a MONOMORPHIZED instantiation (`declared_*_mono`),
//! because the specialization is rebuilt from the template AST and is the path
//! on which a dropped declaration is easiest to miss.

use std::sync::Arc;

use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout, SharedCell};
use shape_value::v2::closure_raw::{
    alloc_owned_mutable_i64, alloc_owned_mutable_ptr, alloc_typed_closure, release_typed_closure,
    write_capture_raw_u64,
};
use shape_value::v2::typed_array::{
    ELEM_TYPE_F64, TypedArray, retain_v2_typed_array, stamp_elem_type,
};
use shape_value::{HeapKind, NativeKind};

use crate::compiler::BytecodeCompiler;

// ── the artifact under test ─────────────────────────────────────────────────

/// Compile a fixture and return the EMITTED layout of the closure whose sole
/// capture is `capture_name`, together with that capture's kind.
fn emitted_layout_for(src: &str, capture_name: &str) -> Arc<ClosureLayout> {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
    compiler
        .compile_in_place(&program)
        .expect("fixture compiles");
    let pack = compiler
        .closure_capture_packs
        .iter()
        .find(|pack| pack.descriptors.len() == 1 && pack.descriptors[0].name == capture_name)
        .unwrap_or_else(|| panic!("no closure captures exactly `{capture_name}`"));
    compiler.program.closure_function_layouts[pack.closure as usize]
        .as_ref()
        .expect("the closure has an emitted layout")
        .clone()
}

/// Downgrade an `Arc::into_raw` pointer to a `Weak` WITHOUT perturbing the
/// strong count, so `strong_count() == 0` can be read after the payload is
/// freed. (Same witness the Phase-4 teardown tests use — `gc_teardown.rs`.)
unsafe fn weak_of<T>(raw: *const T) -> std::sync::Weak<T> {
    unsafe {
        let arc = Arc::from_raw(raw);
        let weak = Arc::downgrade(&arc);
        let _ = Arc::into_raw(arc);
        weak
    }
}

/// A `TypedArray<f64>`'s v2 header refcount, read without perturbing it.
unsafe fn array_rc(arr: *mut TypedArray<f64>) -> u32 {
    unsafe { (*(arr as *const shape_value::V2HeapHeader)).get_refcount() }
}

// ── the fixtures ────────────────────────────────────────────────────────────

/// `move` × a `let string` — Immutable + Ptr.
const MOVE_LET_STRING: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read() -> string { let tag = "hi"
        let worker = |; move tag| tag
        worker() }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read()
"#;

/// `move` × a `let int` — Immutable + SCALAR. The heap mask must stay CLEAR:
/// the mask derives from the TYPE, not the mode.
const MOVE_LET_INT: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read() -> int { let tag = 7
        let worker = |; move tag| tag
        worker() }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read()
"#;

/// `move` × a `let mut Array<number>` — OwnedMutable, interior Ptr.
const MOVE_LET_MUT_ARRAY: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read() -> int { let mut xs = [1.0, 2.0]
        let worker = |; move xs| xs.len()
        worker() }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read()
"#;

/// `share` × a `var` — Shared.
const SHARE_VAR: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int { var total = 5
        let worker = |y: int; share total| y + total
        worker(x) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(2)
"#;

/// The MONOMORPHIZED forms. A generic generated `extend` method reaches
/// emission through `substitute_function_def`; before slice 3 the registry copy
/// the specialization is rebuilt from was UNSTAMPED, so the gate went blind on
/// exactly this shape.
const MOVE_LET_MUT_ARRAY_MONO: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read<T>(f: T) -> int { let mut xs = [1.0, 2.0]
        let worker = |t: T; move xs| xs.len()
        worker(f) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(9)
"#;

const SHARE_VAR_MONO: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read<T>(f: T) -> int { var total = 5
        let worker = |t: T; share total| total
        worker(f) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(9)
"#;

const MOVE_LET_STRING_MONO: &str = r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read<T>(f: T) -> string { let tag = "hi"
        let worker = |t: T; move tag| tag
        worker(f) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(9)
"#;

// ── move × heap `let` ⇒ Immutable + Ptr, EXACTLY ONE share ──────────────────

/// `move` of a `let string`: the emitted layout says `Immutable` and sets the
/// HEAP mask (the type is a pointer). The closure must own EXACTLY ONE share —
/// a double-retain at `op_make_closure` leaks, a missing retain double-frees —
/// and `release_typed_closure` must drive it to zero.
fn assert_move_heap_let_installs_exactly_one_share(src: &str) {
    let layout = emitted_layout_for(src, "tag");
    assert_eq!(layout.capture_count(), 1);
    assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
    assert_eq!(
        layout.heap_capture_mask & 1,
        1,
        "a `string` capture is heap-refcounted whatever the declared mode says"
    );
    assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
    assert_eq!(layout.shared_capture_mask & 1, 0);

    unsafe {
        // The referent: one `Arc<String>` share, exactly as the emitter installs.
        let payload = Arc::new("hi".to_string());
        let raw = Arc::into_raw(payload);
        let weak = weak_of(raw);
        assert_eq!(weak.strong_count(), 1, "the source owns one share");

        // The closure takes ONE share (this models `op_make_closure`'s retain).
        Arc::increment_strong_count(raw);
        assert_eq!(
            weak.strong_count(),
            2,
            "exactly one share installed into the closure — a double-retain here \
             is the `control_flow/mod.rs` op_make_closure leak"
        );

        let block = alloc_typed_closure(0, 0, &layout);
        write_capture_raw_u64(block, &layout, 0, raw as u64);

        // Teardown: the closure retires ITS share, and only its share.
        release_typed_closure(block, &layout);
        assert_eq!(
            weak.strong_count(),
            1,
            "release_typed_closure retired exactly the closure's share"
        );

        // The source's own share retires last; the count reaches ZERO.
        drop(Arc::from_raw(raw));
        assert_eq!(weak.strong_count(), 0, "no leak: the count reaches zero");
    }
}

#[test]
fn declared_move_of_a_heap_let_balances_at_teardown() {
    assert_move_heap_let_installs_exactly_one_share(MOVE_LET_STRING);
}

#[test]
fn declared_move_of_a_heap_let_balances_at_teardown_monomorphized() {
    assert_move_heap_let_installs_exactly_one_share(MOVE_LET_STRING_MONO);
}

// ── move × scalar `let` ⇒ Immutable, heap mask CLEAR ────────────────────────

/// The mask is derived from the TYPE, not the mode. `move` of a `let int` owns
/// nothing, so `release_typed_closure` must free the block and touch no
/// refcount at all — if the declared mode had set the heap bit, this would
/// decrement a refcount on an integer's bit pattern.
#[test]
fn declared_move_of_a_scalar_let_owns_no_heap_share() {
    let layout = emitted_layout_for(MOVE_LET_INT, "tag");
    assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
    assert_eq!(
        layout.heap_capture_mask & 1,
        0,
        "the heap mask follows the capture TYPE — a declared `move` must not set it"
    );
    assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
    assert_eq!(layout.shared_capture_mask & 1, 0);

    unsafe {
        let block = alloc_typed_closure(0, 0, &layout);
        write_capture_raw_u64(block, &layout, 0, 7u64);
        // No share to retire; this must not treat `7` as a pointer.
        release_typed_closure(block, &layout);
    }
}

// ── move × `let mut Array<number>` ⇒ OwnedMutable, interior share retired ───

/// `drop_owned_mutable_capture` must reclaim the `Box` cell AND retire the
/// interior heap share exactly once.
fn assert_owned_mutable_retires_its_interior_once(src: &str) {
    let layout = emitted_layout_for(src, "xs");
    assert_eq!(layout.capture_count(), 1);
    assert_eq!(
        layout.capture_storage_kind(0),
        CaptureKind::OwnedMutable,
        "`move` over a `let mut` is OwnedMutable — the DECLARATION decides, and \
         this body does not even mutate `xs`"
    );
    assert_eq!(layout.owned_mutable_capture_mask & 1, 1);
    assert_eq!(layout.shared_capture_mask & 1, 0);
    assert_eq!(
        layout.heap_capture_mask & 1,
        0,
        "OwnedMutable and heap masks are disjoint — the cell owns the interior, \
         the block does not"
    );

    unsafe {
        // The interior: one `TypedArray<f64>` share, as `AllocOwnedMutable*`
        // installs it (the source slot's share MOVES into the cell).
        let arr = TypedArray::<f64>::with_capacity(2);
        // Producer-side stamp contract (ADR-006 §2.7.7) — the real emitter
        // stamps the element type at allocation; `release_v2_typed_array`
        // refuses to run a typed drop without it.
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
        assert_eq!(array_rc(arr), 1, "the source owns one share");
        // The cell takes the source's share by MOVE — `move` semantics: the
        // outer local is poisoned (`captured_let_mut_moved`), so no retain.
        let cell = alloc_owned_mutable_ptr(arr as u64);

        let block = alloc_typed_closure(0, 0, &layout);
        write_capture_raw_u64(block, &layout, 0, cell as u64);

        // Teardown reclaims the Box cell and retires the interior share once,
        // driving the array's refcount to zero (freeing it). A second retire
        // would be a use-after-free here — under `cargo test` the v2 header's
        // release path aborts on an underflow.
        release_typed_closure(block, &layout);
    }
}

#[test]
fn declared_move_of_a_let_mut_array_balances_at_teardown() {
    assert_owned_mutable_retires_its_interior_once(MOVE_LET_MUT_ARRAY);
}

#[test]
fn declared_move_of_a_let_mut_array_balances_at_teardown_monomorphized() {
    assert_owned_mutable_retires_its_interior_once(MOVE_LET_MUT_ARRAY_MONO);
}

/// A scalar `let mut` under `move` — OwnedMutable with a SCALAR interior. The
/// `Box<i64>` cell is reclaimed and no refcount is touched.
#[test]
fn declared_move_of_a_scalar_let_mut_reclaims_its_box_cell() {
    // The flagship program: `let mut hits = 3`, READ-ONLY in the body.
    let layout = emitted_layout_for(
        r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method scale(f: int) -> int { let mut hits = 3
        let worker = |x: int; move hits| x * hits
        worker(f) }
    }
  }
}
@add_scaler()
type Job { id: int }
let job = Job { id: 1 }
job.scale(2)
"#,
        "hits",
    );
    assert_eq!(layout.capture_storage_kind(0), CaptureKind::OwnedMutable);
    assert_eq!(layout.owned_mutable_capture_mask & 1, 1);
    assert_eq!(layout.heap_capture_mask & 1, 0);

    unsafe {
        let cell = alloc_owned_mutable_i64(3);
        let block = alloc_typed_closure(0, 0, &layout);
        write_capture_raw_u64(block, &layout, 0, cell as u64);
        release_typed_closure(block, &layout);
    }
}

// ── share × `var` ⇒ Shared, interior + cell both retired ────────────────────

/// `drop_shared_capture` must retire the cell's interior heap share AND the
/// `Arc<SharedCell>` share — exactly once each.
fn assert_shared_retires_interior_and_cell(src: &str) {
    let layout = emitted_layout_for(src, "total");
    assert_eq!(layout.capture_count(), 1);
    assert_eq!(layout.capture_storage_kind(0), CaptureKind::Shared);
    assert_eq!(layout.shared_capture_mask & 1, 1);
    assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
    assert_eq!(layout.heap_capture_mask & 1, 0);

    unsafe {
        // A heap-payload cell, so BOTH legs of `drop_shared_capture` run: the
        // interior share and the Arc itself. (`total: int` in the fixture is a
        // scalar; the layout's `capture_inner_kind` is what selects the interior
        // discipline, and a Ptr payload is the harder case, so exercise it with
        // a Ptr-payload cell against a Ptr-typed layout below.)
        let arr = TypedArray::<f64>::with_capacity(1);
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
        assert_eq!(array_rc(arr), 1);
        retain_v2_typed_array(arr as *mut u8); // the cell's payload share
        assert_eq!(array_rc(arr), 2);

        let cell_arc = Arc::new(SharedCell::new(
            arr as u64,
            NativeKind::Ptr(HeapKind::TypedArray),
        ));
        let cell = Arc::into_raw(cell_arc);
        let cell_weak = weak_of(cell);
        assert_eq!(cell_weak.strong_count(), 1, "the outer scope owns the cell");

        // The closure takes ONE share of the cell (`op_make_closure`'s
        // `Arc::increment_strong_count` for a Shared capture).
        Arc::increment_strong_count(cell);
        assert_eq!(cell_weak.strong_count(), 2);

        // Build a Ptr-interior layout with the EMITTED kinds — the kinds are the
        // artifact; only the interior TYPE is swapped so both legs of
        // `drop_shared_capture` are exercised.
        let ptr_layout = Arc::new(ClosureLayout::from_capture_types(
            &[shape_value::v2::concrete_type::ConcreteType::Array(
                Box::new(shape_value::v2::concrete_type::ConcreteType::F64),
            )],
            &[layout.capture_storage_kind(0)],
        ));
        assert_eq!(ptr_layout.shared_capture_mask & 1, 1);

        let block = alloc_typed_closure(0, 0, &ptr_layout);
        write_capture_raw_u64(block, &ptr_layout, 0, cell as u64);

        release_typed_closure(block, &ptr_layout);
        assert_eq!(
            cell_weak.strong_count(),
            1,
            "the closure's cell share is retired exactly once"
        );
        assert_eq!(
            array_rc(arr),
            2,
            "the cell is still alive, so its interior share is untouched"
        );

        // The outer scope's cell share retires last: the cell frees, and its
        // interior share goes with it.
        drop(Arc::from_raw(cell));
        assert_eq!(
            cell_weak.strong_count(),
            0,
            "no leak: the cell is reclaimed"
        );
        assert_eq!(
            array_rc(arr),
            1,
            "the cell retired its interior share exactly once"
        );

        // Reclaim the source's own share.
        shape_value::v2::typed_array::release_v2_typed_array(arr as *mut u8);
    }
}

#[test]
fn declared_share_of_a_var_balances_at_teardown() {
    assert_shared_retires_interior_and_cell(SHARE_VAR);
}

#[test]
fn declared_share_of_a_var_balances_at_teardown_monomorphized() {
    assert_shared_retires_interior_and_cell(SHARE_VAR_MONO);
}

/// Execute production `MakeClosure` installation, capture-cell access, frame
/// teardown, and VM end-of-program teardown for every currently lowerable C1
/// storage discipline. The focused refcount witnesses above isolate exact
/// counts; this is the missing end-to-end proof that the real VM path can
/// install and retire the compiler-emitted artifacts without a manual block.
#[test]
fn declared_capture_modes_run_through_actual_vm_install_and_teardown() {
    use super::test_utils::eval;

    assert_eq!(eval(MOVE_LET_INT).as_i64(), Some(7));
    assert_eq!(eval(MOVE_LET_MUT_ARRAY).as_i64(), Some(2));
    assert_eq!(eval(SHARE_VAR).as_i64(), Some(7));
}

#[path = "declared_capture_teardown/slice4.rs"]
mod slice4;
