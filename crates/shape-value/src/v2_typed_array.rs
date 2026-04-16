//! v2 runtime: `TypedArray<T>` — contiguous native-element arrays.
//!
//! This is the v2 replacement for the NaN-boxed `HeapValue::Array`. Each array
//! stores elements as raw `T` values in a separate allocation pointed to by
//! `data`, with the header carrying refcount, element type tag, length, and
//! capacity.
//!
//! ## Memory layout (`TypedArrayHeader`, 24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       4   refcount  (AtomicU32)
//!   4       2   kind      (HeapKind discriminant, u16)
//!   6       1   elem_type (ElemType discriminant, u8)
//!   7       1   _pad
//!   8       8   data      (*mut u8, raw pointer to element buffer)
//!  16       4   len       (current element count)
//!  20       4   cap       (allocated capacity in elements)
//! ```
//!
//! The element buffer at `*data` is a contiguous `[T; cap]` allocation whose
//! element size is determined by `elem_type`.

use std::alloc::{self, Layout};
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// ElemType
// ---------------------------------------------------------------------------

/// Element type tag stored in `TypedArrayHeader::elem_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ElemType {
    F64 = 1,
    I64 = 2,
    I32 = 3,
    U32 = 4,
    Bool = 5,
    Ptr = 6,
}

impl ElemType {
    /// Size in bytes of a single element of this type.
    #[inline]
    pub const fn size(self) -> usize {
        match self {
            ElemType::F64 => 8,
            ElemType::I64 => 8,
            ElemType::I32 => 4,
            ElemType::U32 => 4,
            ElemType::Bool => 1,
            ElemType::Ptr => 8,
        }
    }

    /// Required alignment for elements of this type.
    #[inline]
    pub const fn align(self) -> usize {
        match self {
            ElemType::F64 => 8,
            ElemType::I64 => 8,
            ElemType::I32 => 4,
            ElemType::U32 => 4,
            ElemType::Bool => 1,
            ElemType::Ptr => 8,
        }
    }

    /// Try to convert a raw `u8` discriminant into an `ElemType`.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(ElemType::F64),
            2 => Some(ElemType::I64),
            3 => Some(ElemType::I32),
            4 => Some(ElemType::U32),
            5 => Some(ElemType::Bool),
            6 => Some(ElemType::Ptr),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// TypedArrayHeader
// ---------------------------------------------------------------------------

/// The fixed-size, `repr(C)` header that prefixes every v2 typed array.
///
/// The element buffer is a separate heap allocation; `data` points to its start.
/// This keeps the header a fixed 24 bytes regardless of element type.
#[repr(C)]
pub struct TypedArrayHeader {
    /// Reference count (offset 0, 4 bytes).
    pub refcount: AtomicU32,
    /// Heap object kind discriminant (offset 4, 2 bytes).
    pub kind: u16,
    /// Element type tag (offset 6, 1 byte). See [`ElemType`].
    pub elem_type: u8,
    /// Padding byte (offset 7).
    pub _pad: u8,
    /// Pointer to the contiguous element buffer (offset 8, 8 bytes).
    pub data: *mut u8,
    /// Current number of live elements (offset 16, 4 bytes).
    pub len: u32,
    /// Capacity (in elements) of the allocation at `data` (offset 20, 4 bytes).
    pub cap: u32,
}

// Safety: the header is managed through raw pointers with explicit
// synchronization on refcount. We need Send/Sync so that the header pointer
// can be passed across threads (the actual thread-safety contract is enforced
// at a higher level by the runtime).
unsafe impl Send for TypedArrayHeader {}
unsafe impl Sync for TypedArrayHeader {}

/// Compile-time assertions on size and field offsets.
const _: () = {
    assert!(std::mem::size_of::<TypedArrayHeader>() == 24);
    assert!(std::mem::offset_of!(TypedArrayHeader, refcount) == 0);
    assert!(std::mem::offset_of!(TypedArrayHeader, kind) == 4);
    assert!(std::mem::offset_of!(TypedArrayHeader, elem_type) == 6);
    assert!(std::mem::offset_of!(TypedArrayHeader, _pad) == 7);
    assert!(std::mem::offset_of!(TypedArrayHeader, data) == 8);
    assert!(std::mem::offset_of!(TypedArrayHeader, len) == 16);
    assert!(std::mem::offset_of!(TypedArrayHeader, cap) == 20);
};

/// HeapKind discriminant value used for v2 typed arrays.
/// This is a placeholder — it will be unified with `HeapKind` once the v2
/// runtime is wired end-to-end.
pub const HEAP_KIND_TYPED_ARRAY: u16 = 0xFF;

// ---------------------------------------------------------------------------
// Allocation helpers
// ---------------------------------------------------------------------------

/// Compute the `Layout` for an element buffer of `cap` elements of the given
/// `ElemType`. Returns `None` if `cap` is zero (no allocation needed).
fn data_layout(et: ElemType, cap: u32) -> Option<Layout> {
    if cap == 0 {
        return None;
    }
    let size = et.size().checked_mul(cap as usize)?;
    Layout::from_size_align(size, et.align()).ok()
}

/// Allocate a new `TypedArrayHeader` with room for `cap` elements.
///
/// The returned pointer is heap-allocated (via `alloc::alloc`) and must be
/// freed with [`typed_array_free`]. The refcount is initialised to 1, length
/// to 0, and the element buffer is zeroed.
///
/// # Panics
///
/// Panics if the header or data allocation fails (OOM).
pub fn typed_array_alloc(elem_type: u8, cap: u32) -> *mut TypedArrayHeader {
    let et = ElemType::from_u8(elem_type).expect("invalid elem_type");

    // Allocate the header.
    let header_layout = Layout::new::<TypedArrayHeader>();
    let header_ptr = unsafe { alloc::alloc_zeroed(header_layout) } as *mut TypedArrayHeader;
    assert!(!header_ptr.is_null(), "TypedArrayHeader allocation failed");

    // Allocate the element buffer (may be null if cap == 0).
    let data_ptr = if let Some(layout) = data_layout(et, cap) {
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "TypedArray data allocation failed");
        ptr
    } else {
        std::ptr::null_mut()
    };

