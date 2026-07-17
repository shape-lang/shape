use super::jit_finalize_heap_closure;
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout, SharedCell};
use shape_value::v2::closure_raw::{
    OwnedClosureBlock, alloc_owned_mutable_ptr, alloc_typed_closure, release_typed_closure,
    typed_closure_function_id, typed_closure_refcount, typed_closure_type_id, write_capture_typed,
};
use shape_value::v2::concrete_type::ConcreteType;
use shape_value::{HeapKind, HeapValue, KindedSlot, NativeKind, ValueSlot};
use std::sync::Arc;

const CLOSURE_SLOT_KIND: NativeKind = NativeKind::Ptr(HeapKind::Closure);

fn capture_layout(types: &[ConcreteType], kinds: &[CaptureKind]) -> Arc<ClosureLayout> {
    Arc::new(ClosureLayout::from_capture_types(types, kinds))
}

fn immutable_layout(types: &[ConcreteType]) -> Arc<ClosureLayout> {
    capture_layout(types, &vec![CaptureKind::Immutable; types.len()])
}

unsafe fn finalize_owned(
    header: *mut u8,
    argument_function_id: u32,
    capture_count: u32,
    layout: &Arc<ClosureLayout>,
) -> KindedSlot {
    let bits = unsafe {
        jit_finalize_heap_closure(
            header,
            argument_function_id,
            capture_count,
            Arc::as_ptr(layout),
        )
    };
    assert_ne!(
        bits, 0,
        "a valid closure block must finalize to a raw Arc share"
    );
    KindedSlot::new(ValueSlot::from_raw(bits), CLOSURE_SLOT_KIND)
}

fn raw_block(value: &HeapValue) -> &OwnedClosureBlock {
    match value {
        HeapValue::ClosureRaw(block) => block,
        other => panic!("expected ClosureRaw, got {}", other.type_name()),
    }
}

fn owner_block(owner: &KindedSlot) -> &OwnedClosureBlock {
    assert_eq!(owner.kind(), CLOSURE_SLOT_KIND);
    raw_block(owner.slot.as_heap_value())
}

unsafe fn clone_outer_share(owner: &KindedSlot) -> Arc<HeapValue> {
    assert_eq!(owner.kind(), CLOSURE_SLOT_KIND);
    let ptr = owner.raw() as *const HeapValue;
    unsafe {
        Arc::increment_strong_count(ptr);
        Arc::from_raw(ptr)
    }
}

#[test]
fn empty_finalizer_uses_header_authority_and_keeps_its_layout_alive() {
    let layout = immutable_layout(&[]);
    let weak_layout = Arc::downgrade(&layout);
    let header = unsafe { alloc_typed_closure(42, 17, &layout) };
    assert_eq!(unsafe { typed_closure_refcount(header) }, 1);

    // The argument deliberately disagrees: the raw header is authoritative.
    let owner = unsafe { finalize_owned(header, 9_999, 0, &layout) };
    assert_eq!(Arc::strong_count(&layout), 2);
    let block = owner_block(&owner);
    assert_eq!(block.as_ptr(), header as *const u8);
    assert_eq!(unsafe { typed_closure_function_id(block.as_ptr()) }, 42);
    assert_eq!(unsafe { typed_closure_type_id(block.as_ptr()) }, 17);
    assert_eq!(block.layout().capture_count(), 0);
    assert!(Arc::ptr_eq(block.layout(), &layout));

    // `bits` is one raw `Arc<HeapValue::ClosureRaw>` share. This observer is
    // an ordinary second Arc share, balanced below; `owner` keeps its own.
    let outer_observer = unsafe { clone_outer_share(&owner) };
    assert_eq!(Arc::strong_count(&outer_observer), 2);
    assert!(matches!(outer_observer.as_ref(), HeapValue::ClosureRaw(_)));
    drop(outer_observer);

    drop(layout);
    assert!(weak_layout.upgrade().is_some());
    drop(owner);
    assert!(weak_layout.upgrade().is_none());
}

