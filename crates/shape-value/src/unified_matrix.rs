//! Unified matrix representation with `#[repr(C)]` layout.
//!
//! `UnifiedMatrix` is a C-ABI-compatible matrix that can be used by both the
//! VM and JIT without conversion. It wraps an `Arc<MatrixData>` via a leaked
//! raw pointer, giving JIT code direct access to the flat f64 data buffer.
//!
//! ## Memory layout (40 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       2   kind              (HEAP_KIND_MATRIX as u16)
//!   2       1   flags
//!   3       1   _reserved
//!   4       4   refcount          (AtomicU32)
//!   8       8   data              (*const f64, pointer into MatrixData.data)
//!  16       4   rows              (u32)
//!  20       4   cols              (u32)
//!  24       8   total_len         (u64, rows * cols cached)
//!  32       8   owner             (*const MatrixData, leaked Arc)
//! ```

use crate::heap_value::MatrixData;
use crate::tags;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

// ── Offset constants ────────────────────────────────────────────────────────

pub const UM_KIND_OFFSET: i32 = 0;
pub const UM_FLAGS_OFFSET: i32 = 2;
pub const UM_REFCOUNT_OFFSET: i32 = 4;
pub const UM_DATA_OFFSET: i32 = 8;
pub const UM_ROWS_OFFSET: i32 = 16;
pub const UM_COLS_OFFSET: i32 = 20;
pub const UM_TOTAL_LEN_OFFSET: i32 = 24;
pub const UM_OWNER_OFFSET: i32 = 32;

// ── UnifiedMatrix ───────────────────────────────────────────────────────────

/// Unified matrix representation shared between VM and JIT.
///
/// Holds a leaked `Arc<MatrixData>` to keep the data buffer alive, with
/// a direct pointer for zero-copy JIT access.
#[repr(C)]
pub struct UnifiedMatrix {
    /// Heap kind discriminator (HEAP_KIND_MATRIX).
    pub kind: u16,
    /// Bitfield flags.
    pub flags: u8,
    /// Reserved byte for future use.
    pub _reserved: u8,
    /// Reference count for shared ownership.
    pub refcount: AtomicU32,
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
    pub owner: *const MatrixData,
}

// Safety: UnifiedMatrix is a raw data structure with manual memory management.
// Send/Sync are safe because the data is immutable f64 values accessed via
// a raw pointer, and the refcount uses AtomicU32.
unsafe impl Send for UnifiedMatrix {}
unsafe impl Sync for UnifiedMatrix {}

// ── Compile-time layout assertions ──────────────────────────────────────────

const _: () = {
    assert!(std::mem::size_of::<UnifiedMatrix>() == 40);
    assert!(std::mem::offset_of!(UnifiedMatrix, kind) == 0);
    assert!(std::mem::offset_of!(UnifiedMatrix, flags) == 2);
    assert!(std::mem::offset_of!(UnifiedMatrix, _reserved) == 3);
    assert!(std::mem::offset_of!(UnifiedMatrix, refcount) == 4);
    assert!(std::mem::offset_of!(UnifiedMatrix, data) == 8);
    assert!(std::mem::offset_of!(UnifiedMatrix, rows) == 16);
    assert!(std::mem::offset_of!(UnifiedMatrix, cols) == 20);
    assert!(std::mem::offset_of!(UnifiedMatrix, total_len) == 24);
    assert!(std::mem::offset_of!(UnifiedMatrix, owner) == 32);
};