    // Initialise header fields.
    unsafe {
        // We must use ptr::write for AtomicU32 since the zeroed memory isn't
        // a valid AtomicU32 initialisation on all platforms (though in practice
        // it is on x86/ARM). Being explicit is correct.
        std::ptr::addr_of_mut!((*header_ptr).refcount).write(AtomicU32::new(1));
        (*header_ptr).kind = HEAP_KIND_TYPED_ARRAY;
        (*header_ptr).elem_type = elem_type;
        (*header_ptr)._pad = 0;
        (*header_ptr).data = data_ptr;
        (*header_ptr).len = 0;
        (*header_ptr).cap = cap;
    }

    header_ptr
}

/// Free a `TypedArrayHeader` and its element buffer.
///
/// # Safety
///
/// `ptr` must have been returned by [`typed_array_alloc`] and must not be used
/// after this call.
pub fn typed_array_free(ptr: *mut TypedArrayHeader) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let cap = (*ptr).cap;
        let et_raw = (*ptr).elem_type;
        let data = (*ptr).data;

        // Free the data buffer.
        if let Some(et) = ElemType::from_u8(et_raw) {
            if let Some(layout) = data_layout(et, cap) {
                if !data.is_null() {
                    alloc::dealloc(data, layout);
                }
            }
        }

        // Free the header itself.
        alloc::dealloc(ptr as *mut u8, Layout::new::<TypedArrayHeader>());
    }
}

// ---------------------------------------------------------------------------
// Element access — f64
// ---------------------------------------------------------------------------

/// Read `arr[index]` as `f64`.
///
/// # Panics
///
/// Panics if `index >= len` or `elem_type != ElemType::F64`.
#[inline]
pub fn typed_array_get_f64(arr: *mut TypedArrayHeader, index: u32) -> f64 {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::F64 as u8);
        assert!(index < (*arr).len, "index out of bounds");
        let ptr = (*arr).data as *const f64;
        *ptr.add(index as usize)
    }
}

/// Push an `f64` element. Returns the (possibly reallocated) header pointer.
///
/// If `len == cap`, the buffer is grown by 2x (minimum 4 elements).
///
/// # Panics
///
/// Panics if `elem_type != ElemType::F64` or allocation fails.
pub fn typed_array_push_f64(arr: *mut TypedArrayHeader, val: f64) -> *mut TypedArrayHeader {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::F64 as u8);

        if (*arr).len == (*arr).cap {
            grow(arr, ElemType::F64);
        }

        let ptr = (*arr).data as *mut f64;
        ptr.add((*arr).len as usize).write(val);
        (*arr).len += 1;
    }
    arr
}

// ---------------------------------------------------------------------------
// Element access — i64
// ---------------------------------------------------------------------------

