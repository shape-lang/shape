//! ADR-009 C1 slice-4 nested-Shared runtime and teardown proofs.

use std::sync::Arc;

use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout, SharedCell};
use shape_value::v2::closure_raw::{OwnedClosureBlock, alloc_typed_closure, write_capture_raw_u64};
use shape_value::{HeapKind, NativeKind};

use super::weak_of;

const NESTED_SHARE_VAR: &str = r#"
annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method read() -> int { var total = 40
        let outer = |; share total| { let inner = |; share total| {
          total = total + 2
          total }
          inner()
          total }
        outer() }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read()
"#;

/// Build the smallest VM whose function 0 is a one-capture closure body. The
/// tests below exercise frame setup/teardown directly; no bytecode body needs
/// to run because `unwind_call_frames_to` performs the same local-window
/// release used by the error path and leaves the borrowed closure block alive.
fn one_capture_closure_vm() -> crate::executor::VirtualMachine {
    let mut vm = crate::executor::VirtualMachine::new(crate::VMConfig::default());
    vm.load_program(crate::bytecode::BytecodeProgram {
        functions: vec![crate::bytecode::Function {
            name: "__shared_frame_probe".to_string(),
            arity: 1,
            param_names: vec!["capture".to_string()],
            locals_count: 1,
            entry_point: 0,
            body_length: 0,
            is_closure: true,
            captures_count: 1,
            is_async: false,
            ref_params: vec![false],
            ref_mutates: vec![false],
            mutable_captures: vec![true],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }],
        ..Default::default()
    });
    vm
}

unsafe fn block_owning_shared_cell(
    layout: Arc<ClosureLayout>,
    cell: *const SharedCell,
) -> OwnedClosureBlock {
    unsafe {
        let block = alloc_typed_closure(0, 0, &layout);
        write_capture_raw_u64(block, &layout, 0, cell as u64);
        OwnedClosureBlock::from_raw(block as *const u8, layout)
    }
}

/// Frame setup installs exactly one independent raw-cell share into the Shared
/// synthetic parameter local. The upvalue table is non-owning, so the count is
/// 2 (closure block + local), not 3. Intermediate counts catch both a missing
/// retain and an early/double release.
#[test]
fn shared_scalar_capture_frame_local_has_one_balanced_carrier_share() {
    let layout = Arc::new(ClosureLayout::from_capture_types(
        &[shape_value::v2::concrete_type::ConcreteType::I64],
        &[CaptureKind::Shared],
    ));
    let cell = Arc::into_raw(Arc::new(SharedCell::new(42, NativeKind::Int64)));
    let weak = unsafe { weak_of(cell) };
    let block = unsafe { block_owning_shared_cell(layout, cell) };
    assert_eq!(weak.strong_count(), 1, "the closure block owns the cell");

    let mut vm = one_capture_closure_vm();
    vm.call_closure_with_nb_args(0, &block, &[])
        .expect("Shared closure frame setup");
    assert_eq!(
        weak.strong_count(),
        2,
        "exactly one local carrier share; the raw upvalue table owns none"
    );
    assert_eq!(vm.stack[0], cell as u64);
    assert_eq!(
        vm.kinds[0],
        NativeKind::Ptr(HeapKind::SharedCell),
        "the local kind describes the raw cell carrier, never its Int64 payload"
    );

    vm.unwind_call_frames_to(0);
    assert_eq!(
        weak.strong_count(),
        1,
        "frame teardown retires only the local share, never the block share"
    );
    drop(block);
    assert_eq!(weak.strong_count(), 0, "final block drop reaches zero");
}

/// Heap-payload companion to the scalar proof. Retaining the raw SharedCell
/// carrier must not interpret its bits using the payload's `String` kind.
#[test]
fn shared_heap_capture_frame_local_retains_the_cell_not_its_payload() {
    let layout = Arc::new(ClosureLayout::from_capture_types(
        &[shape_value::v2::concrete_type::ConcreteType::String],
        &[CaptureKind::Shared],
    ));
    let payload = Arc::new("payload".to_string());
    let payload_weak = Arc::downgrade(&payload);
    let payload_bits = Arc::into_raw(payload) as u64;
    let cell = Arc::into_raw(Arc::new(SharedCell::new(payload_bits, NativeKind::String)));
    let cell_weak = unsafe { weak_of(cell) };
    let block = unsafe { block_owning_shared_cell(layout, cell) };

    let mut vm = one_capture_closure_vm();
    vm.call_closure_with_nb_args(0, &block, &[])
        .expect("heap-payload Shared closure frame setup");
    assert_eq!(cell_weak.strong_count(), 2, "block + one local cell share");
    assert_eq!(
        payload_weak.strong_count(),
        1,
        "frame setup must not retain the payload in place of the cell"
    );

    vm.unwind_call_frames_to(0);
    assert_eq!(cell_weak.strong_count(), 1);
    assert_eq!(
        payload_weak.strong_count(),
        1,
        "the live block keeps the cell and payload alive after frame teardown"
    );
    drop(block);
    assert_eq!(
        cell_weak.strong_count(),
        0,
        "cell reaches zero exactly once"
    );
    assert_eq!(
        payload_weak.strong_count(),
        0,
        "cell teardown retires its sole heap-payload share exactly once"
    );
}

/// Full production compiler + VM proof: the inner mutation must be observed
/// through the outer capture. A fresh cell or payload snapshot returns the
/// wrong value even if its refcounts happen to balance.
#[test]
fn nested_declared_share_runs_through_vm_install_and_teardown() {
    let value = super::super::test_utils::eval(NESTED_SHARE_VAR);
    assert_eq!(
        value.as_i64(),
        Some(42),
        "nested `share` must mutate the same inherited SharedCell"
    );
}
