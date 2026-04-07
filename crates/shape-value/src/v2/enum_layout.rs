//! Enum layout computation for v2 typed enums.
//!
//! Each enum gets a compile-time layout: a tag byte identifies the variant
//! and a payload area large enough to hold the largest variant. The total
//! object layout is:
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   HeapHeader
//!   8       1   tag (u8 — variant discriminant)
//!   9       7   padding to 8-byte alignment
//!  16     ...   payload (max_payload_size bytes), rounded up to 8
//! ```
//!
//! Each variant has its own field layout within the payload area. Fields are
//! packed at their natural alignment starting from offset 0 within the payload.
//!
//! Match dispatch reads the tag byte at offset 8 (no string comparison) and
//! then reads variant fields at compile-time-known offsets within the payload.

use std::alloc::{Layout, alloc};

use super::heap_header::{HEAP_KIND_V2_TYPED_ENUM, HeapHeader};
use super::struct_layout::FieldKind;

/// Layout of a single variant of a typed enum.
///
/// Field offsets are relative to the start of the payload area (NOT the start
/// of the heap object). To compute the absolute byte offset within the heap
/// object, add the layout's payload base offset (typically 16: 8 header + 1 tag
/// + 7 padding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantLayout {
    /// Variant name (e.g. "Circle", "Rectangle", "Ok", "Err").
    pub name: String,
    /// Variant discriminant (assigned sequentially: 0, 1, 2, ...).
    pub tag: u8,
    /// Per-field byte offsets within the payload area (NOT including header/tag).
    pub field_offsets: Vec<usize>,
    /// Per-field native types.
    pub field_kinds: Vec<FieldKind>,
    /// Total size of the payload for this variant in bytes (sum of fields with padding).
    pub size: usize,
}

impl VariantLayout {
    /// Compute the layout of a single variant from its field types.
    ///
    /// Fields are packed starting at offset 0 (within the payload area) with
    /// natural alignment. The reported `size` is the offset just past the last
    /// field — the enum-level layout is responsible for rounding the max
    /// payload size up to 8-byte alignment.
    fn compute(name: String, tag: u8, fields: &[FieldKind]) -> Self {
        let mut current_offset = 0usize;
        let mut field_offsets = Vec::with_capacity(fields.len());
        let mut field_kinds = Vec::with_capacity(fields.len());

        for kind in fields {
            let align = kind.alignment();
            let size = kind.size();
            // Align current_offset to field's natural alignment.
            current_offset = (current_offset + align - 1) & !(align - 1);
            field_offsets.push(current_offset);
            field_kinds.push(*kind);
            current_offset += size;
        }

        VariantLayout {
            name,
            tag,
            field_offsets,
            field_kinds,
            size: current_offset,
        }
    }

    /// Number of fields in this variant (0 for unit variants).
    #[inline]
    pub fn field_count(&self) -> usize {
        self.field_offsets.len()
    }

    /// Whether this is a unit variant (no payload fields).
    #[inline]
    pub fn is_unit(&self) -> bool {
        self.field_offsets.is_empty()
    }
}

/// Layout of a complete typed enum.
///
/// This describes the memory layout of any heap-allocated value of this enum
/// type: an 8-byte `HeapHeader`, a 1-byte tag at offset 8, padding, and then
/// `max_payload_size` bytes of payload starting at offset 16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumLayout {
    /// Enum type name (for debugging / error messages).
    pub name: String,
    /// All variants in declaration order. Each variant's `tag` matches its index.
    pub variants: Vec<VariantLayout>,
    /// Largest payload size across all variants (before final 8-byte rounding).
    pub max_payload_size: usize,
    /// Total size of the heap object: 16 (header + tag + padding) + payload,
    /// rounded up to 8-byte alignment.
    pub total_size: usize,
}

/// Byte offset of the tag within a typed enum heap object.
pub const ENUM_TAG_OFFSET: usize = 8;
/// Byte offset of the payload area within a typed enum heap object.
/// 8 bytes header + 1 byte tag + 7 bytes padding = 16.
pub const ENUM_PAYLOAD_OFFSET: usize = 16;

