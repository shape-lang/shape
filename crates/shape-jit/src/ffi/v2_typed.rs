//! v2 Typed FFI Functions for JIT
//!
//! Native-typed FFI functions that bypass NaN-boxing for known element/field types.
//! These accept raw f64/i64/i32 values directly, avoiding the box/unbox overhead
//! that the v1 NaN-boxed FFI path requires.
//!
//! The JIT compiler emits calls to these functions when the element or field type
//! is statically known at compile time. For unknown types, the NaN-boxed fallback
//! path remains.

use crate::jit_array::JitArray;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

// ============================================================================
// v2 Array Operations (f64-typed)
// ============================================================================

/// Allocate a new empty array for f64 elements with optional pre-allocated capacity.
///
/// Returns a NaN-boxed HK_ARRAY value. The array is element-kind tagged as Float64
/// so subsequent typed pushes can bypass kind inference.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_alloc_f64(capacity: i64) -> u64 {
    let cap = capacity.max(0) as usize;
    let cap = cap.min(100_000_000);
    let mut arr = JitArray::with_capacity(cap);
    arr.element_kind = crate::jit_array::ArrayElementKind::Float64 as u8;
    jit_box(HK_ARRAY, arr)
}

/// Push a raw f64 value onto an array, NaN-boxing it for storage.
///
/// The caller passes a raw f64 — this function boxes it and appends to the array.
/// Returns the (unchanged) array bits.
///
/// # Safety
/// `arr_bits` must be a valid NaN-boxed HK_ARRAY value.
///
/// Note: `#[no_mangle]` removed — the symbol name `jit_v2_array_push_f64`
/// is owned by the typed v2 module (`crates/shape-jit/src/ffi/v2/mod.rs`),
/// which uses `*mut TypedArray<f64>` instead of NaN-boxed bits. This legacy
/// thunk is now dead code but is kept here as a Rust-only helper for any
/// future legacy callers.
#[allow(dead_code)]
pub extern "C" fn jit_v2_array_push_f64_legacy(arr_bits: u64, val: f64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return arr_bits;
    }
    let arr = unsafe { jit_unbox_mut::<JitArray>(arr_bits) };
    // Store as NaN-boxed f64 (which is just the raw f64 bits for non-NaN values,
    // but we use box_number for correctness with NaN/infinity)
    arr.push(box_number(val));
    arr_bits
}

/// Get a raw f64 element from an array at the given index.
///
/// Returns the element as a raw f64. If out of bounds, returns NaN.
/// The caller knows the element type is f64, so we unbox directly.
///
/// # Safety
/// `arr_bits` must be a valid NaN-boxed HK_ARRAY value.
///
/// Note: `#[no_mangle]` removed — the symbol name `jit_v2_array_get_f64`
/// is owned by the typed v2 module (`crates/shape-jit/src/ffi/v2/mod.rs`),
/// which uses `*const TypedArray<f64>` instead of NaN-boxed bits.
#[allow(dead_code)]
pub extern "C" fn jit_v2_array_get_f64_legacy(arr_bits: u64, idx: i64) -> f64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return f64::NAN;
    }
    let arr = unsafe { jit_unbox::<JitArray>(arr_bits) };
    let actual_index = if idx < 0 {
        let len = arr.len() as i64;
        (len + idx) as usize
    } else {
        idx as usize
    };
    match arr.get(actual_index) {
        Some(&bits) => {
            if is_number(bits) {
                unbox_number(bits)
            } else {
                f64::NAN
            }
        }
        None => f64::NAN,
    }
}

// ============================================================================
// v2 Struct Operations (f64-typed)
// ============================================================================

/// Allocate a typed struct (TypedObject) with the given total data size in bytes.
///
/// Returns a NaN-boxed HK_TYPED_OBJECT value. The caller provides the schema_id
/// and data_size; this function allocates the underlying buffer.
///
/// This is a thin wrapper around jit_typed_object_alloc, but with a simpler
/// signature for the v2 typed path.
///
/// Note: `#[no_mangle]` removed — the symbol name `jit_v2_struct_alloc` is now
/// owned by `v2_struct.rs` which provides a proper raw-pointer implementation
/// without NaN-boxing. This legacy version is kept for any callers that still
/// operate in the NaN-boxed domain.
#[allow(dead_code)]
pub extern "C" fn jit_v2_struct_alloc_nanboxed(schema_id: i32, total_size: i64) -> u64 {
    // Delegate to existing typed object allocation
    crate::ffi::typed_object::jit_typed_object_alloc(schema_id as u32, total_size as u64)
}