impl UnifiedMatrix {
    /// Create a UnifiedMatrix from an `Arc<MatrixData>`.
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
            kind: tags::HEAP_KIND_MATRIX as u16,
            flags: 0,
            _reserved: 0,
            refcount: AtomicU32::new(1),
            data,
            rows,
            cols,
            total_len,
            owner,
        }
    }

    /// Reconstitute the owned `Arc<MatrixData>` without dropping it.
    ///
    /// Returns a new Arc clone. The UnifiedMatrix retains its own reference
    /// (will be released on drop).
    pub fn to_arc(&self) -> Arc<MatrixData> {
        assert!(!self.owner.is_null(), "UnifiedMatrix has null owner");
        // Safety: owner was created by Arc::into_raw in from_arc.
        let arc = unsafe { Arc::from_raw(self.owner) };
        let cloned = Arc::clone(&arc);
        // Leak back so Drop still has a reference to release.
        std::mem::forget(arc);
        cloned
    }

    // ── NaN-boxing (JIT format, NO bit 47) ──────────────────────────────

    /// Box this matrix into a NaN-boxed TAG_HEAP u64 (JIT format).
    ///
    /// Uses standard `TAG_BASE | ptr` encoding without bit 47.
    #[inline]
    pub fn heap_box(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        tags::TAG_BASE | ((ptr as u64) & tags::PAYLOAD_MASK)
    }

    /// Get a reference from NaN-boxed TAG_HEAP bits (JIT format).
    ///
    /// # Safety
    /// `bits` must be a valid TAG_HEAP value pointing to a live UnifiedMatrix.
    #[inline]
    pub unsafe fn from_heap_bits(bits: u64) -> &'static Self {
        let ptr = (bits & tags::PAYLOAD_MASK) as *const Self;
        unsafe { &*ptr }
    }

    /// Get a mutable reference from NaN-boxed TAG_HEAP bits (JIT format).
    ///
    /// # Safety
    /// `bits` must be a valid TAG_HEAP value pointing to a live UnifiedMatrix.
    /// Caller must ensure exclusive access.
    #[inline]
    pub unsafe fn from_heap_bits_mut(bits: u64) -> &'static mut Self {
        let ptr = (bits & tags::PAYLOAD_MASK) as *mut Self;
        unsafe { &mut *ptr }
    }

    /// Drop a heap-boxed UnifiedMatrix from its NaN-boxed bits (JIT format).
    ///
    /// # Safety
    /// Must only be called once per allocation.
    pub unsafe fn heap_drop(bits: u64) {
        let ptr = (bits & tags::PAYLOAD_MASK) as *mut Self;
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

// ── Drop ────────────────────────────────────────────────────────────────────

impl Drop for UnifiedMatrix {
    fn drop(&mut self) {
        if !self.owner.is_null() {
            // Reconstitute and drop the leaked Arc.
            unsafe {
                let _ = Arc::from_raw(self.owner);
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aligned_vec::AlignedVec;

    fn make_test_matrix(rows: u32, cols: u32) -> Arc<MatrixData> {
        let n = (rows as usize) * (cols as usize);
        let mut data = AlignedVec::with_capacity(n);
        for i in 0..n {
            data.push(i as f64);
        }
        Arc::new(MatrixData::from_flat(data, rows, cols))
    }

    #[test]
    fn test_layout_size() {
        assert_eq!(std::mem::size_of::<UnifiedMatrix>(), 40);
    }

    #[test]
    fn test_layout_offsets() {
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, kind), UM_KIND_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, flags), UM_FLAGS_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, refcount), UM_REFCOUNT_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, data), UM_DATA_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, rows), UM_ROWS_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, cols), UM_COLS_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, total_len), UM_TOTAL_LEN_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedMatrix, owner), UM_OWNER_OFFSET as usize);
    }

    #[test]
    fn test_from_arc_round_trip() {
        let arc = make_test_matrix(3, 4);
        let um = UnifiedMatrix::from_arc(&arc);
        assert_eq!(um.rows, 3);
        assert_eq!(um.cols, 4);
        assert_eq!(um.total_len, 12);
        assert_eq!(um.kind, tags::HEAP_KIND_MATRIX as u16);

        // Data pointer gives direct access.
        let slice = unsafe { std::slice::from_raw_parts(um.data, um.total_len as usize) };
        assert_eq!(slice[0], 0.0);
        assert_eq!(slice[11], 11.0);

        // Round-trip back to Arc.
        let recovered = um.to_arc();
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

        let um = UnifiedMatrix::from_arc(&arc);
        assert_eq!(Arc::strong_count(&arc), 2); // um holds one ref

        let recovered = um.to_arc();
        assert_eq!(Arc::strong_count(&arc), 3); // um + recovered

        drop(recovered);
        assert_eq!(Arc::strong_count(&arc), 2);

        drop(um);
        assert_eq!(Arc::strong_count(&arc), 1); // back to original
    }

    #[test]
    fn test_heap_box_round_trip() {
        let arc = make_test_matrix(2, 3);
        let um = UnifiedMatrix::from_arc(&arc);
        let bits = um.heap_box();

        // Verify it's a TAG_HEAP value
        assert!(tags::is_tagged(bits));
        assert_eq!(tags::get_tag(bits), tags::TAG_HEAP);

        // Verify bit 47 is NOT set (JIT format)
        let payload = tags::get_payload(bits);
        assert_eq!(payload & (1u64 << 47), 0, "bit 47 must not be set");

        // Round-trip
        let recovered = unsafe { UnifiedMatrix::from_heap_bits(bits) };
        assert_eq!(recovered.rows, 2);
        assert_eq!(recovered.cols, 3);
        assert_eq!(recovered.total_len, 6);

        // Clean up
        unsafe { UnifiedMatrix::heap_drop(bits) };
    }

    #[test]
    fn test_refcount_default() {
        let arc = make_test_matrix(1, 1);
        let um = UnifiedMatrix::from_arc(&arc);
        assert_eq!(um.refcount.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
