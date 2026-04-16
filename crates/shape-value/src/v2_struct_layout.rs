//! Compile-time `repr(C)` struct layout computation for the v2 runtime.
//!
//! Given a type definition like `type Point { x: number, y: number }`, this module
//! computes the exact byte layout with field offsets as compile-time constants:
//!
//! ```text
//! #[repr(C)]
//! struct PointLayout {
//!     header: HeapHeader,   // 8 bytes (offset 0)
//!     x: f64,               // 8 bytes (offset 8)
//!     y: f64,               // 8 bytes (offset 16)
//! }
//! ```
//!
//! Field access compiles to a direct load at a known offset: `point.x` becomes
//! `load f64 [ptr + 8]`. No schema lookup, no HashMap, no runtime dispatch.

/// Primitive field types with known sizes and alignments.
///
/// These map directly to machine types the JIT can emit loads/stores for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldType {
    /// 64-bit IEEE 754 float (`number` in Shape). 8 bytes, 8-byte aligned.
    F64,
    /// 64-bit signed integer. 8 bytes, 8-byte aligned.
    I64,
    /// 32-bit signed integer. 4 bytes, 4-byte aligned.
    I32,
    /// 32-bit unsigned integer. 4 bytes, 4-byte aligned.
    U32,
    /// 16-bit signed integer. 2 bytes, 2-byte aligned.
    I16,
    /// 16-bit unsigned integer. 2 bytes, 2-byte aligned.
    U16,
    /// 8-bit signed integer. 1 byte, 1-byte aligned.
    I8,
    /// 8-bit unsigned integer. 1 byte, 1-byte aligned.
    U8,
    /// Boolean. 1 byte, 1-byte aligned.
    Bool,
    /// Pointer to a heap object. 8 bytes, 8-byte aligned.
    Ptr,
}

impl FieldType {
    /// Size of this field type in bytes.
    #[inline]
    pub const fn size(self) -> u32 {
        match self {
            FieldType::F64 | FieldType::I64 | FieldType::Ptr => 8,
            FieldType::I32 | FieldType::U32 => 4,
            FieldType::I16 | FieldType::U16 => 2,
            FieldType::I8 | FieldType::U8 | FieldType::Bool => 1,
        }
    }

    /// Natural alignment of this field type in bytes.
    #[inline]
    pub const fn align(self) -> u32 {
        // Natural alignment: size == alignment for all primitive types.
        self.size()
    }
}

/// Layout information for a single field within a struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructFieldLayout {
    /// Field name (e.g. `"x"`).
    pub name: String,
    /// Byte offset from the start of the struct (including header).
    pub offset: u32,
    /// Size of this field in bytes.
    pub size: u32,
    /// The primitive type of this field.
    pub field_type: FieldType,
}

/// Complete layout of a `repr(C)` struct including its v2 heap header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructLayout {
    /// Type name (e.g. `"Point"`).
    pub name: String,
    /// Total size of the struct in bytes, including header and tail padding.
    /// Padded to the struct's overall alignment.
    pub total_size: u32,
    /// Per-field layout information, in declaration order.
    pub fields: Vec<StructFieldLayout>,
}

/// Size of the v2 heap header in bytes.
///
/// The v2 runtime uses a compact 8-byte header at offset 0 of every heap object.
/// This is distinct from the v1 `HeapHeader` (32 bytes). The v2 header packs
/// kind, flags, and auxiliary data into 8 bytes.
pub const V2_HEADER_SIZE: u32 = 8;

/// Alignment of the v2 heap header.
pub const V2_HEADER_ALIGN: u32 = 8;

/// Round `offset` up to the next multiple of `align`.
///
/// `align` must be a power of two.
#[inline]
const fn align_up(offset: u32, align: u32) -> u32 {
    debug_assert!(align.is_power_of_two());
    let mask = align - 1;
    (offset + mask) & !mask
}

