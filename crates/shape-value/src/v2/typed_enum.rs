//! Heap allocation and field access helpers for v2 typed enums.
//!
//! A typed enum heap object has the layout:
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   HeapHeader
//!   8       1   tag (variant discriminant)
//!   9       7   padding
//!  16     ...   payload
//! ```
//!
//! The tag at offset 8 selects which `VariantLayout` describes the payload.
//! Field offsets within the payload are relative to offset 16.
//!
//! All operations here are unsafe `*mut u8` based — the compiler emits direct
//! loads/stores against compile-time-known offsets, so this module exposes raw
//! primitives rather than a typed Rust API.

use std::ptr;

use super::enum_layout::{ENUM_PAYLOAD_OFFSET, ENUM_TAG_OFFSET, EnumLayout};
use super::heap_header::{HEAP_KIND_V2_TYPED_ENUM, HeapHeader};
use super::struct_layout::FieldKind;

/// Allocate a fresh typed enum heap object for the given layout.
///
/// Writes a `HeapHeader` (refcount = 1, kind = `HEAP_KIND_V2_TYPED_ENUM`) at
/// offset 0. The tag byte and payload are left uninitialized — callers must
/// `write_tag` and write any payload fields before reading.
///
/// Returns a raw pointer to `layout.total_size` bytes; caller is responsible
/// for refcount management and eventual `dealloc_typed_enum`.
pub fn alloc_typed_enum(layout: &EnumLayout) -> *mut u8 {
    let ptr = layout.alloc();
    unsafe {
        ptr::write(ptr as *mut HeapHeader, HeapHeader::new(HEAP_KIND_V2_TYPED_ENUM));
    }
    ptr
}

/// Deallocate a previously allocated typed enum object.
///
/// # Safety
/// `ptr` must have been produced by `alloc_typed_enum(layout)` (or
/// `layout.alloc()`), with the same `layout`, and must not be aliased.
pub unsafe fn dealloc_typed_enum(layout: &EnumLayout, ptr: *mut u8) {
    unsafe { layout.dealloc(ptr) };
}

/// Read the tag byte from a typed enum object.
///
/// # Safety
/// `ptr` must point to a live, initialized typed enum heap object.
#[inline]
pub unsafe fn read_tag(ptr: *const u8) -> u8 {
    unsafe { *ptr.add(ENUM_TAG_OFFSET) }
}

/// Write the tag byte of a typed enum object.
///
/// # Safety
/// `ptr` must point to a writable typed enum heap object of sufficient size.
#[inline]
pub unsafe fn write_tag(ptr: *mut u8, tag: u8) {
    unsafe {
        *ptr.add(ENUM_TAG_OFFSET) = tag;
    }
}

/// Get a raw pointer to the start of the payload area (offset 16).
///
/// # Safety
/// `ptr` must point to a typed enum heap object of at least
/// `ENUM_PAYLOAD_OFFSET` bytes.
#[inline]
pub unsafe fn payload_ptr(ptr: *mut u8) -> *mut u8 {
    unsafe { ptr.add(ENUM_PAYLOAD_OFFSET) }
}

/// Read a field at `field_offset` (within the payload area) as the given
/// `FieldKind`, returning the bits as a `u64`.
///
/// Numeric values are zero-extended to `u64`. `Bool` returns 0 or 1. `Ptr`
/// returns the raw pointer bits. `F64` returns the IEEE-754 bit pattern (use
/// `f64::from_bits`).
///
/// # Safety
/// `ptr` must point to a live typed enum object whose current variant has a
/// field of the given kind at the given offset within its payload.
#[inline]
pub unsafe fn read_payload_field(ptr: *const u8, field_offset: usize, kind: FieldKind) -> u64 {
    unsafe {
        let field_ptr = ptr.add(ENUM_PAYLOAD_OFFSET + field_offset);
        match kind {
            FieldKind::F64 => {
                let v = ptr::read_unaligned(field_ptr as *const f64);
                v.to_bits()
            }
            FieldKind::I64 => {
                let v = ptr::read_unaligned(field_ptr as *const i64);
                v as u64
            }
            FieldKind::U64 | FieldKind::Ptr => ptr::read_unaligned(field_ptr as *const u64),
            FieldKind::I32 => {
                let v = ptr::read_unaligned(field_ptr as *const i32);
                v as i64 as u64
            }
            FieldKind::U32 => {
                let v = ptr::read_unaligned(field_ptr as *const u32);
                v as u64
            }
            FieldKind::I16 => {
                let v = ptr::read_unaligned(field_ptr as *const i16);
                v as i64 as u64
            }
            FieldKind::U16 => {
                let v = ptr::read_unaligned(field_ptr as *const u16);
                v as u64
            }
            FieldKind::I8 => {
                let v = ptr::read_unaligned(field_ptr as *const i8);
                v as i64 as u64
            }
            FieldKind::U8 => {
                let v = ptr::read_unaligned(field_ptr as *const u8);
                v as u64
            }
            FieldKind::Bool => {
                let v = ptr::read_unaligned(field_ptr as *const u8);
                v as u64
            }
        }
    }
}

