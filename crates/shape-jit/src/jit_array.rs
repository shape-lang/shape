//! JIT array — re-exports `UnifiedArray` from shape-value.
//!
//! `JitArray` is now a type alias for `shape_value::unified_array::UnifiedArray`,
//! which has the same data field layout (at offset 8+) but includes a unified
//! heap header prefix (kind, flags, refcount) at offset 0-7.
//!
//! The inline Cranelift offsets (DATA_OFFSET=0, LEN_OFFSET=8, etc.) remain
//! correct because they are relative to (ptr + 8), and `emit_array_ptr` still
//! adds the 8-byte prefix offset.

pub use shape_value::unified_array::ArrayElementKind;
pub use shape_value::unified_array::UnifiedArray as JitArray;

// Legacy offset constants (relative to data fields, i.e., after the 8-byte header).
// These are used by Cranelift IR emission after `emit_array_ptr` adds 8.
pub const DATA_OFFSET: i32 = 0;   // UA_DATA_OFFSET - 8
pub const LEN_OFFSET: i32 = 8;    // UA_LEN_OFFSET - 8
pub const CAP_OFFSET: i32 = 16;   // UA_CAP_OFFSET - 8
pub const TYPED_DATA_OFFSET: i32 = 24;  // UA_TYPED_DATA_OFFSET - 8
pub const ELEMENT_KIND_OFFSET: i32 = 32; // UA_ELEMENT_KIND_OFFSET - 8