/// Compute the `repr(C)` struct layout for a named type with the given fields.
///
/// The layout follows C struct rules:
/// 1. An 8-byte v2 heap header occupies bytes `[0, 8)`.
/// 2. Fields are placed sequentially after the header, each aligned to its
///    natural alignment (inserting padding bytes as needed).
/// 3. The total size is padded to the struct's overall alignment (the maximum
///    alignment of the header and all fields).
///
/// # Examples
///
/// ```
/// use shape_value::v2_struct_layout::{compute_struct_layout, FieldType};
use crate::value_word::ValueWordExt;
///
/// let layout = compute_struct_layout("Point", &[
///     ("x".into(), FieldType::F64),
///     ("y".into(), FieldType::F64),
/// ]);
/// assert_eq!(layout.fields[0].offset, 8);  // after 8-byte header
/// assert_eq!(layout.fields[1].offset, 16);
/// assert_eq!(layout.total_size, 24);
/// ```
pub fn compute_struct_layout(name: &str, fields: &[(String, FieldType)]) -> StructLayout {
    // Track the maximum alignment across header and all fields.
    let mut max_align = V2_HEADER_ALIGN;
    // Current write cursor starts after the header.
    let mut cursor = V2_HEADER_SIZE;

    let mut field_layouts = Vec::with_capacity(fields.len());

    for (field_name, field_type) in fields {
        let size = field_type.size();
        let align = field_type.align();

        // Update struct-wide max alignment.
        if align > max_align {
            max_align = align;
        }

        // Align cursor for this field.
        cursor = align_up(cursor, align);

        field_layouts.push(StructFieldLayout {
            name: field_name.clone(),
            offset: cursor,
            size,
            field_type: *field_type,
        });

        cursor += size;
    }

    // Pad total size to struct alignment (C layout rule).
    let total_size = align_up(cursor, max_align);

    StructLayout {
        name: name.to_string(),
        total_size,
        fields: field_layouts,
    }
}

impl StructLayout {
    /// Look up a field by name, returning its layout if found.
    pub fn field(&self, name: &str) -> Option<&StructFieldLayout> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// The overall alignment of this struct (max of header and field alignments).
    pub fn alignment(&self) -> u32 {
        let mut max_align = V2_HEADER_ALIGN;
        for f in &self.fields {
            let a = f.field_type.align();
            if a > max_align {
                max_align = a;
            }
        }
        max_align
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // FieldType basics
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_type_sizes() {
        assert_eq!(FieldType::F64.size(), 8);
        assert_eq!(FieldType::I64.size(), 8);
        assert_eq!(FieldType::Ptr.size(), 8);
        assert_eq!(FieldType::I32.size(), 4);
        assert_eq!(FieldType::U32.size(), 4);
        assert_eq!(FieldType::I16.size(), 2);
        assert_eq!(FieldType::U16.size(), 2);
        assert_eq!(FieldType::I8.size(), 1);
        assert_eq!(FieldType::U8.size(), 1);
        assert_eq!(FieldType::Bool.size(), 1);
    }

    #[test]
    fn test_field_type_alignments() {
        assert_eq!(FieldType::F64.align(), 8);
        assert_eq!(FieldType::I64.align(), 8);
        assert_eq!(FieldType::Ptr.align(), 8);
        assert_eq!(FieldType::I32.align(), 4);
        assert_eq!(FieldType::U32.align(), 4);
        assert_eq!(FieldType::I16.align(), 2);
        assert_eq!(FieldType::U16.align(), 2);
        assert_eq!(FieldType::I8.align(), 1);
        assert_eq!(FieldType::U8.align(), 1);
        assert_eq!(FieldType::Bool.align(), 1);
    }

    // -----------------------------------------------------------------------
    // align_up helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_align_up() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);
        assert_eq!(align_up(3, 4), 4);
        assert_eq!(align_up(4, 4), 4);
        assert_eq!(align_up(5, 4), 8);
        assert_eq!(align_up(0, 1), 0);
        assert_eq!(align_up(7, 1), 7);
        assert_eq!(align_up(1, 2), 2);
        assert_eq!(align_up(2, 2), 2);
        assert_eq!(align_up(3, 2), 4);
    }

    // -----------------------------------------------------------------------
    // Point { x: f64, y: f64 }
    // -----------------------------------------------------------------------

