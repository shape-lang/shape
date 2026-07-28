//! Exhaustive nonzero ownership proof for every SharedCell pointer carrier.

use super::closure::{jit_alloc_shared_cell, jit_arc_shared_release};
use super::shared_cell_payload::{jit_read_shared_cell_ptr, jit_write_shared_cell_ptr};
use super::shared_cell_tests::assert_arc_payload_lifecycle;
use crate::ffi::stack_kind_code;
use arrow_array::RecordBatch;
use arrow_schema::Schema;
use shape_value::content::ContentNode;
use shape_value::datatable::DataTable;
use shape_value::heap_value::{
    AtomicData, ChannelData, DequeData, HashMapData, HashMapKindedRef, HashSetData, HeapValue,
    IoHandleData, LazyData, MatrixData, MatrixSliceData, MutexData, NativeTypeLayout,
    NativeViewData, OptionData, PriorityQueueData, RangeData, ResultData, TableViewData,
    TaskGroupData, TemporalData, TraitObjectStorage, TypedObjectStorage,
};
use shape_value::iterator_state::{IteratorSource, IteratorState};
use shape_value::reference::RefTarget;
use shape_value::v2::closure_layout::{ClosureLayout, SharedCell};
use shape_value::v2::closure_raw::{OwnedClosureBlock, alloc_typed_closure};
use shape_value::v2::decimal_obj::DecimalObj;
use shape_value::v2::heap_element::HeapElement;
use shape_value::v2::heap_header::HeapHeader;
use shape_value::v2::refcount::v2_retain;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{
    ELEM_TYPE_I64, TypedArray, release_v2_typed_array, stamp_elem_type,
};
use shape_value::value::{FilterLiteral, FilterNode, FilterOp, VTable};
use shape_value::{ForeignRefData, ForeignRefDisposer, ForeignRefOrigin};
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnershipClass {
    Inline,
    StdArc,
    ClosureArc,
    V2Header,
    TypedArrayHeader,
    TypedObjectHeader,
    TraitObjectHeader,
    Refused,
}

/// Exercise the same transfer graph for every header-refcounted carrier:
/// cell ownership, retained read, replacement retirement, and final cell drop.
unsafe fn assert_v2_header_lifecycle<T>(
    kind: NativeKind,
    initial: *mut T,
    replacement: *mut T,
    release: unsafe fn(*const T),
) {
    let initial_header = unsafe { &*(initial as *const HeapHeader) };
    let replacement_header = unsafe { &*(replacement as *const HeapHeader) };

    // The constructor's first share remains as the observer. Mint a second
    // share and transfer that one into the cell.
    unsafe {
        v2_retain(initial_header);
        v2_retain(replacement_header);
    }
    assert_eq!(initial_header.get_refcount(), 2);
    assert_eq!(replacement_header.get_refcount(), 2);

    let cell = unsafe { jit_alloc_shared_cell(initial as u64, stack_kind_code::encode(kind)) };
    assert_ne!(cell, 0, "{kind:?}: valid carrier must allocate a cell");

    let read = unsafe { jit_read_shared_cell_ptr(cell as i64) } as *const T;
    assert_eq!(read, initial);
    assert_eq!(initial_header.get_refcount(), 3, "read retains one share");
    unsafe { release(read) };
    assert_eq!(initial_header.get_refcount(), 2);

    unsafe { jit_write_shared_cell_ptr(cell as i64, replacement as i64) };
    assert_eq!(
        initial_header.get_refcount(),
        1,
        "replace retires old share"
    );
    assert_eq!(
        replacement_header.get_refcount(),
        2,
        "cell owns replacement"
    );

    let replacement_read = unsafe { jit_read_shared_cell_ptr(cell as i64) } as *const T;
    assert_eq!(replacement_read, replacement);
    assert_eq!(replacement_header.get_refcount(), 3);
    unsafe { release(replacement_read) };

    unsafe { jit_arc_shared_release(cell) };
    assert_eq!(
        replacement_header.get_refcount(),
        1,
        "cell drop retires payload"
    );
    unsafe {
        release(initial);
        release(replacement);
    }
}

unsafe fn release_typed_array(ptr: *const TypedArray<i64>) {
    unsafe { release_v2_typed_array(ptr as *mut u8) };
}

fn typed_i64_array() -> *mut TypedArray<i64> {
    let ptr = TypedArray::<i64>::new();
    unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_I64) };
    ptr
}