/// Write a field at `field_offset` (within the payload area) using the given
/// `FieldKind`. The `bits` parameter is reinterpreted according to `kind`:
/// for `F64`, the bits are written as an `f64` via `f64::from_bits`; for
/// signed integers, the low bits of `bits` are truncated; etc.
///
/// # Safety
/// `ptr` must point to a writable typed enum object whose current variant has
/// a field of the given kind at the given offset within its payload.
#[inline]
pub unsafe fn write_payload_field(
    ptr: *mut u8,
    field_offset: usize,
    kind: FieldKind,
    bits: u64,
) {
    unsafe {
        let field_ptr = ptr.add(ENUM_PAYLOAD_OFFSET + field_offset);
        match kind {
            FieldKind::F64 => {
                ptr::write_unaligned(field_ptr as *mut f64, f64::from_bits(bits));
            }
            FieldKind::I64 => {
                ptr::write_unaligned(field_ptr as *mut i64, bits as i64);
            }
            FieldKind::U64 | FieldKind::Ptr => {
                ptr::write_unaligned(field_ptr as *mut u64, bits);
            }
            FieldKind::I32 => {
                ptr::write_unaligned(field_ptr as *mut i32, bits as i32);
            }
            FieldKind::U32 => {
                ptr::write_unaligned(field_ptr as *mut u32, bits as u32);
            }
            FieldKind::I16 => {
                ptr::write_unaligned(field_ptr as *mut i16, bits as i16);
            }
            FieldKind::U16 => {
                ptr::write_unaligned(field_ptr as *mut u16, bits as u16);
            }
            FieldKind::I8 => {
                ptr::write_unaligned(field_ptr as *mut i8, bits as i8);
            }
            FieldKind::U8 => {
                ptr::write_unaligned(field_ptr as *mut u8, bits as u8);
            }
            FieldKind::Bool => {
                ptr::write_unaligned(field_ptr as *mut u8, (bits != 0) as u8);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::enum_layout::compute_enum_layout;
    use super::*;

    #[test]
    fn test_alloc_writes_header() {
        let variants = vec![
            ("Circle".to_string(), vec![FieldKind::F64]),
            ("Rectangle".to_string(), vec![FieldKind::F64, FieldKind::F64]),
        ];
        let layout = compute_enum_layout("Shape", &variants);

        let ptr = alloc_typed_enum(&layout);
        assert!(!ptr.is_null());

        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.kind(), HEAP_KIND_V2_TYPED_ENUM);
            assert_eq!(header.get_refcount(), 1);

            dealloc_typed_enum(&layout, ptr);
        }
    }

    #[test]
    fn test_write_then_read_tag() {
        let variants = vec![
            ("A".to_string(), vec![]),
            ("B".to_string(), vec![]),
            ("C".to_string(), vec![]),
        ];
        let layout = compute_enum_layout("E", &variants);

        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 0);
            assert_eq!(read_tag(ptr), 0);

            write_tag(ptr, 1);
            assert_eq!(read_tag(ptr), 1);

            write_tag(ptr, 2);
            assert_eq!(read_tag(ptr), 2);

            // Header is unchanged after tag writes.
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.kind(), HEAP_KIND_V2_TYPED_ENUM);
            assert_eq!(header.get_refcount(), 1);

            dealloc_typed_enum(&layout, ptr);
        }
    }

    #[test]
    fn test_payload_ptr_offset() {
        let variants = vec![("V".to_string(), vec![FieldKind::F64])];
        let layout = compute_enum_layout("E", &variants);
        let ptr = alloc_typed_enum(&layout);

        unsafe {
            let pp = payload_ptr(ptr);
            let diff = pp as usize - ptr as usize;
            assert_eq!(diff, ENUM_PAYLOAD_OFFSET);
            assert_eq!(diff, 16);
            dealloc_typed_enum(&layout, ptr);
        }
    }

    #[test]
    fn test_read_write_f64_field() {
        // enum Shape { Circle(f64), Rectangle(f64, f64) }
        let variants = vec![
            ("Circle".to_string(), vec![FieldKind::F64]),
            ("Rectangle".to_string(), vec![FieldKind::F64, FieldKind::F64]),
        ];
        let layout = compute_enum_layout("Shape", &variants);

        // Build a Circle(3.14)
        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 0);
            let circle = layout.variant_layout(0).unwrap();
            assert_eq!(circle.field_offsets[0], 0);
            write_payload_field(ptr, 0, FieldKind::F64, 3.14_f64.to_bits());

            assert_eq!(read_tag(ptr), 0);
            let bits = read_payload_field(ptr, 0, FieldKind::F64);
            assert_eq!(f64::from_bits(bits), 3.14);

            dealloc_typed_enum(&layout, ptr);
        }

        // Build a Rectangle(4.0, 5.0)
        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 1);
            let rect = layout.variant_layout(1).unwrap();
            assert_eq!(rect.field_offsets[0], 0);
            assert_eq!(rect.field_offsets[1], 8);
            write_payload_field(ptr, 0, FieldKind::F64, 4.0_f64.to_bits());
            write_payload_field(ptr, 8, FieldKind::F64, 5.0_f64.to_bits());

            assert_eq!(read_tag(ptr), 1);
            let w = f64::from_bits(read_payload_field(ptr, 0, FieldKind::F64));
            let h = f64::from_bits(read_payload_field(ptr, 8, FieldKind::F64));
            assert_eq!(w, 4.0);
            assert_eq!(h, 5.0);

            dealloc_typed_enum(&layout, ptr);
        }
    }

    #[test]
    fn test_read_write_int_fields() {
        // Variant with i32, i64, i8 — exercise the integer code paths.
        let variants = vec![(
            "V".to_string(),
            vec![FieldKind::I32, FieldKind::I64, FieldKind::I8],
        )];
        let layout = compute_enum_layout("E", &variants);
        let v = layout.variant_layout(0).unwrap();
        // i32 at 0, i64 at 8 (padded from 4), i8 at 16
        assert_eq!(v.field_offsets, vec![0, 8, 16]);

        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 0);
            write_payload_field(ptr, 0, FieldKind::I32, (-42_i32) as u32 as u64);
            write_payload_field(ptr, 8, FieldKind::I64, i64::MAX as u64);
            write_payload_field(ptr, 16, FieldKind::I8, (-7_i8) as u8 as u64);

            let a = read_payload_field(ptr, 0, FieldKind::I32) as i32;
            let b = read_payload_field(ptr, 8, FieldKind::I64) as i64;
            let c = read_payload_field(ptr, 16, FieldKind::I8) as i8;

            assert_eq!(a, -42);
            assert_eq!(b, i64::MAX);
            assert_eq!(c, -7);

            dealloc_typed_enum(&layout, ptr);
        }
    }

    #[test]
    fn test_read_write_bool_and_ptr() {
        let variants = vec![(
            "V".to_string(),
            vec![FieldKind::Bool, FieldKind::Ptr],
        )];
        let layout = compute_enum_layout("E", &variants);
        // Bool at 0, Ptr at 8 (padded from 1)
        let v = layout.variant_layout(0).unwrap();
        assert_eq!(v.field_offsets, vec![0, 8]);

        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 0);
            write_payload_field(ptr, 0, FieldKind::Bool, 1);
            let fake_ptr_bits: u64 = 0xDEAD_BEEF_CAFE_F00D;
            write_payload_field(ptr, 8, FieldKind::Ptr, fake_ptr_bits);

            assert_eq!(read_payload_field(ptr, 0, FieldKind::Bool), 1);
            assert_eq!(read_payload_field(ptr, 8, FieldKind::Ptr), fake_ptr_bits);

            // Bool truncates non-zero values to 1.
            write_payload_field(ptr, 0, FieldKind::Bool, 0xFF_FF_FF_FF);
            assert_eq!(read_payload_field(ptr, 0, FieldKind::Bool), 1);
            write_payload_field(ptr, 0, FieldKind::Bool, 0);
            assert_eq!(read_payload_field(ptr, 0, FieldKind::Bool), 0);

            dealloc_typed_enum(&layout, ptr);
        }
    }

    #[test]
    fn test_alloc_zero_payload() {
        // Unit variants — no payload writes.
        let variants = vec![
            ("Red".to_string(), vec![]),
            ("Green".to_string(), vec![]),
            ("Blue".to_string(), vec![]),
        ];
        let layout = compute_enum_layout("Color", &variants);

        let ptr = alloc_typed_enum(&layout);
        unsafe {
            write_tag(ptr, 2);
            assert_eq!(read_tag(ptr), 2);
            dealloc_typed_enum(&layout, ptr);
        }
    }
}