    #[test]
    fn test_point_layout() {
        let layout = compute_struct_layout("Point", &[
            ("x".into(), FieldType::F64),
            ("y".into(), FieldType::F64),
        ]);

        assert_eq!(layout.name, "Point");
        assert_eq!(layout.fields.len(), 2);

        // x: f64 at offset 8 (right after 8-byte header)
        assert_eq!(layout.fields[0].name, "x");
        assert_eq!(layout.fields[0].offset, 8);
        assert_eq!(layout.fields[0].size, 8);
        assert_eq!(layout.fields[0].field_type, FieldType::F64);

        // y: f64 at offset 16
        assert_eq!(layout.fields[1].name, "y");
        assert_eq!(layout.fields[1].offset, 16);
        assert_eq!(layout.fields[1].size, 8);
        assert_eq!(layout.fields[1].field_type, FieldType::F64);

        // Total: header(8) + x(8) + y(8) = 24, aligned to 8 = 24
        assert_eq!(layout.total_size, 24);
        assert_eq!(layout.alignment(), 8);
    }

    // -----------------------------------------------------------------------
    // Color { r: u8, g: u8, b: u8 }
    // -----------------------------------------------------------------------

    #[test]
    fn test_color_layout() {
        let layout = compute_struct_layout("Color", &[
            ("r".into(), FieldType::U8),
            ("g".into(), FieldType::U8),
            ("b".into(), FieldType::U8),
        ]);

        assert_eq!(layout.name, "Color");
        assert_eq!(layout.fields.len(), 3);

        // r: u8 at offset 8 (right after header, 1-byte aligned — no padding)
        assert_eq!(layout.fields[0].name, "r");
        assert_eq!(layout.fields[0].offset, 8);
        assert_eq!(layout.fields[0].size, 1);

        // g: u8 at offset 9
        assert_eq!(layout.fields[1].name, "g");
        assert_eq!(layout.fields[1].offset, 9);
        assert_eq!(layout.fields[1].size, 1);

        // b: u8 at offset 10
        assert_eq!(layout.fields[2].name, "b");
        assert_eq!(layout.fields[2].offset, 10);
        assert_eq!(layout.fields[2].size, 1);

        // Total: header(8) + r(1) + g(1) + b(1) = 11, padded to max_align(8) = 16
        assert_eq!(layout.total_size, 16);
        assert_eq!(layout.alignment(), 8);
    }

    // -----------------------------------------------------------------------
    // Mixed types with alignment padding
    // -----------------------------------------------------------------------

    #[test]
    fn test_mixed_alignment_padding() {
        // Simulates: type Mixed { flag: bool, value: f64, count: i32 }
        let layout = compute_struct_layout("Mixed", &[
            ("flag".into(), FieldType::Bool),
            ("value".into(), FieldType::F64),
            ("count".into(), FieldType::I32),
        ]);

        assert_eq!(layout.fields.len(), 3);

        // flag: bool at offset 8 (1 byte)
        assert_eq!(layout.fields[0].name, "flag");
        assert_eq!(layout.fields[0].offset, 8);
        assert_eq!(layout.fields[0].size, 1);

        // value: f64 at offset 16 (needs 8-byte alignment, so 7 bytes of padding after flag)
        assert_eq!(layout.fields[1].name, "value");
        assert_eq!(layout.fields[1].offset, 16);
        assert_eq!(layout.fields[1].size, 8);

        // count: i32 at offset 24 (needs 4-byte alignment, already aligned)
        assert_eq!(layout.fields[2].name, "count");
        assert_eq!(layout.fields[2].offset, 24);
        assert_eq!(layout.fields[2].size, 4);

        // Total: 24 + 4 = 28, padded to 8 = 32
        assert_eq!(layout.total_size, 32);
        assert_eq!(layout.alignment(), 8);
    }

    // -----------------------------------------------------------------------
    // Empty struct (header only)
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_struct() {
        let layout = compute_struct_layout("Empty", &[]);

        assert_eq!(layout.name, "Empty");
        assert_eq!(layout.fields.len(), 0);
        // Just the header, padded to header alignment
        assert_eq!(layout.total_size, 8);
        assert_eq!(layout.alignment(), 8);
    }

    // -----------------------------------------------------------------------
    // Single field
    // -----------------------------------------------------------------------