fn empty_table() -> DataTable {
    DataTable::new(RecordBatch::new_empty(Arc::new(Schema::empty())))
}

fn native_view(value: &u64, name: &str) -> NativeViewData {
    NativeViewData {
        ptr: value as *const u64 as usize,
        layout: Arc::new(NativeTypeLayout {
            name: name.to_string(),
            abi: "C".to_string(),
            size: std::mem::size_of::<u64>() as u32,
            align: std::mem::align_of::<u64>() as u32,
            fields: Vec::new(),
        }),
        mutable: false,
    }
}

fn filter(value: i64) -> FilterNode {
    FilterNode::Compare {
        column: "value".to_string(),
        op: FilterOp::Eq,
        value: FilterLiteral::Int(value),
    }
}

/// A disposer that does nothing: this matrix proves the *carrier's* share
/// accounting, and a disposer with side effects would make a lifecycle failure
/// look like a disposal failure. Disposal behaviour has its own fixtures
/// (`shape-vm`'s finalization-observing fake extension).
#[derive(Debug)]
struct InertDisposer;

impl ForeignRefDisposer for InertDisposer {
    fn dispose(&self, _handle: u64) {}
}

fn foreign_ref(handle: u64) -> ForeignRefData {
    ForeignRefData::new(
        handle,
        ForeignRefOrigin {
            language: "python".into(),
            foreign_type: "object".into(),
            produced_by: "fixture".into(),
        },
        Arc::new(InertDisposer),
    )
}

fn closure_value(function_id: u16) -> HeapValue {
    let layout = Arc::new(ClosureLayout::from_capture_types(&[], &[]));
    let ptr = unsafe { alloc_typed_closure(function_id, 0, &layout) };
    let owned = unsafe { OwnedClosureBlock::from_raw(ptr, layout) };
    HeapValue::ClosureRaw(owned)
}

fn typed_object(schema_id: u64) -> *mut TypedObjectStorage {
    TypedObjectStorage::_new(
        schema_id,
        Vec::<ValueSlot>::new().into_boxed_slice(),
        0,
        Arc::from(Vec::<NativeKind>::new().into_boxed_slice()),
    )
}

fn trait_object(type_id: u32) -> *mut TraitObjectStorage {
    TraitObjectStorage::_new(
        typed_object(type_id as u64),
        Arc::new(VTable {
            trait_names: vec![format!("Trait{type_id}")],
            concrete_type_id: type_id,
            methods: HashMap::new(),
        }),
    )
}

fn assert_inline_lifecycle(kind: NativeKind, initial: u64, replacement: u64) {
    let cell = unsafe { jit_alloc_shared_cell(initial, stack_kind_code::encode(kind)) };
    assert_ne!(cell, 0);
    assert_eq!(
        unsafe { jit_read_shared_cell_ptr(cell as i64) } as u64,
        initial
    );
    unsafe { jit_write_shared_cell_ptr(cell as i64, replacement as i64) };
    assert_eq!(
        unsafe { jit_read_shared_cell_ptr(cell as i64) } as u64,
        replacement
    );
    unsafe { jit_arc_shared_release(cell) };
}

macro_rules! arc_case {
    ($kind:expr, $initial:expr, $replacement:expr) => {{
        assert_arc_payload_lifecycle(
            NativeKind::Ptr($kind),
            Arc::new($initial),
            Arc::new($replacement),
        );
        OwnershipClass::StdArc
    }};
}

