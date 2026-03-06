// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 3 sites (in allocation.rs, merge_ops.rs)
//     box_typed_object — jit_typed_object_alloc, jit_new_typed_object,
//     jit_typed_merge_object, jit_typed_object_from_hashmap
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (TypedObject uses custom alloc/dealloc with ref counting — not jit_box.
//      Field values stored as raw u64 may contain JitAlloc pointers, but the
//      TypedObject itself is not wrapped in JitAlloc — it uses TAG_TYPED_OBJECT
//      encoding which directly embeds the pointer. Ref counting handles lifetime.)
//!
//! TypedObject - Fixed-layout objects for JIT optimization
//!
//! TypedObject provides O(1) field access by pre-computed byte offsets,
//! eliminating HashMap lookups entirely. This is the core optimization
//! for type-specialized JIT code.
//!
//! # Memory Layout
//!
//! ```text
//! +-------------+-------------+-------------+-------------+
//! | schema_id   | ref_count   | field[0]    | field[1]    | ...
//! | (4 bytes)   | (4 bytes)   | (8 bytes)   | (8 bytes)   |
//! +-------------+-------------+-------------+-------------+
//!              Header (8 bytes)              Data (field_count * 8 bytes)
//! ```
//!
//! # Performance
//!
//! | Operation          | HashMap | TypedObject | Speedup |
//! |--------------------|---------|-------------|---------|
//! | Single field get   | ~25ns   | ~2ns        | 12.5x   |
//! | Single field set   | ~30ns   | ~2ns        | 15x     |
//! | 5 fields batch     | ~125ns  | ~10ns       | 12.5x   |

mod allocation;
mod field_access;
mod merge_ops;

#[cfg(test)]
mod ffi_exports;

pub use allocation::*;
pub use field_access::*;
pub use merge_ops::*;

/// Header size in bytes (schema_id + ref_count)
pub const TYPED_OBJECT_HEADER_SIZE: usize = 8;

/// Memory alignment for TypedObject allocation.
/// 64-byte alignment for L1 cache line optimization and SIMD operations.
pub const TYPED_OBJECT_ALIGNMENT: usize = 64;

/// A typed object with fixed field layout for O(1) access.
///
/// This struct uses `#[repr(C)]` to ensure predictable memory layout.
/// Fields are stored inline after the header, accessed by byte offset.
#[repr(C)]
pub struct TypedObject {
    /// Schema ID for runtime type checking
    pub schema_id: u32,
    /// Reference count for garbage collection
    pub ref_count: u32,
    // Field data follows inline (not represented in struct)
    // Access via get_field/set_field with byte offset
}

impl TypedObject {
    /// Get a pointer to the field data area.
    #[inline]
    pub fn data_ptr(&self) -> *const u8 {
        unsafe { (self as *const Self as *const u8).add(TYPED_OBJECT_HEADER_SIZE) }
    }

    /// Get a mutable pointer to the field data area.
    #[inline]
    pub fn data_ptr_mut(&mut self) -> *mut u8 {
        unsafe { (self as *mut Self as *mut u8).add(TYPED_OBJECT_HEADER_SIZE) }
    }
}
