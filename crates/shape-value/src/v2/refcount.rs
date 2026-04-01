//! Inline refcount operations for v2 heap objects.
//!
//! These free functions operate on raw `*const HeapHeader` pointers, suitable
//! for use from JIT-generated code and FFI. The caller is responsible for
//! pointer validity.

use super::heap_header::HeapHeader;
use std::sync::atomic::Ordering;

/// Increment the reference count of a v2 heap object.
///
/// # Safety
/// `ptr` must point to a valid, live `HeapHeader`.
#[inline(always)]
pub unsafe fn v2_retain(ptr: *const HeapHeader) {
    unsafe { (*ptr).refcount.fetch_add(1, Ordering::Relaxed) };
}

/// Decrement the reference count of a v2 heap object.
/// Returns `true` if the count reached zero (caller must deallocate).
///
/// Uses Release ordering on the decrement and an Acquire fence when the
/// count reaches zero, ensuring all prior writes are visible before deallocation.
///
/// # Safety
/// `ptr` must point to a valid, live `HeapHeader`.
/// If this returns `true`, the caller must deallocate the object and must not
/// access it again.
#[inline(always)]
pub unsafe fn v2_release(ptr: *const HeapHeader) -> bool {
    let old = unsafe { (*ptr).refcount.fetch_sub(1, Ordering::Release) };
    if old == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        true // caller must dealloc
    } else {
        false
    }
}

/// Get the current reference count of a v2 heap object.
///
/// # Safety
/// `ptr` must point to a valid, live `HeapHeader`.
#[inline(always)]
pub unsafe fn v2_get_refcount(ptr: *const HeapHeader) -> u32 {
    unsafe { (*ptr).refcount.load(Ordering::Relaxed) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::heap_header::HEAP_KIND_V2_TYPED_ARRAY;

    #[test]
    fn test_v2_retain_increments() {
        let h = HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY);
        unsafe {
            let ptr = &h as *const HeapHeader;
            assert_eq!(v2_get_refcount(ptr), 1);

            v2_retain(ptr);
            assert_eq!(v2_get_refcount(ptr), 2);

            v2_retain(ptr);
            assert_eq!(v2_get_refcount(ptr), 3);
        }
    }

    #[test]
    fn test_v2_release_decrements() {
        let h = HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY);
        unsafe {
            let ptr = &h as *const HeapHeader;
            v2_retain(ptr); // 2
            v2_retain(ptr); // 3

            assert!(!v2_release(ptr)); // 3 -> 2
            assert_eq!(v2_get_refcount(ptr), 2);

            assert!(!v2_release(ptr)); // 2 -> 1
            assert_eq!(v2_get_refcount(ptr), 1);

            assert!(v2_release(ptr)); // 1 -> 0, caller must dealloc
        }
    }

    #[test]
    fn test_v2_release_returns_true_on_last_ref() {
        let h = HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY);
        unsafe {
            let ptr = &h as *const HeapHeader;
            assert!(v2_release(ptr)); // 1 -> 0
        }
    }

    #[test]
    fn test_v2_retain_release_thread_safety() {
        use std::sync::Arc;

        // Box the header so it has a stable address, then wrap in Arc for sharing.
        let header = Arc::new(HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY));

        let threads: Vec<_> = (0..8)
            .map(|_| {
                let h = Arc::clone(&header);
                std::thread::spawn(move || {
                    let ptr = &*h as *const HeapHeader;
                    for _ in 0..1000 {
                        unsafe { v2_retain(ptr) };
                    }
                    for _ in 0..1000 {
                        unsafe { v2_release(ptr) };
                    }
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        unsafe {
            let ptr = &*header as *const HeapHeader;
            assert_eq!(v2_get_refcount(ptr), 1);
        }
    }

    #[test]
    fn test_v2_refcount_operations_match_header_methods() {
        let h = HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY);
        unsafe {
            let ptr = &h as *const HeapHeader;

            // Free-function and method should agree
            assert_eq!(v2_get_refcount(ptr), h.get_refcount());

            v2_retain(ptr);
            assert_eq!(v2_get_refcount(ptr), h.get_refcount());
            assert_eq!(h.get_refcount(), 2);

            let should_dealloc = v2_release(ptr);
            assert!(!should_dealloc);
            assert_eq!(v2_get_refcount(ptr), h.get_refcount());
            assert_eq!(h.get_refcount(), 1);
        }
    }
}
