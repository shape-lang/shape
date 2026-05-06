//! Core value types for Shape
//!
//! This crate contains the foundational value types and core data structures
//! used throughout the Shape language implementation. After the strict-typing
//! bulldozer, the runtime representation is per-slot raw native bits whose
//! `NativeKind` is determined at compile time and carried in the
//! `FunctionBlob`. There is no `ValueWord`, no NaN-boxing, no dynamic
//! dispatch on value type.
//!
//! Dependency hierarchy:
//! - shape-value → shape-ast (no circular dependencies)
//! - shape-runtime → shape-value (for types)
//! - shape-vm → shape-value (for types)

// Re-export core types
pub mod aligned_vec;
pub mod content;
pub mod context;
pub mod datatable;
pub mod external_value;
pub mod heap_header;
#[macro_use]
pub mod heap_variants;
pub mod heap_value;
pub mod ids;
pub mod method_id;
pub mod scalar;
pub mod string_intern;
pub mod shape_graph;
pub mod shape_graph_current;
pub mod slot;
pub mod typed_buffer;
pub mod v2;
pub mod value;
pub mod vm_closure_handle;

pub use aligned_vec::AlignedVec;
pub use content::{
    BorderStyle, ChartChannel, ChartSeries, ChartSpec, ChartType, Color, ContentNode, ContentTable,
    NamedColor, Style, StyledSpan, StyledText,
};
pub use context::{ErrorLocation, LocatedVMError, VMError};
pub use datatable::{ColumnPtrs, DataTable, DataTableBuilder};
pub use heap_header::{FLAG_MARKED, FLAG_PINNED, FLAG_READONLY, HeapHeader};
pub use heap_value::{HeapKind, HeapValue, TableViewData, TemporalData, TypedArrayData};
pub use ids::{FunctionId, SchemaId, StackSlotIdx, StringId};
pub use method_id::MethodId;
pub use scalar::{ScalarKind, TypedScalar};
pub use shape_graph::{
    Shape, ShapeId, ShapeTransitionTable, drain_shape_transitions, hash_property_name,
    shape_for_hashmap_keys, shape_property_index, shape_transition,
};
pub use shape_graph_current::{
    ShapeTableHandle, SyncShapeTableScope, current_shape_table, try_current_shape_table,
    with_async_shape_table_scope,
};
pub use slot::ValueSlot;
pub use typed_buffer::{AlignedTypedBuffer, TypedBuffer};
pub use value::{FilterLiteral, FilterNode, FilterOp, VTable, VTableEntry};
pub use vm_closure_handle::VmClosureHandle;

// v2 runtime re-exports
pub use v2::heap_header::HeapHeader as V2HeapHeader;
pub use v2::refcount::{v2_release, v2_retain};
pub use v2::string_obj::StringObj as V2StringObj;
pub use v2::struct_layout::{FieldInfo, FieldKind, StructLayout};
pub use v2::typed_array::TypedArray as V2TypedArray;

// v2 runtime types (zero-tag native values)
//
// V2.b: `v2_typed_array` (v1 tag-discriminated `TypedArrayHeader` + `ElemType`)
// was deleted in favour of the monomorphized `v2::typed_array::TypedArray<T>`.
// Use `shape_value::V2TypedArray` (re-exported above) or
// `shape_value::v2::typed_array::TypedArray<T>` directly.
pub mod v2_struct_layout;