#[test]
fn mixed_scalar_captures_preserve_exact_bits_and_native_kinds() {
    let types = [
        ConcreteType::I64,
        ConcreteType::F64,
        ConcreteType::I32,
        ConcreteType::Bool,
    ];
    let layout = immutable_layout(&types);
    let expected = [
        ((-91i64) as u64, NativeKind::Int64),
        (13.25f64.to_bits(), NativeKind::Float64),
        ((-37i32) as i64 as u64, NativeKind::Int32),
        (1, NativeKind::Bool),
    ];
    let header = unsafe { alloc_typed_closure(7, 3, &layout) };
    for (index, (bits, _)) in expected.iter().copied().enumerate() {
        unsafe { write_capture_typed(header, &layout, index, bits) };
    }

    let owner = unsafe { finalize_owned(header, 7, expected.len() as u32, &layout) };
    let block = owner_block(&owner);
    let actual: Vec<_> = (0..expected.len())
        .map(|index| unsafe { block.read_capture_kinded(index) })
        .collect();
    assert_eq!(actual, expected);
    let expected_kinds = expected.map(|(_, kind)| kind);
    assert_eq!(
        layout.capture_native_kinds.as_slice(),
        expected_kinds.as_slice()
    );

    drop(owner);
    assert_eq!(Arc::strong_count(&layout), 1);
}

#[test]
fn immutable_heap_share_and_all_clone_layers_balance_exactly() {
    let layout = immutable_layout(&[ConcreteType::String]);
    let capture = Arc::new(String::from("finalizer-owned-capture"));
    let capture_bits = Arc::into_raw(Arc::clone(&capture)) as u64;
    assert_eq!(Arc::strong_count(&capture), 2);

    let header = unsafe { alloc_typed_closure(81, 5, &layout) };
    unsafe { write_capture_typed(header, &layout, 0, capture_bits) };

    // The capture slot already owns its share before finalization. This is
    // the finalizer's precondition proof, not a proof of emitter retain code.
    let owner = unsafe { finalize_owned(header, 81, 1, &layout) };
    assert_eq!(Arc::strong_count(&capture), 2);
    assert_eq!(Arc::strong_count(&layout), 2);

    let outer_observer = unsafe { clone_outer_share(&owner) };
    let block = raw_block(outer_observer.as_ref());
    assert_eq!(Arc::strong_count(&outer_observer), 2);
    assert_eq!(unsafe { typed_closure_refcount(block.as_ptr()) }, 1);

    let outer_clone = owner.clone();
    assert_eq!(Arc::strong_count(&outer_observer), 3);
    assert_eq!(unsafe { typed_closure_refcount(block.as_ptr()) }, 1);
    assert_eq!(Arc::strong_count(&layout), 2);
    assert_eq!(Arc::strong_count(&capture), 2);

    let header_clone = (*block).clone();
    assert_eq!(unsafe { typed_closure_refcount(block.as_ptr()) }, 2);
    assert_eq!(Arc::strong_count(&layout), 3);
    assert_eq!(Arc::strong_count(&capture), 2);
    drop(header_clone);
    assert_eq!(unsafe { typed_closure_refcount(block.as_ptr()) }, 1);
    assert_eq!(Arc::strong_count(&layout), 2);

    drop(outer_clone);
    assert_eq!(Arc::strong_count(&outer_observer), 2);
    drop(owner);
    assert_eq!(Arc::strong_count(&outer_observer), 1);
    assert_eq!(unsafe { typed_closure_refcount(block.as_ptr()) }, 1);
    assert_eq!(Arc::strong_count(&capture), 2);

    drop(outer_observer);
    assert_eq!(Arc::strong_count(&layout), 1);
    assert_eq!(Arc::strong_count(&capture), 1);
}

