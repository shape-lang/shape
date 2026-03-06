//! GcPtr<T> — typed pointer to a GC-managed object.
//!
//! Wraps a raw pointer to a T that is preceded by a GcHeader at ptr - 8.
//! GcPtr does NOT implement Drop — the GC handles deallocation.
//!
//! ## Platform-specific mark bit encoding
//!
//! On platforms with hardware pointer masking, mark bits are stored inline
//! in the upper pointer bits (zero-cost read/write):
//! - **ARM64 TBI**: bit 56 (top byte is ignored by hardware)
//! - **x86-64 LAM57**: bit 57 (masked by Linear Address Masking)
//! - **Software fallback**: delegates to the GcHeader side table (the header's
//!   color field) since we cannot store metadata in pointer bits without HW support.
//!
//! The inline mark bit is a fast-path optimization for the concurrent marker:
//! checking `has_mark_bit()` avoids a cache-line fetch of the GcHeader during
//! the mark phase scan.

use crate::header::GcHeader;
use std::marker::PhantomData;

/// A pointer to a GC-managed object of type T.
///
/// The object is laid out in memory as:
/// ```text
/// [GcHeader (8 bytes)][T data (size bytes)]
///                      ^--- GcPtr points here
/// ```
///
/// GcPtr is Copy — no refcount manipulation. The GC is solely responsible
/// for determining liveness and reclaiming memory.
#[repr(transparent)]
pub struct GcPtr<T> {
    ptr: *mut T,
    _marker: PhantomData<T>,
}

// GcPtr is Copy — no refcounting, GC manages lifetime
impl<T> Copy for GcPtr<T> {}
impl<T> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> GcPtr<T> {
    /// Create a GcPtr from a raw pointer.
    ///
    /// # Safety
    /// The pointer must point to a valid T preceded by a GcHeader at ptr - 8.
    #[inline(always)]
    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        debug_assert!(!ptr.is_null(), "GcPtr::from_raw called with null pointer");
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Get the raw pointer to T.
    #[inline(always)]
    pub fn as_ptr(self) -> *mut T {
        self.ptr
    }

    /// Get the raw pointer as usize (for NaN-boxing payload).
    #[inline(always)]
    pub fn as_usize(self) -> usize {
        self.ptr as usize
    }

