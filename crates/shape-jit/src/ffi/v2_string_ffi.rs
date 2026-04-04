//! v2 String FFI functions for JIT-compiled code.
//!
//! These functions operate on the v2 `StringObj` layout — a compact, C-compatible
//! representation that the JIT can manipulate with native pointer types instead
//! of NaN-boxed u64 values.
//!
//! ## StringObj memory layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       4   refcount (u32)
//!   4       2   kind (u16, = HK_STRING = 0)
//!   6       2   (padding)
//!   8       8   data (*const u8, UTF-8 bytes)
//!  16       4   len (u32, byte length)
//!  20       4   (padding)
//! ```
//!
//! The 8-byte header at offset 0 packs refcount + kind + padding so that
//! the `data` pointer is naturally 8-byte aligned.

use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// StringObj repr
// ---------------------------------------------------------------------------

/// Heap kind constant for strings (matches HeapKind::String = 0).
const HK_STRING_U16: u16 = 0;

/// A compact, repr(C) string object for the v2 runtime.
///
/// The JIT emits raw loads/stores at known offsets, so every field position
/// must be stable.
#[repr(C)]
struct StringObj {
    /// Reference count (offset 0). Accessed atomically.
    refcount: AtomicU32,
    /// Heap kind discriminator (offset 4). Always `HK_STRING_U16`.
    kind: u16,
    /// Padding to align `data` at offset 8.
    _pad_header: u16,
    /// Pointer to the UTF-8 byte buffer (offset 8). The buffer is allocated
    /// immediately after the StringObj header when created by `jit_v2_string_alloc`.
    data: *const u8,
    /// Byte length of the string (offset 16).
    len: u32,
    /// Padding to round the struct to 24 bytes (offset 20).
    _pad_tail: u32,
}

// Compile-time layout assertions.
const _: () = {
    assert!(std::mem::size_of::<StringObj>() == 24);
    // data field must be at offset 8
    // len field must be at offset 16
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Layout for a StringObj + trailing string data.
fn layout_for(byte_len: usize) -> Layout {
    // StringObj (24 bytes, 8-aligned) followed by `byte_len` bytes of string data.
    let header_layout = Layout::new::<StringObj>();
    let data_layout = Layout::from_size_align(byte_len.max(1), 1).unwrap();
    let (combined, _offset) = header_layout.extend(data_layout).unwrap();
    combined.pad_to_align()
}

#[inline]
unsafe fn as_obj(ptr: *mut u8) -> &'static StringObj {
    unsafe { &*(ptr as *const StringObj) }
}

#[inline]
unsafe fn as_obj_mut(ptr: *mut u8) -> &'static mut StringObj {
    unsafe { &mut *(ptr as *mut StringObj) }
}

// ---------------------------------------------------------------------------
// Public FFI functions
// ---------------------------------------------------------------------------

/// Allocate a new `StringObj` from raw UTF-8 bytes. Returns a raw pointer
/// to the `StringObj` (which the JIT treats as `*mut u8` / I64).
///
/// The string data is copied into a buffer allocated immediately after the
/// header, so the caller does not need to keep `data` alive.
///
/// Refcount is initialised to 1.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_alloc(data: *const u8, len: u32) -> *mut u8 {
    let byte_len = len as usize;
    let layout = layout_for(byte_len);

    unsafe {
        let ptr = alloc(layout);
        if ptr.is_null() {
            // OOM — return null; caller must handle.
            return std::ptr::null_mut();
        }

        // Data buffer starts right after the StringObj header.
        let data_dst = ptr.add(std::mem::size_of::<StringObj>());

        // Copy the source bytes into the trailing buffer.
        if byte_len > 0 && !data.is_null() {
            std::ptr::copy_nonoverlapping(data, data_dst, byte_len);
        }

        // Initialise the header fields.
        let obj = as_obj_mut(ptr);
        // Write refcount via raw pointer to avoid requiring &mut AtomicU32
        // before the memory is fully initialised.
        std::ptr::write(&raw mut obj.refcount, AtomicU32::new(1));
        obj.kind = HK_STRING_U16;
        obj._pad_header = 0;
        obj.data = data_dst;
        obj.len = len;
        obj._pad_tail = 0;

        ptr
    }
}

/// Return the byte length of the string.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_len(str_ptr: *mut u8) -> i64 {
    if str_ptr.is_null() {
        return 0;
    }
    unsafe { as_obj(str_ptr).len as i64 }
}

/// Return a pointer to the raw UTF-8 data bytes.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_data(str_ptr: *mut u8) -> *const u8 {
    if str_ptr.is_null() {
        return std::ptr::null();
    }
    unsafe { as_obj(str_ptr).data }
}

