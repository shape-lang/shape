/// StructLayout engine: computes C-compatible field layouts from type definitions.
///
/// Every typed struct starts with HeapHeader (8 bytes) then fields in declaration
/// order with natural alignment padding.

/// Field type for v2 struct layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    F64,  // 8 bytes, 8-align
    I64,  // 8 bytes, 8-align
    I32,  // 4 bytes, 4-align
    I16,  // 2 bytes, 2-align
    I8,   // 1 byte, 1-align
    U64,  // 8 bytes, 8-align
    U32,  // 4 bytes, 4-align
    U16,  // 2 bytes, 2-align
    U8,   // 1 byte, 1-align
    Bool, // 1 byte, 1-align
    Ptr,  // 8 bytes, 8-align — typed pointer to heap object
}

impl FieldKind {
    pub fn size(&self) -> usize {
        match self {
            FieldKind::F64 | FieldKind::I64 | FieldKind::U64 | FieldKind::Ptr => 8,
            FieldKind::I32 | FieldKind::U32 => 4,
            FieldKind::I16 | FieldKind::U16 => 2,
            FieldKind::I8 | FieldKind::U8 | FieldKind::Bool => 1,
        }
    }

    pub fn alignment(&self) -> usize {
        self.size()
    }
}

/// Information about a single field in a struct layout.
#[derive(Debug)]
pub struct FieldInfo {
    pub name: String,
    pub kind: FieldKind,
    pub offset: usize, // byte offset from START of struct (including header)
    pub size: usize,
}

/// Computed C-compatible layout for a typed struct.
///
/// The layout starts with an 8-byte HeapHeader at offset 0. Fields follow
/// at offset 8+ with natural alignment padding between them. The total size
/// is rounded up to 8-byte alignment.
#[derive(Debug)]
pub struct StructLayout {
    pub fields: Vec<FieldInfo>,
    pub total_size: usize,        // including HeapHeader (8 bytes)
    pub heap_field_mask: u64,     // bitmap: bit N = field N is Ptr type
}

impl StructLayout {
    /// Compute layout from field definitions. HeapHeader is at offset 0 (8 bytes).
    /// Fields start at offset 8, with natural alignment padding.
    pub fn new(fields: &[(impl AsRef<str>, FieldKind)]) -> Self {
        let mut current_offset = 8; // after HeapHeader
        let mut field_infos = Vec::new();
        let mut heap_mask: u64 = 0;

        for (i, (name, kind)) in fields.iter().enumerate() {
            let align = kind.alignment();
            let size = kind.size();
            // Align current_offset to field's natural alignment
            current_offset = (current_offset + align - 1) & !(align - 1);
            field_infos.push(FieldInfo {
                name: name.as_ref().to_string(),
                kind: *kind,
                offset: current_offset,
                size,
            });
            if *kind == FieldKind::Ptr {
                heap_mask |= 1u64 << i;
            }
            current_offset += size;
        }
        // Final size: align to 8 bytes
        let total_size = (current_offset + 7) & !7;

        StructLayout {
            fields: field_infos,
            total_size,
            heap_field_mask: heap_mask,
        }
    }

    pub fn field_offset(&self, idx: usize) -> usize {
        self.fields[idx].offset
    }

    pub fn field_kind(&self, idx: usize) -> FieldKind {
        self.fields[idx].kind
    }