/// Compute the full layout of a typed enum from its variant declarations.
///
/// Variants are assigned sequential tags 0, 1, 2, ... in the order they appear.
/// Each variant's payload is packed naturally aligned starting at offset 0
/// within the payload area. The enum's `max_payload_size` is the maximum across
/// all variants, and `total_size` is `ENUM_PAYLOAD_OFFSET + max_payload_size`,
/// rounded up to 8-byte alignment.
///
/// # Panics
/// Panics if more than 256 variants are provided (tag is `u8`).
pub fn compute_enum_layout(name: &str, variants: &[(String, Vec<FieldKind>)]) -> EnumLayout {
    assert!(
        variants.len() <= 256,
        "enum {} has {} variants — exceeds u8 tag limit (256)",
        name,
        variants.len()
    );

    let mut variant_layouts = Vec::with_capacity(variants.len());
    let mut max_payload_size = 0usize;

    for (i, (vname, fields)) in variants.iter().enumerate() {
        let tag = i as u8;
        let layout = VariantLayout::compute(vname.clone(), tag, fields);
        if layout.size > max_payload_size {
            max_payload_size = layout.size;
        }
        variant_layouts.push(layout);
    }

    // Round total size up to 8-byte alignment.
    let raw_total = ENUM_PAYLOAD_OFFSET + max_payload_size;
    let total_size = (raw_total + 7) & !7;

    EnumLayout {
        name: name.to_string(),
        variants: variant_layouts,
        max_payload_size,
        total_size,
    }
}

impl EnumLayout {
    /// Number of variants.
    #[inline]
    pub fn variant_count(&self) -> usize {
        self.variants.len()
    }