/// Concatenate two v2 strings. Returns a freshly allocated `StringObj` with
/// refcount 1. Neither input is consumed (their refcounts are unchanged).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_concat(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let a_len = if a.is_null() {
            0usize
        } else {
            as_obj(a).len as usize
        };
        let b_len = if b.is_null() {
            0usize
        } else {
            as_obj(b).len as usize
        };
        let total = a_len + b_len;

        let layout = layout_for(total);
        let ptr = alloc(layout);
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        let data_dst = ptr.add(std::mem::size_of::<StringObj>());

        // Copy a's data.
        if a_len > 0 {
            std::ptr::copy_nonoverlapping(as_obj(a).data, data_dst, a_len);
        }
        // Copy b's data.
        if b_len > 0 {
            std::ptr::copy_nonoverlapping(as_obj(b).data, data_dst.add(a_len), b_len);
        }

        let obj = as_obj_mut(ptr);
        std::ptr::write(&raw mut obj.refcount, AtomicU32::new(1));
        obj.kind = HK_STRING_U16;
        obj._pad_header = 0;
        obj.data = data_dst;
        obj.len = total as u32;
        obj._pad_tail = 0;

        ptr
    }
}

/// Compare two v2 strings for byte-equality. Returns 1 if equal, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_eq(a: *mut u8, b: *mut u8) -> u8 {
    if a == b {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }
    unsafe {
        let obj_a = as_obj(a);
        let obj_b = as_obj(b);
        if obj_a.len != obj_b.len {
            return 0;
        }
        let len = obj_a.len as usize;
        if len == 0 {
            return 1;
        }
        let slice_a = std::slice::from_raw_parts(obj_a.data, len);
        let slice_b = std::slice::from_raw_parts(obj_b.data, len);
        if slice_a == slice_b { 1 } else { 0 }
    }
}

/// Print a v2 string to stdout (with trailing newline), used by the `print`
/// builtin in JIT-compiled code.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_print(str_ptr: *mut u8) {
    if str_ptr.is_null() {
        println!();
        return;
    }
    unsafe {
        let obj = as_obj(str_ptr);
        let len = obj.len as usize;
        let slice = std::slice::from_raw_parts(obj.data, len);
        // Best-effort: interpret as UTF-8, replace invalid sequences.
        let s = std::str::from_utf8(slice).unwrap_or("<invalid utf8>");
        println!("{}", s);
    }
}

/// Increment the reference count (retain). No-op on null.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_retain(str_ptr: *mut u8) {
    if str_ptr.is_null() {
        return;
    }
    unsafe {
        as_obj(str_ptr).refcount.fetch_add(1, Ordering::Relaxed);
    }
}