    #[test]
    fn test_single_bool_field() {
        let layout = compute_struct_layout("Flag", &[
            ("value".into(), FieldType::Bool),
        ]);

        assert_eq!(layout.fields[0].offset, 8);
        assert_eq!(layout.fields[0].size, 1);
        // header(8) + bool(1) = 9, padded to 8 = 16
        assert_eq!(layout.total_size, 16);
    }

    #[test]
    fn test_single_i32_field() {
        let layout = compute_struct_layout("Counter", &[
            ("n".into(), FieldType::I32),
        ]);

        assert_eq!(layout.fields[0].offset, 8);
        assert_eq!(layout.fields[0].size, 4);
        // header(8) + i32(4) = 12, padded to 8 = 16
        assert_eq!(layout.total_size, 16);
    }

    // -----------------------------------------------------------------------
    // All field types in sequence
    // -----------------------------------------------------------------------

    #[test]
    fn test_all_field_types() {
        let layout = compute_struct_layout("AllTypes", &[
            ("a_f64".into(), FieldType::F64),
            ("b_i64".into(), FieldType::I64),
            ("c_ptr".into(), FieldType::Ptr),
            ("d_i32".into(), FieldType::I32),
            ("e_u32".into(), FieldType::U32),
            ("f_i16".into(), FieldType::I16),
            ("g_u16".into(), FieldType::U16),
            ("h_i8".into(), FieldType::I8),
            ("i_u8".into(), FieldType::U8),
            ("j_bool".into(), FieldType::Bool),
        ]);

        // All 8-byte fields first (no padding between them or after header)
        assert_eq!(layout.fields[0].offset, 8);   // f64 @ 8
        assert_eq!(layout.fields[1].offset, 16);  // i64 @ 16
        assert_eq!(layout.fields[2].offset, 24);  // ptr @ 24

        // 4-byte fields (naturally aligned after ptr ends at 32)
        assert_eq!(layout.fields[3].offset, 32);  // i32 @ 32
        assert_eq!(layout.fields[4].offset, 36);  // u32 @ 36

        // 2-byte fields (naturally aligned after u32 ends at 40)
        assert_eq!(layout.fields[5].offset, 40);  // i16 @ 40
        assert_eq!(layout.fields[6].offset, 42);  // u16 @ 42

        // 1-byte fields (no alignment needed)
        assert_eq!(layout.fields[7].offset, 44);  // i8 @ 44
        assert_eq!(layout.fields[8].offset, 45);  // u8 @ 45
        assert_eq!(layout.fields[9].offset, 46);  // bool @ 46

        // Total: 47, padded to 8 = 48
        assert_eq!(layout.total_size, 48);
        assert_eq!(layout.alignment(), 8);
    }

    // -----------------------------------------------------------------------
    // Padding between small and large fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_i16_then_i64_padding() {
        // type T { a: i16, b: i64 }
        let layout = compute_struct_layout("T", &[
            ("a".into(), FieldType::I16),
            ("b".into(), FieldType::I64),
        ]);

