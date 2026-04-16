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
pub mod value_word;
/// Backward-compatibility alias for the renamed module.
pub mod nanboxed {
    pub use crate::value_word::*;
}
pub mod shape_array;
pub mod shape_graph;
pub mod slot;
/// Backward-compatibility shim — tags.rs contents moved to value_word.rs.
pub mod tags {
    pub use crate::value_word::{
        // Bit layout constants
        TAG_BASE, PAYLOAD_MASK, TAG_MASK, TAG_SHIFT, CANONICAL_NAN, I48_MAX, I48_MIN,
        // Tag values
        TAG_HEAP, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_FUNCTION, TAG_MODULE_FN, TAG_REF,
        // Inline helpers
        make_tagged, is_tagged, is_number, get_tag, get_payload, sign_extend_i48,
        // Unified heap object discrimination
        UNIFIED_HEAP_FLAG, UNIFIED_PTR_MASK,
        is_unified_heap, unified_heap_ptr, unified_heap_kind, make_unified_heap,
        // HeapKind discriminator constants
        HEAP_KIND_STRING, HEAP_KIND_ARRAY, HEAP_KIND_TYPED_OBJECT, HEAP_KIND_CLOSURE,
        HEAP_KIND_DECIMAL, HEAP_KIND_BIG_INT, HEAP_KIND_HOST_CLOSURE, HEAP_KIND_DATATABLE,
        HEAP_KIND_TYPED_TABLE, HEAP_KIND_ROW_VIEW, HEAP_KIND_COLUMN_REF, HEAP_KIND_INDEXED_TABLE,
        HEAP_KIND_RANGE, HEAP_KIND_ENUM, HEAP_KIND_SOME, HEAP_KIND_OK, HEAP_KIND_ERR,
        HEAP_KIND_FUTURE, HEAP_KIND_TASK_GROUP, HEAP_KIND_TRAIT_OBJECT, HEAP_KIND_EXPR_PROXY,
        HEAP_KIND_FILTER_EXPR, HEAP_KIND_TIME, HEAP_KIND_DURATION, HEAP_KIND_TIMESPAN,
        HEAP_KIND_TIMEFRAME, HEAP_KIND_TIME_REFERENCE, HEAP_KIND_DATETIME_EXPR,
        HEAP_KIND_DATA_DATETIME_REF, HEAP_KIND_TYPE_ANNOTATION, HEAP_KIND_TYPE_ANNOTATED_VALUE,
        HEAP_KIND_PRINT_RESULT, HEAP_KIND_SIMULATION_CALL, HEAP_KIND_FUNCTION_REF,
        HEAP_KIND_DATA_REFERENCE, HEAP_KIND_NUMBER, HEAP_KIND_BOOL, HEAP_KIND_NONE,
        HEAP_KIND_UNIT, HEAP_KIND_FUNCTION, HEAP_KIND_MODULE_FUNCTION, HEAP_KIND_HASHMAP,
        HEAP_KIND_CONTENT, HEAP_KIND_INSTANT, HEAP_KIND_IO_HANDLE, HEAP_KIND_SHARED_CELL,
        HEAP_KIND_NATIVE_SCALAR, HEAP_KIND_NATIVE_VIEW, HEAP_KIND_INT_ARRAY, HEAP_KIND_FLOAT_ARRAY,
        HEAP_KIND_BOOL_ARRAY, HEAP_KIND_MATRIX, HEAP_KIND_ITERATOR, HEAP_KIND_GENERATOR,
        HEAP_KIND_MUTEX, HEAP_KIND_ATOMIC, HEAP_KIND_LAZY, HEAP_KIND_I8_ARRAY, HEAP_KIND_I16_ARRAY,
        HEAP_KIND_I32_ARRAY, HEAP_KIND_U8_ARRAY, HEAP_KIND_U16_ARRAY, HEAP_KIND_U32_ARRAY,
        HEAP_KIND_U64_ARRAY, HEAP_KIND_F32_ARRAY, HEAP_KIND_SET, HEAP_KIND_DEQUE,
        HEAP_KIND_PRIORITY_QUEUE, HEAP_KIND_CHANNEL, HEAP_KIND_CHAR, HEAP_KIND_PROJECTED_REF,
        HEAP_KIND_FLOAT_ARRAY_SLICE,
    };
}
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
pub use scalar::{ScalarKind, TypedScalar, ValueWordScalarExt};
pub use value_word::{ArrayView, ArrayViewMut, RefTarget, ValueWord, ValueWordDisplay, ValueWordExt, nan_tag_type_name, nan_tag_is_truthy};
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

// v2 runtime types (zero-tag native values)
pub mod v2_typed_array;
pub mod v2_struct_layout;