/// Decrement the reference count (release). Deallocates the object when the
/// count reaches zero. No-op on null.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_string_release(str_ptr: *mut u8) {
    if str_ptr.is_null() {
        return;
    }
    unsafe {
        let obj = as_obj(str_ptr);
        // Acquire on the decrement so that all prior writes to the object are
        // visible before we potentially deallocate.
        let prev = obj.refcount.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            // Ensure all writes from other threads are visible before dealloc.
            std::sync::atomic::fence(Ordering::Acquire);
            let byte_len = obj.len as usize;
            let layout = layout_for(byte_len);
            dealloc(str_ptr, layout);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_obj_layout() {
        assert_eq!(std::mem::size_of::<StringObj>(), 24);

        // Verify field offsets using a zeroed instance.
        let obj = StringObj {
            refcount: AtomicU32::new(0),
            kind: 0,
            _pad_header: 0,
            data: std::ptr::null(),
            len: 0,
            _pad_tail: 0,
        };
        let base = &obj as *const _ as usize;
        assert_eq!(
            &obj.refcount as *const _ as usize - base,
            0,
            "refcount at offset 0"
        );
        assert_eq!(
            &obj.kind as *const _ as usize - base,
            4,
            "kind at offset 4"
        );
        assert_eq!(
            &obj.data as *const _ as usize - base,
            8,
            "data at offset 8"
        );
        assert_eq!(
            &obj.len as *const _ as usize - base,
            16,
            "len at offset 16"
        );
    }

    #[test]
    fn test_alloc_and_len() {
        let src = b"hello";
        let ptr = jit_v2_string_alloc(src.as_ptr(), src.len() as u32);
        assert!(!ptr.is_null());

        assert_eq!(jit_v2_string_len(ptr), 5);

        // Clean up.
        jit_v2_string_release(ptr);
    }

    #[test]
    fn test_alloc_empty() {
        let ptr = jit_v2_string_alloc(std::ptr::null(), 0);
        assert!(!ptr.is_null());
        assert_eq!(jit_v2_string_len(ptr), 0);
        jit_v2_string_release(ptr);
    }

    #[test]
    fn test_data_roundtrip() {
        let src = b"world";
        let ptr = jit_v2_string_alloc(src.as_ptr(), src.len() as u32);
        assert!(!ptr.is_null());

        let data = jit_v2_string_data(ptr);
        let len = jit_v2_string_len(ptr) as usize;
        let slice = unsafe { std::slice::from_raw_parts(data, len) };
        assert_eq!(slice, b"world");

        jit_v2_string_release(ptr);
    }

    #[test]
    fn test_concat() {
        let a = jit_v2_string_alloc(b"foo".as_ptr(), 3);
        let b = jit_v2_string_alloc(b"bar".as_ptr(), 3);
        let c = jit_v2_string_concat(a, b);
        assert!(!c.is_null());

        assert_eq!(jit_v2_string_len(c), 6);
        let data = jit_v2_string_data(c);
        let slice = unsafe { std::slice::from_raw_parts(data, 6) };
        assert_eq!(slice, b"foobar");

        jit_v2_string_release(a);
        jit_v2_string_release(b);
        jit_v2_string_release(c);
    }

    #[test]
    fn test_concat_with_empty() {
        let a = jit_v2_string_alloc(b"abc".as_ptr(), 3);
        let b = jit_v2_string_alloc(std::ptr::null(), 0);
        let c = jit_v2_string_concat(a, b);
        assert_eq!(jit_v2_string_len(c), 3);

        let data = jit_v2_string_data(c);
        let slice = unsafe { std::slice::from_raw_parts(data, 3) };
        assert_eq!(slice, b"abc");

        jit_v2_string_release(a);
        jit_v2_string_release(b);
        jit_v2_string_release(c);
    }

    #[test]
    fn test_eq_same_content() {
        let a = jit_v2_string_alloc(b"test".as_ptr(), 4);
        let b = jit_v2_string_alloc(b"test".as_ptr(), 4);
        assert_eq!(jit_v2_string_eq(a, b), 1);

        jit_v2_string_release(a);
        jit_v2_string_release(b);
    }

    #[test]
    fn test_eq_different_content() {
        let a = jit_v2_string_alloc(b"abc".as_ptr(), 3);
        let b = jit_v2_string_alloc(b"xyz".as_ptr(), 3);
        assert_eq!(jit_v2_string_eq(a, b), 0);

        jit_v2_string_release(a);
        jit_v2_string_release(b);
    }

    #[test]
    fn test_eq_different_lengths() {
        let a = jit_v2_string_alloc(b"ab".as_ptr(), 2);
        let b = jit_v2_string_alloc(b"abc".as_ptr(), 3);
        assert_eq!(jit_v2_string_eq(a, b), 0);

        jit_v2_string_release(a);
        jit_v2_string_release(b);
    }

    #[test]
    fn test_eq_same_pointer() {
        let a = jit_v2_string_alloc(b"dup".as_ptr(), 3);
        assert_eq!(jit_v2_string_eq(a, a), 1);
        jit_v2_string_release(a);
    }

    #[test]
    fn test_eq_null() {
        let a = jit_v2_string_alloc(b"x".as_ptr(), 1);
        assert_eq!(jit_v2_string_eq(a, std::ptr::null_mut()), 0);
        assert_eq!(jit_v2_string_eq(std::ptr::null_mut(), a), 0);
        assert_eq!(jit_v2_string_eq(std::ptr::null_mut(), std::ptr::null_mut()), 1);
        jit_v2_string_release(a);
    }

    #[test]
    fn test_retain_release() {
        let ptr = jit_v2_string_alloc(b"rc".as_ptr(), 2);
        assert!(!ptr.is_null());

        // Retain bumps refcount to 2.
        jit_v2_string_retain(ptr);
        unsafe {
            assert_eq!(as_obj(ptr).refcount.load(Ordering::Relaxed), 2);
        }

        // First release drops to 1 — no dealloc.
        jit_v2_string_release(ptr);
        unsafe {
            assert_eq!(as_obj(ptr).refcount.load(Ordering::Relaxed), 1);
        }

        // Second release drops to 0 — deallocates.
        jit_v2_string_release(ptr);
        // ptr is now dangling; we cannot read it.
    }

    #[test]
    fn test_null_safety() {
        // All functions should handle null gracefully.
        assert_eq!(jit_v2_string_len(std::ptr::null_mut()), 0);
        assert_eq!(jit_v2_string_data(std::ptr::null_mut()), std::ptr::null());
        jit_v2_string_print(std::ptr::null_mut()); // should not crash
        jit_v2_string_retain(std::ptr::null_mut()); // no-op
        jit_v2_string_release(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn test_concat_null_inputs() {
        // concat(null, null) should produce an empty string.
        let c = jit_v2_string_concat(std::ptr::null_mut(), std::ptr::null_mut());
        assert!(!c.is_null());
        assert_eq!(jit_v2_string_len(c), 0);
        jit_v2_string_release(c);
    }

    #[test]
    fn test_unicode() {
        let src = "hello 🌍";
        let bytes = src.as_bytes();
        let ptr = jit_v2_string_alloc(bytes.as_ptr(), bytes.len() as u32);
        assert!(!ptr.is_null());
        assert_eq!(jit_v2_string_len(ptr) as usize, bytes.len());

        let data = jit_v2_string_data(ptr);
        let slice = unsafe { std::slice::from_raw_parts(data, bytes.len()) };
        assert_eq!(std::str::from_utf8(slice).unwrap(), src);

        jit_v2_string_release(ptr);
    }
}