/// Read a f64 field from a typed struct at the given byte offset.
///
/// The caller has statically determined that the field at `offset` is f64.
/// We load the 8 bytes at that offset and interpret as NaN-boxed f64,
/// then unbox to raw f64.
///
/// # Safety
/// `ptr_bits` must be a valid NaN-boxed HK_TYPED_OBJECT value.
/// `offset` must be a valid field byte offset within the object.
///
/// Note: `#[no_mangle]` removed — the symbol name `jit_v2_struct_get_f64` is now
/// owned by `v2_struct.rs` which uses raw pointer access without NaN-boxing.
#[allow(dead_code)]
pub extern "C" fn jit_v2_struct_get_f64_nanboxed(ptr_bits: u64, offset: i32) -> f64 {
    if !is_heap_kind(ptr_bits, HK_TYPED_OBJECT) {
        return f64::NAN;
    }
    unsafe {
        let alloc_ptr = (ptr_bits & PAYLOAD_MASK) as *const u8;
        // JitAlloc header is 8 bytes, then data pointer at offset 8
        let data_ptr = *(alloc_ptr.add(JIT_ALLOC_DATA_OFFSET) as *const *const u8);
        // TypedObject header is 8 bytes, then field data
        let field_addr = data_ptr.add(8 + offset as usize);
        let bits = *(field_addr as *const u64);
        if is_number(bits) {
            unbox_number(bits)
        } else {
            f64::NAN
        }
    }
}

/// Write a raw f64 value to a typed struct field at the given byte offset.
///
/// NaN-boxes the f64 and stores it at the field offset.
///
/// # Safety
/// `ptr_bits` must be a valid NaN-boxed HK_TYPED_OBJECT value.
/// `offset` must be a valid field byte offset within the object.
///
/// Note: `#[no_mangle]` removed — the symbol name `jit_v2_struct_set_f64` is now
/// owned by `v2_struct.rs` which uses raw pointer access without NaN-boxing.
#[allow(dead_code)]
pub extern "C" fn jit_v2_struct_set_f64_nanboxed(ptr_bits: u64, offset: i32, val: f64) {
    if !is_heap_kind(ptr_bits, HK_TYPED_OBJECT) {
        return;
    }
    unsafe {
        let alloc_ptr = (ptr_bits & PAYLOAD_MASK) as *const u8;
        let data_ptr = *(alloc_ptr.add(JIT_ALLOC_DATA_OFFSET) as *const *mut u8);
        let field_addr = data_ptr.add(8 + offset as usize);
        let boxed = box_number(val);
        *(field_addr as *mut u64) = boxed;
    }
}

// ============================================================================
// v2 Math Operations (native f64)
// ============================================================================

/// Compute base^exp using native f64 pow, returning raw f64.
///
/// Avoids NaN-boxing overhead for the common case where both operands
/// are known to be f64 at compile time.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_pow_f64(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

// ============================================================================
// v2 Refcount Operations
// ============================================================================

/// Increment the reference count on a heap-allocated value.
///
/// Currently a no-op placeholder — JIT values use Box-based ownership.
/// Note: `#[no_mangle]` removed — the typed v2 module owns the symbol
/// name `jit_v2_retain`. This is the legacy NaN-boxed signature, kept as
/// a Rust-only helper.
#[allow(dead_code)]
pub extern "C" fn jit_v2_retain_legacy(_ptr_bits: u64) {
    // No-op
}

/// Decrement the reference count on a heap-allocated value, freeing if zero.
///
/// Note: `#[no_mangle]` removed — the typed v2 module owns the symbol
/// name `jit_v2_release`.
#[allow(dead_code)]
pub extern "C" fn jit_v2_release_legacy(_ptr_bits: u64) {
    // No-op: current JIT allocation model leaks all heap values.
    // When refcounting is implemented, this will decrement and
    // potentially free.
}

// ============================================================================
// v2 Print (typed)
// ============================================================================

/// Type tags for v2 typed print.
/// Must match the values used by the JIT compiler when emitting type_tag constants.
pub const V2_TYPE_TAG_F64: i8 = 1;
pub const V2_TYPE_TAG_I64: i8 = 2;
pub const V2_TYPE_TAG_BOOL: i8 = 3;
pub const V2_TYPE_TAG_STRING: i8 = 4;
pub const V2_TYPE_TAG_NANBOXED: i8 = 0;

/// Print a value with a known type tag, avoiding NaN-box format detection.
///
/// When `type_tag` identifies a known type, the value bits are interpreted
/// directly as that type. When `type_tag` is 0 (Dynamic), falls back to
/// the standard NaN-boxed format detection.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_print_typed(value_bits: u64, type_tag: i8) {
    match type_tag {
        V2_TYPE_TAG_F64 => {
            let val = f64::from_bits(value_bits);
            if val == (val as i64 as f64) && val.is_finite() {
                // Print integers without decimal point
                println!("{}", val as i64);
            } else {
                println!("{}", val);
            }
        }
        V2_TYPE_TAG_I64 => {
            println!("{}", value_bits as i64);
        }
        V2_TYPE_TAG_BOOL => {
            println!("{}", if value_bits != 0 { "true" } else { "false" });
        }
        V2_TYPE_TAG_STRING => {
            if is_heap_kind(value_bits, HK_STRING) {
                let s = unsafe { jit_unbox::<String>(value_bits) };
                println!("{}", s);
            } else {
                println!("[invalid string]");
            }
        }
        _ => {
            // Fallback: use standard ValueWord formatting
            println!("{}", crate::ffi::conversion::format_value_word(value_bits));
        }
    }
}
