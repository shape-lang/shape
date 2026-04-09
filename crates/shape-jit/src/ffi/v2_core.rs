//! v2 Core FFI functions — native typed replacements for NaN-boxed functions.
//!
//! These functions accept raw native types (pointers, i64, f64, u8) instead of
//! NaN-boxed u64 values. They are the v2 replacements for:
//! - `jit_print(value_bits: u64)` -> type-specific print functions
//! - arc_retain/arc_release -> pointer-based retain/release
//!
//! Part of the v2 NaN-boxing removal plan (Step 4).

use super::typed_object::TypedObject;
use super::conversion::format_nan_boxed;

// ============================================================================
// Retain / Release — operate on raw heap pointers, no NaN-boxing
// ============================================================================

/// Increment the reference count on a TypedObject (legacy v1 TypedObject path).
///
/// Note: `#[no_mangle]` removed — the symbol names `jit_v2_retain` and
/// `jit_v2_release` are owned by `v2/mod.rs` which uses HeapHeader-based
/// refcounting. This TypedObject-based version is kept as a Rust-only helper.
///
/// # Safety
/// `ptr` must point to a valid, live `TypedObject` or be null (no-op on null).
#[allow(dead_code)]
pub extern "C" fn jit_v2_retain_typed_object(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    let obj = ptr as *mut TypedObject;
    unsafe {
        (*obj).inc_ref();
    }
}

/// Decrement the reference count on a TypedObject (legacy v1 TypedObject path).
/// Does NOT free the object — the caller (or a separate destructor) handles deallocation.
///
/// # Safety
/// `ptr` must point to a valid, live `TypedObject` or be null (no-op on null).
#[allow(dead_code)]
pub extern "C" fn jit_v2_release_typed_object(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    let obj = ptr as *mut TypedObject;
    unsafe {
        (*obj).dec_ref();
    }
}

// ============================================================================
// Typed Print — no NaN-boxing tag dispatch
// ============================================================================

/// v2 type tags for print dispatch (matches StorageHint encoding).
/// These are passed as a `u8` from the JIT compiler which knows the type at compile time.
pub const V2_TYPE_TAG_INT: u8 = 1;
pub const V2_TYPE_TAG_NUMBER: u8 = 2;
pub const V2_TYPE_TAG_BOOL: u8 = 3;
pub const V2_TYPE_TAG_STRING: u8 = 4;
pub const V2_TYPE_TAG_NANBOXED: u8 = 0; // Fallback: still NaN-boxed (transitional)

/// Print an integer value.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_print_int(value: i64) {
    println!("{}", value);
}

/// Print a floating-point number.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_print_number(value: f64) {
    if value.is_finite() && value == value.trunc() && value.abs() < 1e15 {
        println!("{}", value as i64);
    } else {
        println!("{}", value);
    }
}

/// Print a boolean value.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_print_bool(value: u8) {
    if value != 0 {
        println!("true");
    } else {
        println!("false");
    }
}

/// Print a heap-allocated string via raw pointer.
///
/// # Safety
/// `ptr` must point to a valid `JitAlloc<String>` or be null.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_print_string(ptr: *const u8) {
    if ptr.is_null() {
        println!("null");
        return;
    }
    // The pointer is to a JitAlloc<String>: kind(u16) + pad(6) + String
    // String data is at offset 8 (JIT_ALLOC_DATA_OFFSET)
    let string_ref = unsafe { &*(ptr.add(8) as *const String) };
    println!("{}", string_ref);
}

/// Print a value with an explicit type tag (v2 native-typed variant).
///
/// This is the v2-native transitional print function: the compiler passes a type_tag
/// so the FFI function doesn't need to inspect NaN-box bits to determine the type.
/// For type_tag == 0 (V2_TYPE_TAG_NANBOXED), falls back to legacy NaN-box formatting.
///
/// Note: `#[no_mangle]` is intentionally omitted — the symbol name `jit_v2_print_typed`
/// is owned by the legacy v2_typed module which uses `i8` tags. This function uses `u8`
/// tags and will replace the legacy version once the v2 transition is complete.
/// In the meantime, use the type-specific variants (`jit_v2_print_int`, etc.) from
/// JIT-compiled code for zero-overhead typed printing.
pub extern "C" fn jit_v2_print_typed_native(value_bits: u64, type_tag: u8) {
    match type_tag {
        V2_TYPE_TAG_INT => {
            println!("{}", value_bits as i64);
        }
        V2_TYPE_TAG_NUMBER => {
            let f = f64::from_bits(value_bits);
            if f.is_finite() && f == f.trunc() && f.abs() < 1e15 {
                println!("{}", f as i64);
            } else {
                println!("{}", f);
            }
        }
        V2_TYPE_TAG_BOOL => {
            if value_bits != 0 {
                println!("true");
            } else {
                println!("false");
            }
        }
        V2_TYPE_TAG_STRING => {
            // value_bits is a raw pointer to JitAlloc<String>
            let ptr = value_bits as *const u8;
            if ptr.is_null() {
                println!("null");
            } else {
                let string_ref = unsafe { &*(ptr.add(8) as *const String) };
                println!("{}", string_ref);
            }
        }
        _ => {
            // Fallback: NaN-boxed legacy path
            println!("{}", format_nan_boxed(value_bits));
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v2_retain_release_null_is_noop() {
        // Should not crash
        jit_v2_retain_typed_object(std::ptr::null());
        jit_v2_release_typed_object(std::ptr::null());
    }

    #[test]
    fn test_v2_print_typed_int() {
        // Just verify it doesn't crash
        jit_v2_print_typed_native(42u64, V2_TYPE_TAG_INT);
    }

    #[test]
    fn test_v2_print_typed_number() {
        let f: f64 = 3.14;
        jit_v2_print_typed_native(f.to_bits(), V2_TYPE_TAG_NUMBER);
    }

    #[test]
    fn test_v2_print_typed_bool() {
        jit_v2_print_typed_native(1, V2_TYPE_TAG_BOOL);
        jit_v2_print_typed_native(0, V2_TYPE_TAG_BOOL);
    }
}