    /// Look up a variant's tag by name. Returns `None` if no such variant.
    pub fn variant_tag(&self, name: &str) -> Option<u8> {
        self.variants
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.tag)
    }

    /// Look up a variant by tag.
    pub fn variant_layout(&self, tag: u8) -> Option<&VariantLayout> {
        self.variants.get(tag as usize)
    }

    /// Look up a variant by name.
    pub fn variant_by_name(&self, name: &str) -> Option<&VariantLayout> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// Allocate raw bytes for an enum object of this layout.
    ///
    /// Returns a raw pointer to `total_size` bytes of zeroed memory, with no
    /// header initialized. Callers should usually prefer
    /// [`crate::v2::typed_enum::alloc_typed_enum`], which writes the
    /// `HeapHeader` for them.
    ///
    /// The returned pointer is allocated via `std::alloc::alloc` with 8-byte
    /// alignment and must eventually be freed via `std::alloc::dealloc` with
    /// a matching `Layout`.
    ///
    /// # Safety
    /// The returned memory is uninitialized (other than what `alloc` returns).
    /// Caller is responsible for writing a valid `HeapHeader`, the tag byte,
    /// and the payload before reading from it.
    pub fn alloc(&self) -> *mut u8 {
        let layout = Layout::from_size_align(self.total_size, 8)
            .expect("enum layout size/align is invalid");
        let ptr = unsafe { alloc(layout) };
        assert!(!ptr.is_null(), "allocation failed for typed enum");
        ptr
    }

    /// Free a previously allocated enum object pointer.
    ///
    /// # Safety
    /// `ptr` must have been returned from `self.alloc()` (or
    /// `alloc_typed_enum(self)`) and must not be used afterwards.
    pub unsafe fn dealloc(&self, ptr: *mut u8) {
        let layout = Layout::from_size_align(self.total_size, 8)
            .expect("enum layout size/align is invalid");
        unsafe { std::alloc::dealloc(ptr, layout) };
    }

    /// Initialize an allocated enum object with a fresh `HeapHeader` and the
    /// given tag. Payload bytes remain uninitialized.
    ///
    /// # Safety
    /// `ptr` must point to at least `self.total_size` bytes of writable memory
    /// allocated for an enum of this layout.
    pub unsafe fn init_header(&self, ptr: *mut u8, tag: u8) {
        unsafe {
            std::ptr::write(ptr as *mut HeapHeader, HeapHeader::new(HEAP_KIND_V2_TYPED_ENUM));
            *ptr.add(ENUM_TAG_OFFSET) = tag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shape_enum_layout() {
        // enum Shape { Circle(f64), Rectangle(f64, f64) }
        let variants = vec![
            ("Circle".to_string(), vec![FieldKind::F64]),
            ("Rectangle".to_string(), vec![FieldKind::F64, FieldKind::F64]),
        ];
        let layout = compute_enum_layout("Shape", &variants);

        assert_eq!(layout.name, "Shape");
        assert_eq!(layout.variant_count(), 2);

        // Circle: tag 0, one f64 at offset 0, payload size 8
        let circle = &layout.variants[0];
        assert_eq!(circle.name, "Circle");
        assert_eq!(circle.tag, 0);
        assert_eq!(circle.field_offsets, vec![0]);
        assert_eq!(circle.field_kinds, vec![FieldKind::F64]);
        assert_eq!(circle.size, 8);

        // Rectangle: tag 1, two f64s at 0 and 8, payload size 16
        let rect = &layout.variants[1];
        assert_eq!(rect.name, "Rectangle");
        assert_eq!(rect.tag, 1);
        assert_eq!(rect.field_offsets, vec![0, 8]);
        assert_eq!(rect.field_kinds, vec![FieldKind::F64, FieldKind::F64]);
        assert_eq!(rect.size, 16);

        // Max payload = 16, total = 16 (header+tag+pad) + 16 = 32
        assert_eq!(layout.max_payload_size, 16);
        assert_eq!(layout.total_size, 32);
    }

    #[test]
    fn test_color_unit_variants() {
        // enum Color { Red, Green, Blue }
        let variants = vec![
            ("Red".to_string(), vec![]),
            ("Green".to_string(), vec![]),
            ("Blue".to_string(), vec![]),
        ];
        let layout = compute_enum_layout("Color", &variants);

        assert_eq!(layout.variant_count(), 3);
        assert_eq!(layout.max_payload_size, 0);
        // 16 + 0 = 16, already 8-aligned
        assert_eq!(layout.total_size, 16);

        for (i, name) in ["Red", "Green", "Blue"].iter().enumerate() {
            let v = &layout.variants[i];
            assert_eq!(v.name, *name);
            assert_eq!(v.tag, i as u8);
            assert!(v.is_unit());
            assert_eq!(v.field_count(), 0);
            assert_eq!(v.size, 0);
        }
    }

    #[test]
    fn test_result_enum_with_f64() {
        // enum Result<T, E> { Ok(T), Err(E) } with T = E = f64
        let variants = vec![
            ("Ok".to_string(), vec![FieldKind::F64]),
            ("Err".to_string(), vec![FieldKind::F64]),
        ];
        let layout = compute_enum_layout("Result", &variants);

        assert_eq!(layout.variant_count(), 2);
        assert_eq!(layout.max_payload_size, 8);
        // 16 + 8 = 24, already 8-aligned
        assert_eq!(layout.total_size, 24);

        let ok = &layout.variants[0];
        assert_eq!(ok.tag, 0);
        assert_eq!(ok.field_offsets, vec![0]);
        assert_eq!(ok.size, 8);

        let err = &layout.variants[1];
        assert_eq!(err.tag, 1);
        assert_eq!(err.field_offsets, vec![0]);
        assert_eq!(err.size, 8);
    }

    #[test]
    fn test_variant_tag_lookup() {
        let variants = vec![
            ("Red".to_string(), vec![]),
            ("Green".to_string(), vec![]),
            ("Blue".to_string(), vec![]),
        ];
        let layout = compute_enum_layout("Color", &variants);

        assert_eq!(layout.variant_tag("Red"), Some(0));
        assert_eq!(layout.variant_tag("Green"), Some(1));
        assert_eq!(layout.variant_tag("Blue"), Some(2));
        assert_eq!(layout.variant_tag("Unknown"), None);
    }

    #[test]
    fn test_variant_layout_lookup() {
        let variants = vec![
            ("Circle".to_string(), vec![FieldKind::F64]),
            ("Rectangle".to_string(), vec![FieldKind::F64, FieldKind::F64]),
        ];
        let layout = compute_enum_layout("Shape", &variants);

        let v0 = layout.variant_layout(0).unwrap();
        assert_eq!(v0.name, "Circle");

        let v1 = layout.variant_layout(1).unwrap();
        assert_eq!(v1.name, "Rectangle");

        assert!(layout.variant_layout(2).is_none());
        assert!(layout.variant_layout(255).is_none());
    }

    #[test]
    fn test_variant_by_name_lookup() {
        let variants = vec![
            ("Some".to_string(), vec![FieldKind::I64]),
            ("None".to_string(), vec![]),
        ];
        let layout = compute_enum_layout("Option", &variants);

        let some = layout.variant_by_name("Some").unwrap();
        assert_eq!(some.tag, 0);
        assert_eq!(some.field_kinds, vec![FieldKind::I64]);

        let none = layout.variant_by_name("None").unwrap();
        assert_eq!(none.tag, 1);
        assert!(none.is_unit());

        assert!(layout.variant_by_name("Other").is_none());
    }

    #[test]
    fn test_mixed_alignment_within_variant() {
        // Variant with i32, f64, i8 — exercise alignment padding within payload.
        // Layout: i32 at 0 (size 4) → padding to 8 → f64 at 8 (size 8) → i8 at 16 (size 1).
        // Total size 17.
        let variants = vec![(
            "Mixed".to_string(),
            vec![FieldKind::I32, FieldKind::F64, FieldKind::I8],
        )];
        let layout = compute_enum_layout("MixedEnum", &variants);

        let v = &layout.variants[0];
        assert_eq!(v.field_offsets, vec![0, 8, 16]);
        assert_eq!(v.size, 17);
        assert_eq!(layout.max_payload_size, 17);
        // 16 (header+tag+pad) + 17 = 33, rounded up to 40
        assert_eq!(layout.total_size, 40);
    }

    #[test]
    fn test_alloc_and_init_header() {
        let variants = vec![
            ("Circle".to_string(), vec![FieldKind::F64]),
            ("Rectangle".to_string(), vec![FieldKind::F64, FieldKind::F64]),
        ];
        let layout = compute_enum_layout("Shape", &variants);

        let ptr = layout.alloc();
        assert!(!ptr.is_null());

        unsafe {
            layout.init_header(ptr, 1);
            // Verify header kind
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.kind(), HEAP_KIND_V2_TYPED_ENUM);
            assert_eq!(header.get_refcount(), 1);
            // Verify tag
            assert_eq!(*ptr.add(ENUM_TAG_OFFSET), 1);

            layout.dealloc(ptr);
        }
    }

    #[test]
    fn test_total_size_rounding() {
        // Single byte payload → 16 + 1 = 17 → rounds up to 24
        let variants = vec![("V".to_string(), vec![FieldKind::U8])];
        let layout = compute_enum_layout("E", &variants);
        assert_eq!(layout.max_payload_size, 1);
        assert_eq!(layout.total_size, 24);
    }

    #[test]
    fn test_empty_enum_total_size() {
        // No variants at all — degenerate but should not panic.
        let layout = compute_enum_layout("Empty", &[]);
        assert_eq!(layout.variant_count(), 0);
        assert_eq!(layout.max_payload_size, 0);
        // 16 + 0 = 16
        assert_eq!(layout.total_size, 16);
    }

    #[test]
    fn test_sequential_tags() {
        let variants = vec![
            ("A".to_string(), vec![]),
            ("B".to_string(), vec![FieldKind::F64]),
            ("C".to_string(), vec![FieldKind::I32]),
            ("D".to_string(), vec![FieldKind::Bool]),
        ];
        let layout = compute_enum_layout("Many", &variants);
        for (i, v) in layout.variants.iter().enumerate() {
            assert_eq!(v.tag, i as u8);
        }
    }
}
