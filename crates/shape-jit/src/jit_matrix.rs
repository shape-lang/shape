//! JIT matrix — re-exports `UnifiedMatrix` from shape-value.
//!
//! `JitMatrix` is now a type alias for `shape_value::unified_matrix::UnifiedMatrix`,
//! which has the same data field layout (at offset 8+) but includes a unified
//! heap header prefix (kind, flags, refcount) at offset 0-7.

pub use shape_value::unified_matrix::UnifiedMatrix as JitMatrix;

// Legacy offset constants (relative to data fields, i.e., after the 8-byte header).
pub const MATRIX_DATA_OFFSET: i32 = 0;   // UM_DATA_OFFSET - 8
pub const MATRIX_ROWS_OFFSET: i32 = 8;   // UM_ROWS_OFFSET - 8
pub const MATRIX_COLS_OFFSET: i32 = 12;  // UM_COLS_OFFSET - 8
pub const MATRIX_TOTAL_LEN_OFFSET: i32 = 16; // UM_TOTAL_LEN_OFFSET - 8
pub const MATRIX_OWNER_OFFSET: i32 = 24; // UM_OWNER_OFFSET - 8