    /// Get a reference to the GcHeader preceding this object.
    #[inline(always)]
    pub fn header(self) -> &'static GcHeader {
        unsafe {
            let header_ptr =
                (self.ptr as *const u8).sub(std::mem::size_of::<GcHeader>()) as *const GcHeader;
            &*header_ptr
        }
    }

    /// Get a mutable reference to the GcHeader preceding this object.
    ///
    /// # Safety
    /// Caller must ensure exclusive access to the header.
    #[inline(always)]
    pub unsafe fn header_mut(self) -> &'static mut GcHeader {
        unsafe {
            let header_ptr =
                (self.ptr as *mut u8).sub(std::mem::size_of::<GcHeader>()) as *mut GcHeader;
            &mut *header_ptr
        }
    }

    /// Dereference to get a reference to the managed object.
    ///
    /// # Safety
    /// The object must still be alive (not collected).
    #[inline(always)]
    pub unsafe fn deref_gc(self) -> &'static T {
        unsafe { &*self.ptr }
    }

    /// Dereference to get a mutable reference to the managed object.
    ///
    /// # Safety
    /// The object must still be alive and caller must have exclusive access.
    #[inline(always)]
    pub unsafe fn deref_gc_mut(self) -> &'static mut T {
        unsafe { &mut *self.ptr }
    }

    // ── Platform-specific mark bit encoding ─────────────────────────

    /// Bit position for the inline mark bit on ARM64 (TBI — top byte ignore).
    const MARK_BIT_ARM64: usize = 56;

    /// Bit position for the inline mark bit on x86-64 (LAM57).
    const MARK_BIT_X86_LAM: usize = 57;

    /// Set the inline mark bit in the pointer, returning a new GcPtr.
    ///
    /// On ARM64 TBI the mark lives in bit 56. On x86-64 with LAM it lives in
    /// bit 57. On software-fallback platforms this is a no-op (the caller must
    /// use the GcHeader side table instead).
    #[cfg(target_arch = "aarch64")]
    #[inline(always)]
    pub fn with_mark_bit(self) -> Self {
        // ARM TBI: hardware ignores the top byte — bit 56 is safe metadata
        Self {
            ptr: (self.ptr as usize | (1 << Self::MARK_BIT_ARM64)) as *mut T,
            _marker: PhantomData,
        }
    }

    /// Clear the inline mark bit, returning a new GcPtr.
    #[cfg(target_arch = "aarch64")]
    #[inline(always)]
    pub fn clear_mark_bit(self) -> Self {
        Self {
            ptr: (self.ptr as usize & !(1 << Self::MARK_BIT_ARM64)) as *mut T,
            _marker: PhantomData,
        }
    }

    /// Check whether the inline mark bit is set.
    #[cfg(target_arch = "aarch64")]
    #[inline(always)]
    pub fn has_mark_bit(self) -> bool {
        (self.ptr as usize & (1 << Self::MARK_BIT_ARM64)) != 0
    }

    /// Set the inline mark bit (x86-64: use LAM if available, else no-op).
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub fn with_mark_bit(self) -> Self {
        if crate::platform::has_x86_lam() {
            Self {
                ptr: (self.ptr as usize | (1 << Self::MARK_BIT_X86_LAM)) as *mut T,
                _marker: PhantomData,
            }
        } else {
            // Software fallback: no inline mark — caller must use GcHeader
            self
        }
    }

    /// Clear the inline mark bit (x86-64: use LAM if available, else no-op).
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub fn clear_mark_bit(self) -> Self {
        if crate::platform::has_x86_lam() {
            Self {
                ptr: (self.ptr as usize & !(1 << Self::MARK_BIT_X86_LAM)) as *mut T,
                _marker: PhantomData,
            }
        } else {
            self
        }
    }

    /// Check whether the inline mark bit is set (x86-64: use LAM if available).
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub fn has_mark_bit(self) -> bool {
        if crate::platform::has_x86_lam() {
            (self.ptr as usize & (1 << Self::MARK_BIT_X86_LAM)) != 0
        } else {
            // Software fallback: always false — caller must check GcHeader
            false
        }
    }

    /// Set the inline mark bit (generic fallback for non-ARM64 / non-x86-64).
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    #[inline(always)]
    pub fn with_mark_bit(self) -> Self {
        // No hardware pointer masking — side table only
        self
    }

    /// Clear the inline mark bit (generic fallback).
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    #[inline(always)]
    pub fn clear_mark_bit(self) -> Self {
        self
    }

    /// Check whether the inline mark bit is set (generic fallback: always false).
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    #[inline(always)]
    pub fn has_mark_bit(self) -> bool {
        false
    }

    /// Check the mark state, combining inline pointer bits with the GcHeader
    /// side table for a definitive answer on any platform.
    ///
    /// Returns `true` if either the inline mark bit is set (on HW-masking
    /// platforms) or the GcHeader color is Gray/Black.
    #[inline(always)]
    pub fn is_marked(self) -> bool {
        if self.has_mark_bit() {
            return true;
        }
        // Fallback: check GcHeader color (always authoritative)
        let header = self.header();
        header.color() != crate::header::GcColor::White
    }

    /// Strip all metadata bits from the pointer, returning the raw address.
    ///
    /// Uses the detected masking mode to apply the correct mask.
    #[inline(always)]
    pub fn raw_ptr(self) -> *mut T {
        let mode = crate::platform::cached_masking_mode();
        crate::platform::mask_ptr(self.ptr as *mut u8, mode) as *mut T
    }

    /// Convert to an untyped pointer for use in the root set / marker.
    #[inline(always)]
    pub fn as_untyped(self) -> *mut u8 {
        self.ptr as *mut u8
    }

    /// Create from an untyped pointer.
    ///
    /// # Safety
    /// The pointer must actually point to a valid T with GcHeader prefix.
    #[inline(always)]
    pub unsafe fn from_untyped(ptr: *mut u8) -> Self {
        Self {
            ptr: ptr as *mut T,
            _marker: PhantomData,
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for GcPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GcPtr({:p})", self.ptr)
    }
}