/// Read `arr[index]` as `i64`.
#[inline]
pub fn typed_array_get_i64(arr: *mut TypedArrayHeader, index: u32) -> i64 {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::I64 as u8);
        assert!(index < (*arr).len, "index out of bounds");
        let ptr = (*arr).data as *const i64;
        *ptr.add(index as usize)
    }
}

/// Push an `i64` element. Returns the (possibly reallocated) header pointer.
pub fn typed_array_push_i64(arr: *mut TypedArrayHeader, val: i64) -> *mut TypedArrayHeader {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::I64 as u8);

        if (*arr).len == (*arr).cap {
            grow(arr, ElemType::I64);
        }

        let ptr = (*arr).data as *mut i64;
        ptr.add((*arr).len as usize).write(val);
        (*arr).len += 1;
    }
    arr
}

// ---------------------------------------------------------------------------
// Element access — i32
// ---------------------------------------------------------------------------

/// Read `arr[index]` as `i32`.
#[inline]
pub fn typed_array_get_i32(arr: *mut TypedArrayHeader, index: u32) -> i32 {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::I32 as u8);
        assert!(index < (*arr).len, "index out of bounds");
        let ptr = (*arr).data as *const i32;
        *ptr.add(index as usize)
    }
}

/// Push an `i32` element. Returns the (possibly reallocated) header pointer.
pub fn typed_array_push_i32(arr: *mut TypedArrayHeader, val: i32) -> *mut TypedArrayHeader {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::I32 as u8);

        if (*arr).len == (*arr).cap {
            grow(arr, ElemType::I32);
        }

        let ptr = (*arr).data as *mut i32;
        ptr.add((*arr).len as usize).write(val);
        (*arr).len += 1;
    }
    arr
}

// ---------------------------------------------------------------------------
// Element access — bool (u8)
// ---------------------------------------------------------------------------

/// Read `arr[index]` as `bool`.
#[inline]
pub fn typed_array_get_bool(arr: *mut TypedArrayHeader, index: u32) -> bool {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::Bool as u8);
        assert!(index < (*arr).len, "index out of bounds");
        let ptr = (*arr).data;
        *ptr.add(index as usize) != 0
    }
}

/// Push a `bool` element. Returns the (possibly reallocated) header pointer.
pub fn typed_array_push_bool(arr: *mut TypedArrayHeader, val: bool) -> *mut TypedArrayHeader {
    unsafe {
        debug_assert_eq!((*arr).elem_type, ElemType::Bool as u8);

        if (*arr).len == (*arr).cap {
            grow(arr, ElemType::Bool);
        }

        let ptr = (*arr).data;
        ptr.add((*arr).len as usize).write(val as u8);
        (*arr).len += 1;
    }
    arr
}

// ---------------------------------------------------------------------------
// Growth
// ---------------------------------------------------------------------------

/// Grow the element buffer of `arr` by 2x (minimum 4 elements).
///
/// # Safety
///
/// Caller must ensure `arr` is a valid `TypedArrayHeader` pointer.
unsafe fn grow(arr: *mut TypedArrayHeader, et: ElemType) {
    let old_cap = unsafe { (*arr).cap };
    let new_cap = if old_cap == 0 { 4 } else { old_cap * 2 };

    let new_layout = data_layout(et, new_cap).expect("overflow in grow");

    let new_data = if old_cap == 0 || unsafe { (*arr).data.is_null() } {
        unsafe { alloc::alloc_zeroed(new_layout) }
    } else {
        let old_layout = data_layout(et, old_cap).unwrap();
        unsafe { alloc::realloc((*arr).data, old_layout, new_layout.size()) }
    };
    assert!(!new_data.is_null(), "TypedArray grow allocation failed");

    // Zero the newly added portion (realloc doesn't guarantee zeroing).
    if old_cap > 0 {
        let old_bytes = et.size() * old_cap as usize;
        let new_bytes = new_layout.size();
        if new_bytes > old_bytes {
            unsafe { std::ptr::write_bytes(new_data.add(old_bytes), 0, new_bytes - old_bytes) };
        }
    }

    unsafe {
        (*arr).data = new_data;
        (*arr).cap = new_cap;
    }
}

// ---------------------------------------------------------------------------
// Refcounting helpers
// ---------------------------------------------------------------------------

/// Increment the reference count. Returns the new count.
#[inline]
pub fn typed_array_retain(arr: *mut TypedArrayHeader) -> u32 {
    unsafe { (*arr).refcount.fetch_add(1, Ordering::Relaxed) + 1 }
}