#[test]
fn owned_mutable_ptr_cell_releases_its_payload_share() {
    let layout = capture_layout(&[ConcreteType::String], &[CaptureKind::OwnedMutable]);
    assert_eq!(layout.heap_capture_mask, 0);
    assert_eq!(layout.owned_mutable_capture_mask, 1);
    assert_eq!(layout.shared_capture_mask, 0);

    let payload = Arc::new(String::from("owned-mutable-payload"));
    let weak_payload = Arc::downgrade(&payload);
    let payload_bits = Arc::into_raw(Arc::clone(&payload)) as u64;
    let cell = alloc_owned_mutable_ptr(payload_bits);
    assert_eq!(Arc::strong_count(&payload), 2);

    let header = unsafe { alloc_typed_closure(12, 6, &layout) };
    unsafe { write_capture_typed(header, &layout, 0, cell as u64) };
    let owner = unsafe { finalize_owned(header, 12, 1, &layout) };
    assert_eq!(
        unsafe { owner_block(&owner).read_capture_kinded(0) },
        (cell as u64, NativeKind::String)
    );

    drop(owner);
    assert_eq!(Arc::strong_count(&payload), 1);
    drop(payload);
    assert!(weak_payload.upgrade().is_none());
}

#[test]
fn shared_cell_share_and_payload_release_at_their_exact_boundaries() {
    let layout = capture_layout(&[ConcreteType::String], &[CaptureKind::Shared]);
    assert_eq!(layout.heap_capture_mask, 0);
    assert_eq!(layout.owned_mutable_capture_mask, 0);
    assert_eq!(layout.shared_capture_mask, 1);

    let payload = Arc::new(String::from("shared-cell-payload"));
    let payload_bits = Arc::into_raw(Arc::clone(&payload)) as u64;
    let cell_observer = Arc::new(SharedCell::new(payload_bits, NativeKind::String));
    let weak_cell = Arc::downgrade(&cell_observer);
    let cell_bits = Arc::into_raw(Arc::clone(&cell_observer)) as u64;
    assert_eq!(Arc::strong_count(&cell_observer), 2);
    assert_eq!(Arc::strong_count(&payload), 2);

    let header = unsafe { alloc_typed_closure(13, 7, &layout) };
    unsafe { write_capture_typed(header, &layout, 0, cell_bits) };
    let owner = unsafe { finalize_owned(header, 13, 1, &layout) };
    assert_eq!(
        unsafe { owner_block(&owner).read_capture_kinded(0) },
        (cell_bits, NativeKind::String)
    );

    drop(owner);
    assert_eq!(Arc::strong_count(&cell_observer), 1);
    assert_eq!(Arc::strong_count(&payload), 2);
    drop(cell_observer);
    assert!(weak_cell.upgrade().is_none());
    assert_eq!(Arc::strong_count(&payload), 1);
}

#[test]
fn null_inputs_return_raw_zero_and_leave_cleanup_with_the_caller() {
    let layout = immutable_layout(&[]);
    let null_header_bits =
        unsafe { jit_finalize_heap_closure(std::ptr::null_mut(), 0, 0, Arc::as_ptr(&layout)) };
    assert_eq!(null_header_bits, 0);
    assert_eq!(Arc::strong_count(&layout), 1);

    let header = unsafe { alloc_typed_closure(14, 8, &layout) };
    let null_layout_bits = unsafe { jit_finalize_heap_closure(header, 14, 0, std::ptr::null()) };
    assert_eq!(null_layout_bits, 0);
    assert_eq!(unsafe { typed_closure_refcount(header) }, 1);
    assert_eq!(Arc::strong_count(&layout), 1);

    // A null layout is rejected before ownership transfer. Retire the exact
    // caller-owned block share with its matching production layout helper.
    unsafe { release_typed_closure(header, &layout) };
    assert_eq!(Arc::strong_count(&layout), 1);
}
