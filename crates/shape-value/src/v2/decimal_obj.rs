//! Refcounted, repr(C) Decimal carrier for v2 runtime.
//!
//! ## Memory layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader — refcount at offset 0)
//!   8      16   value (rust_decimal::Decimal — inline payload)
//! ```
//!
//! Mirrors the `StringObj` precedent — `#[repr(C)]` 24-byte struct with
//! `HeapHeader` at offset 0. `rust_decimal::Decimal` is `Copy + 16-byte`
//! (4-byte flags + 12-byte mantissa per the crate's internal layout), so
//! the payload lives inline with no nested allocation.
//!
//! ## Authority
//!
//! Per ADR-006 §2.7.24 Q25.A SUPERSEDED + R20 S2-prime audit deliverable
//! (d) §4.1.D.1. `DecimalObj` is the per-element carrier for
//! `TypedArray<*const DecimalObj>`, the v2-raw replacement for
//! `TypedArrayData::Decimal(Arc<TypedBuffer<Arc<Decimal>>>)`.
//!
//! ## Refcount discipline
//!
//! The header refcount initializes to 1 on `new`. `HeapElement::release_elem`
//! decrements via `v2_release`; on return-true, `Self::drop` deallocates the
//! struct. No nested buffer (Decimal payload is inline), so `drop` only frees
//! `Layout::new::<Self>()`.

use super::heap_header::{HeapHeader, HEAP_KIND_V2_DECIMAL};
use rust_decimal::Decimal;

/// Refcounted, repr(C) Decimal carrier for v2 runtime.
/// Total: 24 bytes (header 8 + value 16).
#[repr(C)]
pub struct DecimalObj {
    pub header: HeapHeader,
    pub value: Decimal,
}

impl DecimalObj {
    /// Allocate a new DecimalObj wrapping the given Decimal value.
    /// Returns a raw pointer with refcount initialized to 1.
    pub fn new(value: Decimal) -> *mut Self {
        let layout = std::alloc::Layout::new::<Self>();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for DecimalObj");
        unsafe {
            (*ptr).header = HeapHeader::new(HEAP_KIND_V2_DECIMAL);
            (*ptr).value = value;
        }
        ptr
    }

    /// Get the Decimal value.
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `DecimalObj`.
    pub unsafe fn value(ptr: *const Self) -> Decimal {
        unsafe { (*ptr).value }
    }

    /// Free the DecimalObj.
    ///
    /// # Safety
    /// `ptr` must point to a valid `DecimalObj` with no remaining references.
    /// Must not be called more than once on the same pointer.
    pub unsafe fn drop(ptr: *mut Self) {
        // No nested allocation; just dealloc the struct.
        let layout = std::alloc::Layout::new::<Self>();
        unsafe { std::alloc::dealloc(ptr as *mut u8, layout) };
    }

    /// Byte offset constants for JIT codegen.
    pub const OFFSET_VALUE: usize = 8;
}

// Compile-time size + alignment assertions.
// `rust_decimal::Decimal` is 16 bytes with 4-byte alignment (4-byte flags +
// 12-byte mantissa); `HeapHeader` is 8 bytes with 4-byte alignment
// (AtomicU32 refcount + u16 kind + u8 flags + u8 _pad). Combined struct is
// 24 bytes with 4-byte alignment.
const _: () = {
    assert!(std::mem::size_of::<DecimalObj>() == 24);
    assert!(std::mem::align_of::<DecimalObj>() == 4);
};