fn assert_ptr_kind_lifecycle(kind: HeapKind) -> OwnershipClass {
    match kind {
        HeapKind::String => arc_case!(kind, "first".to_string(), "second".to_string()),
        HeapKind::TypedObject => {
            unsafe {
                assert_v2_header_lifecycle(
                    NativeKind::Ptr(kind),
                    typed_object(1),
                    typed_object(2),
                    TypedObjectStorage::release_elem,
                )
            };
            OwnershipClass::TypedObjectHeader
        }
        HeapKind::Closure => {
            assert_arc_payload_lifecycle(
                NativeKind::Ptr(kind),
                Arc::new(closure_value(1)),
                Arc::new(closure_value(2)),
            );
            OwnershipClass::ClosureArc
        }
        HeapKind::Decimal => arc_case!(
            kind,
            rust_decimal::Decimal::ONE,
            rust_decimal::Decimal::new(2, 0)
        ),
        HeapKind::BigInt => arc_case!(kind, 101_i64, 202_i64),
        HeapKind::DataTable => arc_case!(kind, empty_table(), empty_table()),
        HeapKind::Future => {
            assert_inline_lifecycle(NativeKind::Ptr(kind), 41, 42);
            OwnershipClass::Inline
        }
        HeapKind::TaskGroup => arc_case!(
            kind,
            TaskGroupData {
                kind: 1,
                task_ids: vec![1]
            },
            TaskGroupData {
                kind: 2,
                task_ids: vec![2]
            }
        ),
        HeapKind::TypedArray => {
            unsafe {
                assert_v2_header_lifecycle(
                    NativeKind::Ptr(kind),
                    typed_i64_array(),
                    typed_i64_array(),
                    release_typed_array,
                )
            };
            OwnershipClass::TypedArrayHeader
        }
        HeapKind::Temporal => arc_case!(
            kind,
            TemporalData::TimeSpan(chrono::Duration::seconds(1)),
            TemporalData::TimeSpan(chrono::Duration::seconds(2))
        ),
        HeapKind::TableView => arc_case!(
            kind,
            TableViewData::TypedTable {
                schema_id: 1,
                table: Arc::new(empty_table())
            },
            TableViewData::TypedTable {
                schema_id: 2,
                table: Arc::new(empty_table())
            }
        ),
        HeapKind::Content => arc_case!(
            kind,
            ContentNode::Fragment(vec![]),
            ContentNode::Code {
                language: None,
                source: "shape".to_string()
            }
        ),
        HeapKind::Instant => arc_case!(kind, std::time::Instant::now(), std::time::Instant::now()),
        HeapKind::IoHandle => arc_case!(
            kind,
            IoHandleData::new_custom(Box::new(1_u64), "first".to_string()),
            IoHandleData::new_custom(Box::new(2_u64), "second".to_string())
        ),
        HeapKind::NativeScalar => {
            assert_eq!(
                unsafe { jit_alloc_shared_cell(1, stack_kind_code::encode(NativeKind::Ptr(kind))) },
                0
            );
            OwnershipClass::Refused
        }
        HeapKind::NativeView => {
            let first = Box::new(1_u64);
            let second = Box::new(2_u64);
            let class = arc_case!(
                kind,
                native_view(&first, "first"),
                native_view(&second, "second")
            );
            drop((first, second));
            class
        }
        HeapKind::Char => {
            assert_inline_lifecycle(NativeKind::Ptr(kind), 'a' as u64, 'z' as u64);
            OwnershipClass::Inline
        }
        HeapKind::HashMap => arc_case!(
            kind,
            HashMapKindedRef::I64(Arc::new(HashMapData::<i64>::new())),
            HashMapKindedRef::I64(Arc::new(HashMapData::<i64>::new()))
        ),
        HeapKind::FilterExpr => arc_case!(kind, filter(1), filter(2)),
        // ADR-019 §3 / #200: the foreign-reference carrier is an ordinary
        // typed `Arc` payload, so it must prove the same StdArc lifecycle as
        // the other pure-discriminator kinds.
        HeapKind::ForeignRef => arc_case!(kind, foreign_ref(1), foreign_ref(2)),
        HeapKind::Reference => arc_case!(
            kind,
            RefTarget::Local {
                frame_index: 1,
                slot_index: 2,
                kind: NativeKind::Int64
            },
            RefTarget::Local {
                frame_index: 3,
                slot_index: 4,
                kind: NativeKind::Int64
            }
        ),
        HeapKind::SharedCell => arc_case!(
            kind,
            SharedCell::new(1, NativeKind::Int64),
            SharedCell::new(2, NativeKind::Int64)
        ),
        HeapKind::HashSet => arc_case!(kind, HashSetData::new_string(), HashSetData::new_i64()),
        HeapKind::Iterator => arc_case!(
            kind,
            IteratorState::new(IteratorSource::Range {
                start: 0,
                end: 1,
                step: 1
            }),
            IteratorState::new(IteratorSource::Range {
                start: 1,
                end: 2,
                step: 1
            })
        ),
        HeapKind::Deque => arc_case!(kind, DequeData::new(), DequeData::new()),
        HeapKind::Channel => arc_case!(kind, ChannelData::new(), ChannelData::new()),
        HeapKind::PriorityQueue => {
            arc_case!(kind, PriorityQueueData::new(), PriorityQueueData::new())
        }
        HeapKind::Range => arc_case!(kind, RangeData::exclusive(0, 1), RangeData::inclusive(1, 2)),
        HeapKind::Result => arc_case!(
            kind,
            ResultData::ok(KindedSlot::from_int(1)),
            ResultData::err(KindedSlot::from_int(2))
        ),
        HeapKind::Option => arc_case!(
            kind,
            OptionData::some(KindedSlot::from_int(1)),
            OptionData::none()
        ),
        HeapKind::TraitObject => {
            unsafe {
                assert_v2_header_lifecycle(
                    NativeKind::Ptr(kind),
                    trait_object(1),
                    trait_object(2),
                    TraitObjectStorage::release_elem,
                )
            };
            OwnershipClass::TraitObjectHeader
        }
        HeapKind::Mutex => arc_case!(
            kind,
            MutexData::new(KindedSlot::from_int(1)),
            MutexData::new(KindedSlot::from_int(2))
        ),
        HeapKind::Atomic => arc_case!(kind, AtomicData::new(1), AtomicData::new(2)),
        HeapKind::Lazy => arc_case!(kind, LazyData::uninitialized(), LazyData::uninitialized()),
        HeapKind::ModuleFn => {
            assert_inline_lifecycle(NativeKind::Ptr(kind), 7, 9);
            OwnershipClass::Inline
        }
        HeapKind::Matrix => arc_case!(kind, MatrixData::new(1, 1), MatrixData::new(1, 2)),
        HeapKind::MatrixSlice => arc_case!(
            kind,
            MatrixSliceData::new(Arc::new(MatrixData::new(1, 1)), 0, 1),
            MatrixSliceData::new(Arc::new(MatrixData::new(1, 2)), 0, 2)
        ),
    }
}