/// Decrement the reference count. If it reaches zero, frees the array.
/// Returns the new count.
#[inline]
pub fn typed_array_release(arr: *mut TypedArrayHeader) -> u32 {
    unsafe {
        let prev = (*arr).refcount.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            // Ensure all accesses to the data happen-before deallocation.
            std::sync::atomic::fence(Ordering::Acquire);
            typed_array_free(arr);
            return 0;
        }
        prev - 1
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    // -----------------------------------------------------------------------
    // Layout tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_header_size() {
        assert_eq!(mem::size_of::<TypedArrayHeader>(), 24);
    }

    #[test]
    fn test_header_field_offsets() {
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, refcount), 0);
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, kind), 4);
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, elem_type), 6);
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, _pad), 7);
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, data), 8);
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, len), 16);
        assert_eq!(std::mem::offset_of!(TypedArrayHeader, cap), 20);
    }

    #[test]
    fn test_elem_type_sizes() {
        assert_eq!(ElemType::F64.size(), 8);
        assert_eq!(ElemType::I64.size(), 8);
        assert_eq!(ElemType::I32.size(), 4);
        assert_eq!(ElemType::U32.size(), 4);
        assert_eq!(ElemType::Bool.size(), 1);
        assert_eq!(ElemType::Ptr.size(), 8);
    }

    #[test]
    fn test_elem_type_from_u8_roundtrip() {
        for disc in 1..=6u8 {
            let et = ElemType::from_u8(disc).unwrap();
            assert_eq!(et as u8, disc);
        }
        assert!(ElemType::from_u8(0).is_none());
        assert!(ElemType::from_u8(7).is_none());
        assert!(ElemType::from_u8(255).is_none());
    }

    // -----------------------------------------------------------------------
    // Alloc / free
    // -----------------------------------------------------------------------

    #[test]
    fn test_alloc_and_free_f64() {
        let arr = typed_array_alloc(ElemType::F64 as u8, 8);
        assert!(!arr.is_null());
        unsafe {
            assert_eq!((*arr).refcount.load(Ordering::Relaxed), 1);
            assert_eq!((*arr).kind, HEAP_KIND_TYPED_ARRAY);
            assert_eq!((*arr).elem_type, ElemType::F64 as u8);
            assert_eq!((*arr).len, 0);
            assert_eq!((*arr).cap, 8);
            assert!(!(*arr).data.is_null());
        }
        typed_array_free(arr);
    }

    #[test]
    fn test_alloc_zero_cap() {
        let arr = typed_array_alloc(ElemType::I32 as u8, 0);
        assert!(!arr.is_null());
        unsafe {
            assert_eq!((*arr).len, 0);
            assert_eq!((*arr).cap, 0);
            assert!((*arr).data.is_null());
        }
        typed_array_free(arr);
    }

    // -----------------------------------------------------------------------
    // f64 get / push
    // -----------------------------------------------------------------------

    #[test]
    fn test_push_and_get_f64() {
        let mut arr = typed_array_alloc(ElemType::F64 as u8, 4);
        arr = typed_array_push_f64(arr, 1.0);
        arr = typed_array_push_f64(arr, 2.5);
        arr = typed_array_push_f64(arr, -3.14);

        unsafe { assert_eq!((*arr).len, 3); }

        assert_eq!(typed_array_get_f64(arr, 0), 1.0);
        assert_eq!(typed_array_get_f64(arr, 1), 2.5);
        assert_eq!(typed_array_get_f64(arr, 2), -3.14);

        typed_array_free(arr);
    }

    #[test]
    fn test_push_f64_triggers_grow() {
        // Start with cap=0, every push must grow.
        let mut arr = typed_array_alloc(ElemType::F64 as u8, 0);
        for i in 0..20 {
            arr = typed_array_push_f64(arr, i as f64);
        }
        unsafe {
            assert_eq!((*arr).len, 20);
            assert!((*arr).cap >= 20);
        }
        for i in 0..20 {
            assert_eq!(typed_array_get_f64(arr, i), i as f64);
        }
        typed_array_free(arr);
    }

    // -----------------------------------------------------------------------
    // i64 get / push
    // -----------------------------------------------------------------------

    #[test]
    fn test_push_and_get_i64() {
        let mut arr = typed_array_alloc(ElemType::I64 as u8, 2);
        arr = typed_array_push_i64(arr, 42);
        arr = typed_array_push_i64(arr, -100);
        arr = typed_array_push_i64(arr, i64::MAX);

        assert_eq!(typed_array_get_i64(arr, 0), 42);
        assert_eq!(typed_array_get_i64(arr, 1), -100);
        assert_eq!(typed_array_get_i64(arr, 2), i64::MAX);

        typed_array_free(arr);
    }

    // -----------------------------------------------------------------------
    // i32 get / push
    // -----------------------------------------------------------------------

    #[test]
    fn test_push_and_get_i32() {
        let mut arr = typed_array_alloc(ElemType::I32 as u8, 4);
        arr = typed_array_push_i32(arr, 7);
        arr = typed_array_push_i32(arr, -1);

        assert_eq!(typed_array_get_i32(arr, 0), 7);
        assert_eq!(typed_array_get_i32(arr, 1), -1);

        typed_array_free(arr);
    }

    // -----------------------------------------------------------------------
    // bool get / push
    // -----------------------------------------------------------------------

    #[test]
    fn test_push_and_get_bool() {
        let mut arr = typed_array_alloc(ElemType::Bool as u8, 4);
        arr = typed_array_push_bool(arr, true);
        arr = typed_array_push_bool(arr, false);
        arr = typed_array_push_bool(arr, true);

        assert!(typed_array_get_bool(arr, 0));
        assert!(!typed_array_get_bool(arr, 1));
        assert!(typed_array_get_bool(arr, 2));

        typed_array_free(arr);
    }

    // -----------------------------------------------------------------------
    // Bounds check
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_get_f64_out_of_bounds() {
        let arr = typed_array_alloc(ElemType::F64 as u8, 4);
        typed_array_push_f64(arr, 1.0);
        typed_array_get_f64(arr, 1); // len=1, index=1 => panic
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_get_empty_array_panics() {
        let arr = typed_array_alloc(ElemType::I32 as u8, 4);
        typed_array_get_i32(arr, 0); // len=0 => panic
    }

    // -----------------------------------------------------------------------
    // Refcount
    // -----------------------------------------------------------------------

    #[test]
    fn test_refcount_retain_release() {
        let arr = typed_array_alloc(ElemType::F64 as u8, 4);
        unsafe { assert_eq!((*arr).refcount.load(Ordering::Relaxed), 1); }

        assert_eq!(typed_array_retain(arr), 2);
        assert_eq!(typed_array_retain(arr), 3);
        unsafe { assert_eq!((*arr).refcount.load(Ordering::Relaxed), 3); }

        assert_eq!(typed_array_release(arr), 2);
        assert_eq!(typed_array_release(arr), 1);
        // Final release frees; we don't dereference after this.
        let rc = typed_array_release(arr);
        assert_eq!(rc, 0);
    }

    // -----------------------------------------------------------------------
    // Indexing arithmetic sanity check
    // -----------------------------------------------------------------------

    #[test]
    fn test_element_pointer_arithmetic() {
        // Verify that `data + index * elem_size` gives the right address.
        let arr = typed_array_alloc(ElemType::F64 as u8, 4);
        typed_array_push_f64(arr, 0.0);
        typed_array_push_f64(arr, 0.0);
        unsafe {
            let base = (*arr).data as usize;
            let p0 = ((*arr).data as *const f64).add(0) as usize;
            let p1 = ((*arr).data as *const f64).add(1) as usize;
            assert_eq!(p0, base);
            assert_eq!(p1, base + 8); // f64 = 8 bytes
        }
        typed_array_free(arr);

        let arr = typed_array_alloc(ElemType::I32 as u8, 4);
        typed_array_push_i32(arr, 0);
        typed_array_push_i32(arr, 0);
        unsafe {
            let base = (*arr).data as usize;
            let p0 = ((*arr).data as *const i32).add(0) as usize;
            let p1 = ((*arr).data as *const i32).add(1) as usize;
            assert_eq!(p0, base);
            assert_eq!(p1, base + 4); // i32 = 4 bytes
        }
        typed_array_free(arr);

        let arr = typed_array_alloc(ElemType::Bool as u8, 4);
        typed_array_push_bool(arr, false);
        typed_array_push_bool(arr, false);
        unsafe {
            let base = (*arr).data as usize;
            let p1 = (*arr).data.add(1) as usize;
            assert_eq!(p1, base + 1); // bool = 1 byte
        }
        typed_array_free(arr);
    }
}
