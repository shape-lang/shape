//! Native JIT matrix with guaranteed C-compatible layout.
//!
//! Mirrors `Arc<MatrixData>` for the JIT. Holds the Arc alive via a leaked
//! raw pointer so the flat f64 data buffer remains valid for direct SIMD
//! access from Cranelift-generated code.
//!
//! Memory layout (`#[repr(C)]`):
//! ```text
//!   offset  0: data       — *const f64 (pointer into MatrixData.data, NOT owned)
//!   offset  8: rows       — u32
//!   offset 12: cols       — u32
//!   offset 16: total_len  — u64  (rows * cols, cached for bounds checks)
//!   offset 24: owner      — *const () (leaked Arc<MatrixData>, reconstituted on drop)
//! ```

use std::sync::Arc;

use shape_value::heap_value::MatrixData;

pub const MATRIX_DATA_OFFSET: i32 = 0;
pub const MATRIX_ROWS_OFFSET: i32 = 8;
pub const MATRIX_COLS_OFFSET: i32 = 12;
pub const MATRIX_TOTAL_LEN_OFFSET: i32 = 16;
pub const MATRIX_OWNER_OFFSET: i32 = 24;

/// Native JIT matrix — a flat f64 buffer with row/col dimensions.
///
/// The `data` pointer points directly into the owned `Arc<MatrixData>`'s
/// `AlignedVec<f64>`, giving the JIT zero-copy access to the underlying
/// SIMD-aligned storage.
#[repr(C)]
pub struct JitMatrix {
    /// Pointer to the flat f64 data buffer (row-major order).
    /// NOT owned — lifetime tied to `owner`.
    pub data: *const f64,
    /// Number of rows.
    pub rows: u32,
    /// Number of columns.
    pub cols: u32,
    /// Total element count (rows * cols), cached.
    pub total_len: u64,
    /// Leaked `Arc<MatrixData>` that owns the data buffer.
    /// Reconstituted and dropped in `Drop`.
    owner: *const MatrixData,
}

impl JitMatrix {
    /// Create a JitMatrix from an `Arc<MatrixData>`.
    ///
    /// Leaks one Arc strong reference to keep the data alive. The `Drop`
    /// impl reconstitutes the Arc and releases it.
    pub fn from_arc(arc: &Arc<MatrixData>) -> Self {
        let mat = arc.as_ref();
        let data = mat.data.as_slice().as_ptr();
        let rows = mat.rows;
        let cols = mat.cols;
        let total_len = mat.data.len() as u64;
        // Increment refcount; raw pointer keeps data alive.
        let owner = Arc::into_raw(Arc::clone(arc));
        Self {
            data,
            rows,
            cols,
            total_len,
            owner,
        }
    }

    /// Reconstitute the owned `Arc<MatrixData>` without dropping it.
    ///
    /// Returns a new Arc clone. The JitMatrix retains its own reference
    /// (will be released on drop).
    pub fn to_arc(&self) -> Arc<MatrixData> {
        assert!(!self.owner.is_null(), "JitMatrix has null owner");
        // Safety: owner was created by Arc::into_raw in from_arc.
        let arc = unsafe { Arc::from_raw(self.owner) };
        let cloned = Arc::clone(&arc);
        // Leak back so Drop still has a reference to release.
        std::mem::forget(arc);
        cloned
    }
}

impl Drop for JitMatrix {
    fn drop(&mut self) {
        if !self.owner.is_null() {
            // Reconstitute and drop the leaked Arc.
            unsafe {
                let _ = Arc::from_raw(self.owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::aligned_vec::AlignedVec;

    fn make_test_matrix(rows: u32, cols: u32) -> Arc<MatrixData> {
        let n = (rows as usize) * (cols as usize);
        let mut data = AlignedVec::with_capacity(n);
        for i in 0..n {
            data.push(i as f64);
        }
        Arc::new(MatrixData::from_flat(data, rows, cols))
    }

    #[test]
    fn test_layout() {
        assert_eq!(std::mem::offset_of!(JitMatrix, data), MATRIX_DATA_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(JitMatrix, rows), MATRIX_ROWS_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(JitMatrix, cols), MATRIX_COLS_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(JitMatrix, total_len), MATRIX_TOTAL_LEN_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(JitMatrix, owner), MATRIX_OWNER_OFFSET as usize);
        assert_eq!(std::mem::size_of::<JitMatrix>(), 32);
    }

    #[test]
    fn test_round_trip() {
        let arc = make_test_matrix(3, 4);
        let jm = JitMatrix::from_arc(&arc);
        assert_eq!(jm.rows, 3);
        assert_eq!(jm.cols, 4);
        assert_eq!(jm.total_len, 12);

        // Data pointer gives direct access.
        let slice = unsafe { std::slice::from_raw_parts(jm.data, jm.total_len as usize) };
        assert_eq!(slice[0], 0.0);
        assert_eq!(slice[11], 11.0);

        // Round-trip back to Arc.
        let recovered = jm.to_arc();
        assert_eq!(recovered.rows, 3);
        assert_eq!(recovered.cols, 4);
        assert_eq!(recovered.data[0], 0.0);
        assert_eq!(recovered.data[11], 11.0);

        // Original Arc is still valid.
        assert_eq!(arc.data[5], 5.0);
    }

    #[test]
    fn test_arc_refcount() {
        let arc = make_test_matrix(2, 2);
        assert_eq!(Arc::strong_count(&arc), 1);

        let jm = JitMatrix::from_arc(&arc);
        assert_eq!(Arc::strong_count(&arc), 2); // jm holds one ref

        let recovered = jm.to_arc();
        assert_eq!(Arc::strong_count(&arc), 3); // jm + recovered

        drop(recovered);
        assert_eq!(Arc::strong_count(&arc), 2);

        drop(jm);
        assert_eq!(Arc::strong_count(&arc), 1); // back to original
    }
}