impl<T> std::fmt::Pointer for GcPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Pointer::fmt(&self.ptr, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{GcColor, GcHeader};

    #[test]
    fn test_gc_ptr_is_8_bytes() {
        assert_eq!(std::mem::size_of::<GcPtr<u64>>(), 8);
    }

    #[test]
    fn test_gc_ptr_header_access() {
        // Simulate a GC allocation: [GcHeader][u64 data]
        let mut buf = [0u8; 16]; // 8 header + 8 data
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe {
            header_ptr.write(GcHeader::new(1, 8));
        }
        let data_ptr = unsafe { buf.as_mut_ptr().add(8) } as *mut u64;
        unsafe {
            data_ptr.write(42);
        }

        let gc_ptr = unsafe { GcPtr::<u64>::from_raw(data_ptr) };
        let header = gc_ptr.header();
        assert_eq!(header.kind, 1);
        assert_eq!(header.size, 8);

        let val = unsafe { gc_ptr.deref_gc() };
        assert_eq!(*val, 42);
    }

    // ── Mark bit tests ──────────────────────────────────────────────

    #[test]
    fn test_mark_bit_initial_state() {
        // On x86-64 without LAM (the common test environment), mark bit
        // methods are no-ops — has_mark_bit() always returns false.
        let mut buf = [0u8; 16];
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe { header_ptr.write(GcHeader::new(0, 8)) };
        let data_ptr = unsafe { buf.as_mut_ptr().add(8) } as *mut u64;
        unsafe { data_ptr.write(0) };

        let gc_ptr = unsafe { GcPtr::<u64>::from_raw(data_ptr) };
        assert!(!gc_ptr.has_mark_bit());
    }

    #[test]
    fn test_mark_bit_set_clear_cycle() {
        let mut buf = [0u8; 16];
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe { header_ptr.write(GcHeader::new(0, 8)) };
        let data_ptr = unsafe { buf.as_mut_ptr().add(8) } as *mut u64;
        unsafe { data_ptr.write(0xCAFE) };

        let gc_ptr = unsafe { GcPtr::<u64>::from_raw(data_ptr) };

        // Set mark bit
        let marked = gc_ptr.with_mark_bit();
        // Clear mark bit
        let cleared = marked.clear_mark_bit();

        // On software fallback (no HW masking), set/clear are no-ops,
        // so the pointer should be unchanged throughout.
        #[cfg(target_arch = "x86_64")]
        {
            if !crate::platform::has_x86_lam() {
                // Software fallback: mark bit operations are no-ops
                assert!(!marked.has_mark_bit());
                assert!(!cleared.has_mark_bit());
                assert_eq!(gc_ptr.as_ptr(), marked.as_ptr());
                assert_eq!(gc_ptr.as_ptr(), cleared.as_ptr());
            } else {
                // LAM available: mark bit should work
                assert!(marked.has_mark_bit());
                assert!(!cleared.has_mark_bit());
                // Clearing should recover the original pointer
                assert_eq!(gc_ptr.as_ptr(), cleared.as_ptr());
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // ARM TBI: mark bit always works
            assert!(marked.has_mark_bit());
            assert!(!cleared.has_mark_bit());
            // The raw address portion should be preserved
            assert_eq!(gc_ptr.raw_ptr(), marked.raw_ptr());
        }

        // Generic fallback check for other architectures
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        {
            assert!(!marked.has_mark_bit());
            assert!(!cleared.has_mark_bit());
            assert_eq!(gc_ptr.as_ptr(), marked.as_ptr());
        }
    }

    #[test]
    fn test_mark_bit_idempotent() {
        let mut buf = [0u8; 16];
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe { header_ptr.write(GcHeader::new(0, 8)) };
        let data_ptr = unsafe { buf.as_mut_ptr().add(8) } as *mut u64;
        unsafe { data_ptr.write(0) };

        let gc_ptr = unsafe { GcPtr::<u64>::from_raw(data_ptr) };
        let marked_once = gc_ptr.with_mark_bit();
        let marked_twice = marked_once.with_mark_bit();
        // Applying mark bit twice should yield the same pointer
        assert_eq!(marked_once.as_ptr(), marked_twice.as_ptr());
    }

    #[test]
    fn test_is_marked_uses_header_fallback() {
        // is_marked() should return true when the GcHeader color is not White,
        // even when the inline mark bit is not set (software fallback).
        let mut buf = [0u8; 16];
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe { header_ptr.write(GcHeader::new(0, 8)) };
        let data_ptr = unsafe { buf.as_mut_ptr().add(8) } as *mut u64;
        unsafe { data_ptr.write(42) };

        let gc_ptr = unsafe { GcPtr::<u64>::from_raw(data_ptr) };

        // Initially white — not marked
        assert!(!gc_ptr.is_marked());

        // Set header to Gray — is_marked should return true via fallback
        let header = unsafe { gc_ptr.header_mut() };
        header.set_color(GcColor::Gray);
        assert!(gc_ptr.is_marked());

        // Set header to Black — still marked
        header.set_color(GcColor::Black);
        assert!(gc_ptr.is_marked());

        // Reset to White — no longer marked
        header.set_color(GcColor::White);
        assert!(!gc_ptr.is_marked());
    }

    #[test]
    fn test_raw_ptr_strips_metadata() {
        let mut buf = [0u8; 16];
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe { header_ptr.write(GcHeader::new(0, 8)) };
        let data_ptr = unsafe { buf.as_mut_ptr().add(8) } as *mut u64;
        unsafe { data_ptr.write(0) };

        let gc_ptr = unsafe { GcPtr::<u64>::from_raw(data_ptr) };
        let marked = gc_ptr.with_mark_bit();

        // raw_ptr() should strip any metadata bits and return the clean address.
        // On software fallback, with_mark_bit is a no-op so raw_ptr == as_ptr.
        let raw = marked.raw_ptr();
        // The raw pointer's lower 48 bits must match the original data_ptr
        assert_eq!(
            raw as usize & 0x0000_FFFF_FFFF_FFFF,
            data_ptr as usize & 0x0000_FFFF_FFFF_FFFF,
        );
    }
}