// HeapElement impl per ADR-006 §2.7.24 Q25.A SUPERSEDED + R20 S2-prime
// audit deliverable (b) §4.1.B decision. Constrains `DecimalObj` to the
// HeapHeader-at-offset-0 v2-raw element-carrier contract; enables
// `TypedArray<*const DecimalObj>::drop_array_heap` per-T release dispatch
// via compile-time monomorphization (no runtime NativeKind probe).
unsafe impl super::heap_element::HeapElement for DecimalObj {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { super::refcount::v2_release(&(*ptr).header) } {
            unsafe { Self::drop(ptr as *mut Self) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::FromPrimitive;

    #[test]
    fn test_size_of_decimal_obj() {
        assert_eq!(std::mem::size_of::<DecimalObj>(), 24);
        assert_eq!(std::mem::align_of::<DecimalObj>(), 4);
    }

    #[test]
    fn test_create_and_read_decimal() {
        unsafe {
            let d = Decimal::from_f64(3.14).unwrap();
            let ptr = DecimalObj::new(d);
            assert_eq!(DecimalObj::value(ptr), d);
            assert_eq!((*ptr).header.kind(), HEAP_KIND_V2_DECIMAL);
            assert_eq!((*ptr).header.get_refcount(), 1);
            DecimalObj::drop(ptr);
        }
    }

    #[test]
    fn test_create_zero_decimal() {
        unsafe {
            let d = Decimal::ZERO;
            let ptr = DecimalObj::new(d);
            assert_eq!(DecimalObj::value(ptr), Decimal::ZERO);
            DecimalObj::drop(ptr);
        }
    }

    #[test]
    fn test_create_max_decimal() {
        unsafe {
            let d = Decimal::MAX;
            let ptr = DecimalObj::new(d);
            assert_eq!(DecimalObj::value(ptr), Decimal::MAX);
            DecimalObj::drop(ptr);
        }
    }

    #[test]
    fn test_drop_does_not_leak() {
        // Create and drop many DecimalObjs — under Miri/valgrind this would
        // catch leaks.
        unsafe {
            for i in 0..200 {
                let d = Decimal::from_f64(i as f64 * 0.123).unwrap_or(Decimal::ZERO);
                let ptr = DecimalObj::new(d);
                DecimalObj::drop(ptr);
            }
        }
    }

    #[test]
    fn test_field_offsets() {
        unsafe {
            let ptr = DecimalObj::new(Decimal::ONE);
            let base = ptr as usize;
            let value_offset = &(*ptr).value as *const _ as usize - base;
            assert_eq!(value_offset, DecimalObj::OFFSET_VALUE, "value must be at offset 8");
            DecimalObj::drop(ptr);
        }
    }

    #[test]
    fn test_refcount_starts_at_one() {
        unsafe {
            let ptr = DecimalObj::new(Decimal::ONE);
            assert_eq!((*ptr).header.get_refcount(), 1);
            DecimalObj::drop(ptr);
        }
    }

    #[test]
    fn test_refcount_retain_release() {
        use crate::v2::refcount::{v2_get_refcount, v2_release, v2_retain};
        unsafe {
            let ptr = DecimalObj::new(Decimal::ONE);
            let header = &(*ptr).header as *const HeapHeader;

            assert_eq!(v2_get_refcount(header), 1);
            v2_retain(header);
            assert_eq!(v2_get_refcount(header), 2);
            assert!(!v2_release(header)); // 2 -> 1
            assert_eq!(v2_get_refcount(header), 1);

            // Don't release to zero here — use drop for cleanup.
            DecimalObj::drop(ptr);
        }
    }

    #[test]
    fn test_heap_element_release_elem_to_zero() {
        use crate::v2::heap_element::HeapElement;
        unsafe {
            // Allocate; release_elem from refcount 1 → 0 should deallocate.
            let ptr = DecimalObj::new(Decimal::ONE);
            DecimalObj::release_elem(ptr);
            // ptr is dangling; we cannot dereference further. The valgrind /
            // Miri pass confirms no leak.
        }
    }

    #[test]
    fn test_heap_element_release_elem_held_share() {
        use crate::v2::heap_element::HeapElement;
        use crate::v2::refcount::{v2_get_refcount, v2_retain};
        unsafe {
            let ptr = DecimalObj::new(Decimal::ONE);
            let header = &(*ptr).header as *const HeapHeader;

            v2_retain(header); // refcount = 2
            DecimalObj::release_elem(ptr); // refcount = 1 (does not deallocate)
            assert_eq!(v2_get_refcount(header), 1);

            // Clean up the held share.
            DecimalObj::drop(ptr);
        }
    }
}
