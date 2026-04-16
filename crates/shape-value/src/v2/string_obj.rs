//! Refcounted, repr(C) string for v2 runtime.
//!
//! ## Memory layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader)
//!   8       8   data (*const u8, UTF-8 bytes, NOT null-terminated)
//!  16       4   len (u32, byte count)
//!  20       4   _pad (u32)
//! ```

use super::heap_header::{HeapHeader, HEAP_KIND_V2_STRING};
use crate::value_word::ValueWordExt;

/// Refcounted, repr(C) string for v2 runtime.
/// Total: 24 bytes (header 8 + data ptr 8 + len 4 + pad 4).
#[repr(C)]
pub struct StringObj {
    pub header: HeapHeader,
    /// Pointer to UTF-8 bytes. NOT null-terminated.
    pub data: *const u8,
    /// Byte length (not char count).
    pub len: u32,
    pub _pad: u32,
}

impl StringObj {
    /// Allocate a new StringObj from a `&str`. Copies the bytes.
    pub fn new(s: &str) -> *mut Self {
        let layout = std::alloc::Layout::new::<Self>();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut Self };

        // Allocate and copy string data
        let data = if s.is_empty() {
            std::ptr::null()
        } else {
            let data_layout = std::alloc::Layout::from_size_align(s.len(), 1).unwrap();
            let data_ptr = unsafe { std::alloc::alloc(data_layout) };
            unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), data_ptr, s.len()) };
            data_ptr as *const u8
        };

        unsafe {
            (*ptr).header = HeapHeader::new(HEAP_KIND_V2_STRING);
            (*ptr).data = data;
            (*ptr).len = s.len() as u32;
            (*ptr)._pad = 0;
        }
        ptr
    }

    /// Get string as `&str`.
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `StringObj`.
    pub unsafe fn as_str(ptr: *const Self) -> &'static str {
        unsafe {
            if (*ptr).len == 0 {
                ""
            } else {
                let bytes = std::slice::from_raw_parts((*ptr).data, (*ptr).len as usize);
                std::str::from_utf8_unchecked(bytes)
            }
        }
    }

    /// Byte length of the string.
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `StringObj`.
    pub unsafe fn len(ptr: *const Self) -> u32 {
        unsafe { (*ptr).len }
    }

    /// Whether the string is empty.
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `StringObj`.
    pub unsafe fn is_empty(ptr: *const Self) -> bool {
        unsafe { (*ptr).len == 0 }
    }

    /// Free the StringObj and its data buffer.
    ///
    /// # Safety
    /// `ptr` must point to a valid `StringObj` with no remaining references.
    /// Must not be called more than once on the same pointer.
    pub unsafe fn drop(ptr: *mut Self) {
        unsafe {
            if (*ptr).len > 0 && !(*ptr).data.is_null() {
                let data_layout =
                    std::alloc::Layout::from_size_align((*ptr).len as usize, 1).unwrap();
                std::alloc::dealloc((*ptr).data as *mut u8, data_layout);
            }
            let layout = std::alloc::Layout::new::<Self>();
            std::alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    /// Byte offset constants for JIT codegen.
    pub const OFFSET_DATA: usize = 8;
    pub const OFFSET_LEN: usize = 16;
}

// Compile-time size assertion.
const _: () = {
    assert!(std::mem::size_of::<StringObj>() == 24);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_of_string_obj() {
        assert_eq!(std::mem::size_of::<StringObj>(), 24);
    }

    #[test]
    fn test_create_and_read_hello() {
        unsafe {
            let ptr = StringObj::new("hello");
            assert_eq!(StringObj::as_str(ptr), "hello");
            assert_eq!(StringObj::len(ptr), 5);
            assert!(!StringObj::is_empty(ptr));
            assert_eq!((*ptr).header.kind(), HEAP_KIND_V2_STRING);
            assert_eq!((*ptr).header.get_refcount(), 1);
            StringObj::drop(ptr);
        }
    }

    #[test]
    fn test_unicode_string() {
        unsafe {
            let s = "日本語";
            let ptr = StringObj::new(s);
            assert_eq!(StringObj::as_str(ptr), "日本語");
            // 3 chars x 3 bytes each = 9 bytes
            assert_eq!(StringObj::len(ptr), 9);
            assert!(!StringObj::is_empty(ptr));
            StringObj::drop(ptr);
        }
    }

    #[test]
    fn test_empty_string() {
        unsafe {
            let ptr = StringObj::new("");
            assert_eq!(StringObj::as_str(ptr), "");
            assert_eq!(StringObj::len(ptr), 0);
            assert!(StringObj::is_empty(ptr));
            StringObj::drop(ptr);
        }
    }

    #[test]
    fn test_drop_does_not_leak() {
        // Create and drop several strings — under Miri/valgrind this would catch leaks.
        unsafe {
            for _ in 0..100 {
                let ptr = StringObj::new("leak test string with some content");
                StringObj::drop(ptr);
            }
            // Empty string variant
            for _ in 0..100 {
                let ptr = StringObj::new("");
                StringObj::drop(ptr);
            }
        }
    }

    #[test]
    fn test_field_offsets() {
        let ptr = StringObj::new("test");
        let base = ptr as usize;
        unsafe {
            let data_offset = &(*ptr).data as *const _ as usize - base;
            let len_offset = &(*ptr).len as *const _ as usize - base;

            assert_eq!(data_offset, StringObj::OFFSET_DATA, "data must be at offset 8");
            assert_eq!(len_offset, StringObj::OFFSET_LEN, "len must be at offset 16");

            StringObj::drop(ptr);
        }
    }

    #[test]
    fn test_refcount_starts_at_one() {
        unsafe {
            let ptr = StringObj::new("refcount test");
            assert_eq!((*ptr).header.get_refcount(), 1);
            StringObj::drop(ptr);
        }
    }

    #[test]
    fn test_emoji_string() {
        unsafe {
            let s = "hello 🌍🚀✨";
            let ptr = StringObj::new(s);
            assert_eq!(StringObj::as_str(ptr), s);
            assert_eq!(StringObj::len(ptr), s.len() as u32);
            StringObj::drop(ptr);
        }
    }

    #[test]
    fn test_long_string_1mb() {
        unsafe {
            let s = "x".repeat(1_000_000);
            let ptr = StringObj::new(&s);
            assert_eq!(StringObj::len(ptr), 1_000_000);
            assert_eq!(StringObj::as_str(ptr), s.as_str());
            StringObj::drop(ptr);
        }
    }
}