        // a: i16 at offset 8 (2 bytes)
        assert_eq!(layout.fields[0].offset, 8);
        // b: i64 needs 8-byte alignment. cursor=10, aligned to 8 → 16
        assert_eq!(layout.fields[1].offset, 16);
        // Total: 16 + 8 = 24, already aligned to 8
        assert_eq!(layout.total_size, 24);
    }

    #[test]
    fn test_bool_i32_bool_padding() {
        // type T { a: bool, b: i32, c: bool }
        let layout = compute_struct_layout("T", &[
            ("a".into(), FieldType::Bool),
            ("b".into(), FieldType::I32),
            ("c".into(), FieldType::Bool),
        ]);

        // a: bool at offset 8
        assert_eq!(layout.fields[0].offset, 8);
        // b: i32 needs 4-byte alignment. cursor=9, aligned to 4 → 12
        assert_eq!(layout.fields[1].offset, 12);
        // c: bool at offset 16 (12+4=16, 1-byte aligned)
        assert_eq!(layout.fields[2].offset, 16);
        // Total: 17, padded to max_align=8 → 24
        assert_eq!(layout.total_size, 24);
    }

    // -----------------------------------------------------------------------
    // Pointer fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_struct_with_pointer_fields() {
        // type Node { value: i32, next: ptr, prev: ptr }
        let layout = compute_struct_layout("Node", &[
            ("value".into(), FieldType::I32),
            ("next".into(), FieldType::Ptr),
            ("prev".into(), FieldType::Ptr),
        ]);

        // value: i32 at offset 8
        assert_eq!(layout.fields[0].offset, 8);
        assert_eq!(layout.fields[0].size, 4);

        // next: ptr needs 8-byte alignment. cursor=12, aligned to 8 → 16
        assert_eq!(layout.fields[1].offset, 16);
        assert_eq!(layout.fields[1].size, 8);

        // prev: ptr at offset 24
        assert_eq!(layout.fields[2].offset, 24);
        assert_eq!(layout.fields[2].size, 8);

        // Total: 32, already aligned to 8
        assert_eq!(layout.total_size, 32);
    }

    // -----------------------------------------------------------------------
    // field() lookup
    // -----------------------------------------------------------------------

    #[test]
    fn test_field_lookup_by_name() {
        let layout = compute_struct_layout("Point", &[
            ("x".into(), FieldType::F64),
            ("y".into(), FieldType::F64),
        ]);

        let x = layout.field("x").expect("field 'x' should exist");
        assert_eq!(x.offset, 8);
        assert_eq!(x.field_type, FieldType::F64);

        let y = layout.field("y").expect("field 'y' should exist");
        assert_eq!(y.offset, 16);
        assert_eq!(y.field_type, FieldType::F64);

        assert!(layout.field("z").is_none());
    }

    // -----------------------------------------------------------------------
    // Worst-case padding scenario
    // -----------------------------------------------------------------------

    #[test]
    fn test_worst_case_padding() {
        // Deliberately adversarial ordering: small, large, small, large
        // type Padded { a: u8, b: f64, c: u8, d: f64 }
        let layout = compute_struct_layout("Padded", &[
            ("a".into(), FieldType::U8),
            ("b".into(), FieldType::F64),
            ("c".into(), FieldType::U8),
            ("d".into(), FieldType::F64),
        ]);

        // a: u8 at offset 8
        assert_eq!(layout.fields[0].offset, 8);
        // b: f64 needs 8-byte alignment. cursor=9 → 16
        assert_eq!(layout.fields[1].offset, 16);
        // c: u8 at offset 24
        assert_eq!(layout.fields[2].offset, 24);
        // d: f64 needs 8-byte alignment. cursor=25 → 32
        assert_eq!(layout.fields[3].offset, 32);
        // Total: 32 + 8 = 40, aligned to 8 = 40
        assert_eq!(layout.total_size, 40);
    }

    // -----------------------------------------------------------------------
    // Compile-time constant offsets — verifies offsets are deterministic
    // -----------------------------------------------------------------------

    #[test]
    fn test_layout_is_deterministic() {
        let fields: Vec<(String, FieldType)> = vec![
            ("a".into(), FieldType::I32),
            ("b".into(), FieldType::Bool),
            ("c".into(), FieldType::F64),
        ];

        let layout1 = compute_struct_layout("T", &fields);
        let layout2 = compute_struct_layout("T", &fields);

        assert_eq!(layout1, layout2);
    }

    // -----------------------------------------------------------------------
    // Only 16-bit fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_only_16bit_fields() {
        let layout = compute_struct_layout("Shorts", &[
            ("a".into(), FieldType::I16),
            ("b".into(), FieldType::U16),
            ("c".into(), FieldType::I16),
        ]);

        assert_eq!(layout.fields[0].offset, 8);  // i16 at 8
        assert_eq!(layout.fields[1].offset, 10); // u16 at 10
        assert_eq!(layout.fields[2].offset, 12); // i16 at 12

        // Total: 14, padded to max_align=8 → 16
        assert_eq!(layout.total_size, 16);
    }

    // -----------------------------------------------------------------------
    // V2_HEADER constants
    // -----------------------------------------------------------------------

    #[test]
    fn test_header_constants() {
        assert_eq!(V2_HEADER_SIZE, 8);
        assert_eq!(V2_HEADER_ALIGN, 8);
    }
}