#[test]
fn every_accepted_shared_carrier_balances_a_nonzero_lifecycle() {
    assert_arc_payload_lifecycle(
        NativeKind::String,
        Arc::new("first".to_string()),
        Arc::new("second".to_string()),
    );
    unsafe {
        assert_v2_header_lifecycle(
            NativeKind::StringV2,
            StringObj::new("first"),
            StringObj::new("second"),
            StringObj::release_elem,
        );
        assert_v2_header_lifecycle(
            NativeKind::DecimalV2,
            DecimalObj::new(rust_decimal::Decimal::ONE),
            DecimalObj::new(rust_decimal::Decimal::new(2, 0)),
            DecimalObj::release_elem,
        );
    }

    let scalar_classes = [
        (NativeKind::String, OwnershipClass::StdArc),
        (NativeKind::StringV2, OwnershipClass::V2Header),
        (NativeKind::DecimalV2, OwnershipClass::V2Header),
    ];
    assert_eq!(scalar_classes.len(), 3);

    let classes: Vec<_> = HeapKind::ALL
        .iter()
        .copied()
        .map(|kind| (kind, assert_ptr_kind_lifecycle(kind)))
        .collect();
    assert_eq!(classes.len(), HeapKind::ALL.len());
    for (class, expected) in [
        (OwnershipClass::Inline, 3),
        // 28 → 29: `HeapKind::ForeignRef` joins the standard-`Arc` class
        // (ADR-019 §3 / #200, 2026-07-28). Its payload is a plain
        // `Arc<ForeignRefData>`, so it must never drift into a HeapHeader
        // class — that mismatch is the documented segfault family.
        (OwnershipClass::StdArc, 29),
        (OwnershipClass::ClosureArc, 1),
        (OwnershipClass::TypedArrayHeader, 1),
        (OwnershipClass::TypedObjectHeader, 1),
        (OwnershipClass::TraitObjectHeader, 1),
        (OwnershipClass::Refused, 1),
    ] {
        assert_eq!(
            classes
                .iter()
                .filter(|(_, actual)| *actual == class)
                .count(),
            expected,
            "{class:?}: ownership-class count drifted"
        );
    }
    assert_eq!(
        scalar_classes.len()
            + classes
                .iter()
                .filter(|(_, class)| *class != OwnershipClass::Refused)
                .count(),
        // 38 → 39 with `HeapKind::ForeignRef` (ADR-019 §3 / #200, 2026-07-28).
        39,
        "three scalar refcounted carriers plus 36 accepted Ptr carriers"
    );
    assert_eq!(
        classes
            .iter()
            .filter(|(_, class)| *class == OwnershipClass::Refused)
            .map(|(kind, _)| *kind)
            .collect::<Vec<_>>(),
        [HeapKind::NativeScalar]
    );
}
