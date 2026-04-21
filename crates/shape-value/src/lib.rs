//! Core value types for Shape
//!
//! This crate contains the foundational value types and core data structures
//! used throughout the Shape language implementation. The canonical runtime
//! representation is `ValueWord` — an 8-byte NaN-boxed value.
//!
//! Dependency hierarchy:
//! - shape-value → shape-ast (no circular dependencies)
//! - shape-runtime → shape-value (for types)
//! - shape-vm → shape-value (for types)

// Re-export core types
pub mod aligned_vec;
pub mod array_view;
pub mod closure;
pub mod content;
pub mod context;
pub mod datatable;
pub mod enums;
pub mod external_value;
pub mod extraction;
pub mod heap_header;
#[macro_use]
pub mod heap_variants;
pub mod heap_value;
pub mod ids;
pub mod method_id;
pub mod scalar;
pub mod string_intern;
pub mod tag_bits;
pub mod value_bits;
pub mod value_word;
pub mod value_word_ext;
/// Backward-compatibility alias for the renamed module.
pub mod nanboxed {
    pub use crate::value_word::*;
}
pub mod shape_array;
pub mod shape_graph;
pub mod slot;
/// Backward-compatibility shim — tag constants / helpers live in `tag_bits`.
pub mod tags {
    pub use crate::tag_bits::*;
}
pub mod typed_buffer;
pub mod unified_array;
pub mod unified_matrix;
pub mod unified_string;
pub mod unified_wrapper;
pub mod v2;
pub mod value;
pub mod vm_closure_handle;

pub use aligned_vec::AlignedVec;
pub use closure::Closure;
pub use content::{
    BorderStyle, ChartChannel, ChartSeries, ChartSpec, ChartType, Color, ContentNode, ContentTable,
    NamedColor, Style, StyledSpan, StyledText,
};
pub use context::{ErrorLocation, LocatedVMError, VMContext, VMError};
pub use datatable::{ColumnPtrs, DataTable, DataTableBuilder};
pub use enums::{EnumPayload, EnumValue};
pub use external_value::{
    ExternalValue, NoSchemaLookup, SchemaLookup, external_to_nb, nb_to_external,
};
pub use extraction::{
    nb_to_display_string, require_arc_string, require_array, require_bool, require_datatable,
    require_f64, require_int, require_number, require_string, require_typed_object,
};
pub use heap_header::{FLAG_MARKED, FLAG_PINNED, FLAG_READONLY, HeapHeader};
pub use heap_value::{
    ChannelData, ConcurrencyData, DataReferenceData, DequeData, HashMapData, HeapKind, HeapValue,
    PriorityQueueData, ProjectedRefData, RareHeapData, RefProjection, SetData, SimulationCallData,
    TableViewData, TemporalData, TypedArrayData,
};
pub use ids::{FunctionId, SchemaId, StackSlotIdx, StringId};
pub use method_id::MethodId;
pub use scalar::{ScalarKind, TypedScalar, ValueWordScalarExt};
pub use value_word::{ArrayView, ArrayViewMut, RefTarget, ValueBits, ValueWord, ValueWordDisplay};
pub use value_word_ext::ValueWordExt;
pub use shape_array::ShapeArray;
pub use shape_graph::{
    Shape, ShapeId, ShapeTransitionTable, drain_shape_transitions, hash_property_name,
    shape_for_hashmap_keys, shape_property_index, shape_transition,
};
pub use slot::ValueSlot;
pub use typed_buffer::{AlignedTypedBuffer, TypedBuffer};
pub use value::{
    FilterLiteral, FilterNode, FilterOp, HostCallable, PrintResult, PrintSpan, Upvalue, VMArray,
    VMArrayBuf, VMARRAY_INLINE_CAP, VTable, VTableEntry, vmarray_from_nanboxed,
    vmarray_from_value_words, vmarray_from_vec,
};
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
