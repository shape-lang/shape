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
pub mod value_word;
/// Backward-compatibility alias for the renamed module.
pub mod nanboxed {
    pub use crate::value_word::*;
    /// Backward-compatibility alias: `NanBoxed` is now `ValueWord`.
    pub type NanBoxed = super::value_word::ValueWord;
}
pub mod shape_array;
pub mod shape_graph;
pub mod slot;
pub mod tags;
pub mod typed_buffer;
pub mod unified_array;
pub mod unified_matrix;
pub mod unified_string;
pub mod unified_wrapper;
pub mod v2;
pub mod value;

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
    nb_to_display_string, require_arc_string, require_array, require_bool, require_closure,
    require_datatable, require_f64, require_int, require_number, require_string,
    require_typed_object,
};
pub use heap_header::{FLAG_MARKED, FLAG_PINNED, FLAG_READONLY, HeapHeader};
pub use heap_value::{
    ChannelData, DataReferenceData, DequeData, HashMapData, HeapKind, HeapValue, PriorityQueueData,
    ProjectedRefData, RefProjection, SetData, SimulationCallData,
};
pub use ids::{FunctionId, SchemaId, StackSlotIdx, StringId};
pub use method_id::MethodId;
pub use scalar::{ScalarKind, TypedScalar};
pub use value_word::{ArrayView, ArrayViewMut, NanTag, RefTarget, ValueWord};
/// Backward-compatibility alias: `NanBoxed` is now `ValueWord`.
pub type NanBoxed = ValueWord;
pub use shape_array::ShapeArray;
pub use shape_graph::{
    Shape, ShapeId, ShapeTransitionTable, drain_shape_transitions, hash_property_name,
    shape_for_hashmap_keys, shape_property_index, shape_transition,
};
pub use slot::ValueSlot;
pub use typed_buffer::{AlignedTypedBuffer, TypedBuffer};
pub use value::{
    FilterLiteral, FilterNode, FilterOp, HostCallable, PrintResult, PrintSpan, Upvalue, VMArray,
    VTable, VTableEntry, vmarray_from_nanboxed, vmarray_from_value_words,
};

// v2 runtime re-exports
pub use v2::heap_header::HeapHeader as V2HeapHeader;
pub use v2::refcount::{v2_release, v2_retain};
pub use v2::string_obj::StringObj as V2StringObj;
pub use v2::struct_layout::{FieldInfo, FieldKind, StructLayout};
pub use v2::typed_array::TypedArray as V2TypedArray;