    pub fn total_size(&self) -> usize {
        self.total_size
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_f64_f64() {
        // Point { x: F64, y: F64 }
        let layout = StructLayout::new(&[
            ("x", FieldKind::F64),
            ("y", FieldKind::F64),
        ]);
        assert_eq!(layout.field_count(), 2);
        assert_eq!(layout.field_offset(0), 8);  // x at offset 8
        assert_eq!(layout.field_offset(1), 16); // y at offset 16
        assert_eq!(layout.total_size(), 24);
        assert_eq!(layout.heap_field_mask, 0);
    }

    #[test]
    fn test_mixed_alignment_padding() {
        // Mixed { a: I32, b: F64, c: I8 }
        // a at offset 8 (4 bytes, 4-align: 8 is 4-aligned)
        // b at offset 16 (8 bytes, 8-align: 12 not 8-aligned, pad to 16)
        // c at offset 24 (1 byte, 1-align)
        // total: round 25 up to 32
        let layout = StructLayout::new(&[
            ("a", FieldKind::I32),
            ("b", FieldKind::F64),
            ("c", FieldKind::I8),
        ]);
        assert_eq!(layout.field_count(), 3);
        assert_eq!(layout.field_offset(0), 8);   // a: I32 at 8
        assert_eq!(layout.field_kind(0), FieldKind::I32);
        assert_eq!(layout.fields[0].size, 4);

        assert_eq!(layout.field_offset(1), 16);  // b: F64 at 16 (padded from 12)
        assert_eq!(layout.field_kind(1), FieldKind::F64);
        assert_eq!(layout.fields[1].size, 8);

        assert_eq!(layout.field_offset(2), 24);  // c: I8 at 24
        assert_eq!(layout.field_kind(2), FieldKind::I8);
        assert_eq!(layout.fields[2].size, 1);

        assert_eq!(layout.total_size(), 32);     // 25 rounded up to 32
    }

    #[test]
    fn test_all_field_kinds() {
        let layout = StructLayout::new(&[
            ("f0", FieldKind::F64),   // 8, size 8
            ("f1", FieldKind::I64),   // 16, size 8
            ("f2", FieldKind::I32),   // 24, size 4
            ("f3", FieldKind::I16),   // 28, size 2
            ("f4", FieldKind::I8),    // 30, size 1
            ("f5", FieldKind::U64),   // 32, size 8 (pad from 31 to 32)
            ("f6", FieldKind::U32),   // 40, size 4
            ("f7", FieldKind::U16),   // 44, size 2
            ("f8", FieldKind::U8),    // 46, size 1
            ("f9", FieldKind::Bool),  // 47, size 1
            ("f10", FieldKind::Ptr),  // 48, size 8
        ]);
        assert_eq!(layout.field_count(), 11);

        // F64 at 8
        assert_eq!(layout.field_offset(0), 8);
        // I64 at 16
        assert_eq!(layout.field_offset(1), 16);
        // I32 at 24
        assert_eq!(layout.field_offset(2), 24);
        // I16 at 28
        assert_eq!(layout.field_offset(3), 28);
        // I8 at 30
        assert_eq!(layout.field_offset(4), 30);
        // U64 at 32 (31 padded to 32)
        assert_eq!(layout.field_offset(5), 32);
        // U32 at 40
        assert_eq!(layout.field_offset(6), 40);
        // U16 at 44
        assert_eq!(layout.field_offset(7), 44);
        // U8 at 46
        assert_eq!(layout.field_offset(8), 46);
        // Bool at 47
        assert_eq!(layout.field_offset(9), 47);
        // Ptr at 48
        assert_eq!(layout.field_offset(10), 48);

        assert_eq!(layout.total_size(), 56); // 48 + 8 = 56
        // Only field 10 (Ptr) is heap
        assert_eq!(layout.heap_field_mask, 1u64 << 10);
    }

    #[test]
    fn test_heap_field_mask_positions_1_and_3() {
        // Struct with Ptr fields at positions 1 and 3
        let layout = StructLayout::new(&[
            ("a", FieldKind::I32),   // pos 0: not Ptr
            ("b", FieldKind::Ptr),   // pos 1: Ptr
            ("c", FieldKind::F64),   // pos 2: not Ptr
            ("d", FieldKind::Ptr),   // pos 3: Ptr
        ]);
        assert_eq!(layout.heap_field_mask, 0b1010);
    }

    #[test]
    fn test_empty_struct() {
        let layout = StructLayout::new(&[] as &[(&str, FieldKind)]);
        assert_eq!(layout.field_count(), 0);
        assert_eq!(layout.total_size(), 8); // header only
        assert_eq!(layout.heap_field_mask, 0);
    }

    #[test]
    fn test_single_bool_field() {
        // Struct with a single bool: header (8) + bool at 8 (1 byte) = 9 → round to 16
        let layout = StructLayout::new(&[("flag", FieldKind::Bool)]);
        assert_eq!(layout.field_count(), 1);
        assert_eq!(layout.field_offset(0), 8);
        assert_eq!(layout.fields[0].size, 1);
        assert_eq!(layout.total_size(), 16);
        assert_eq!(layout.heap_field_mask, 0);
    }

    #[test]
    fn test_all_ptr_fields() {
        let layout = StructLayout::new(&[
            ("a", FieldKind::Ptr),
            ("b", FieldKind::Ptr),
            ("c", FieldKind::Ptr),
        ]);
        assert_eq!(layout.field_offset(0), 8);
        assert_eq!(layout.field_offset(1), 16);
        assert_eq!(layout.field_offset(2), 24);
        assert_eq!(layout.total_size(), 32);
        assert_eq!(layout.heap_field_mask, 0b111);
    }

    #[test]
    fn test_small_fields_packing() {
        // Multiple small fields pack tightly
        let layout = StructLayout::new(&[
            ("a", FieldKind::I8),   // 8
            ("b", FieldKind::I8),   // 9
            ("c", FieldKind::I8),   // 10
            ("d", FieldKind::I8),   // 11
        ]);
        assert_eq!(layout.field_offset(0), 8);
        assert_eq!(layout.field_offset(1), 9);
        assert_eq!(layout.field_offset(2), 10);
        assert_eq!(layout.field_offset(3), 11);
        assert_eq!(layout.total_size(), 16); // 12 rounded to 16
    }

    #[test]
    fn test_field_names_preserved() {
        let layout = StructLayout::new(&[
            ("x_coord", FieldKind::F64),
            ("y_coord", FieldKind::F64),
        ]);
        assert_eq!(layout.fields[0].name, "x_coord");
        assert_eq!(layout.fields[1].name, "y_coord");
    }

    #[test]
    fn test_field_kind_size_and_alignment() {
        // Verify all FieldKind sizes
        assert_eq!(FieldKind::F64.size(), 8);
        assert_eq!(FieldKind::I64.size(), 8);
        assert_eq!(FieldKind::I32.size(), 4);
        assert_eq!(FieldKind::I16.size(), 2);
        assert_eq!(FieldKind::I8.size(), 1);
        assert_eq!(FieldKind::U64.size(), 8);
        assert_eq!(FieldKind::U32.size(), 4);
        assert_eq!(FieldKind::U16.size(), 2);
        assert_eq!(FieldKind::U8.size(), 1);
        assert_eq!(FieldKind::Bool.size(), 1);
        assert_eq!(FieldKind::Ptr.size(), 8);

        // Alignment equals size for natural alignment
        assert_eq!(FieldKind::F64.alignment(), 8);
        assert_eq!(FieldKind::I32.alignment(), 4);
        assert_eq!(FieldKind::I16.alignment(), 2);
        assert_eq!(FieldKind::Bool.alignment(), 1);
        assert_eq!(FieldKind::Ptr.alignment(), 8);
    }
}
