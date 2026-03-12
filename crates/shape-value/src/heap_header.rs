//! Fixed-layout heap object header for JIT-friendly type dispatch.
//!
//! `HeapHeader` is a `#[repr(C, align(16))]` struct that prefixes heap-allocated
//! objects, giving the JIT a stable memory layout to read the object's kind, length,
//! and capacity without depending on Rust's enum discriminant layout.
//!
//! ## Memory layout (32 bytes, 16-byte aligned)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       2   kind (HeapKind as u16)
//!   2       1   elem_type (element type hint for arrays/typed objects)
//!   3       1   flags (bitfield: MARKED, PINNED, READONLY, etc.)
//!   4       4   len (element count / field count)
//!   8       4   cap (allocated capacity, 0 if not applicable)
//!  12       4   (padding)
//!  16       8   aux (auxiliary data: schema_id, function_id, etc.)
//!  24       8   (reserved / future use)
//! ```

use crate::heap_value::{HeapKind, HeapValue};

/// Flag: object has been marked by the GC during a collection cycle.
pub const FLAG_MARKED: u8 = 0b0000_0001;
/// Flag: object is pinned and must not be relocated by the GC.
pub const FLAG_PINNED: u8 = 0b0000_0010;
/// Flag: object is read-only (immutable after construction).
pub const FLAG_READONLY: u8 = 0b0000_0100;

/// Fixed-layout header for heap-allocated objects.
///
/// This struct is designed to be readable by JIT-generated code at known offsets.
/// The JIT can load `kind` at offset 0, `len` at offset 4, and `aux` at offset 16
/// without any Rust ABI knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct HeapHeader {
    /// Object type discriminator (matches `HeapKind` and `HEAP_KIND_*` constants).
    pub kind: u16,
    /// Element type hint for homogeneous containers (0 = untyped/mixed).
    /// For arrays: 1=f64, 2=i64, 3=string, 4=bool, 5=typed_object.
    /// For typed objects: unused (0).
    pub elem_type: u8,
    /// Bitfield flags (FLAG_MARKED, FLAG_PINNED, FLAG_READONLY).
    pub flags: u8,
    /// Element count (array length, field count, string byte length, etc.).
    pub len: u32,
    /// Allocated capacity (for growable containers). 0 if not applicable.
    pub cap: u32,
    /// Padding to align `aux` at offset 16.
    _pad: u32,
    /// Auxiliary data interpreted per-kind:
    /// - TypedObject: schema_id (u64)
    /// - Closure: function_id (low u16) | captures_count (next u16)
    /// - TypedTable/RowView/ColumnRef/IndexedTable: schema_id (u64)
    /// - Future: future_id (u64)
    /// - Enum: variant_id (low u32)
    /// - Other: 0
    pub aux: u64,
    /// Reserved for future use (e.g., GC forwarding pointer).
    _reserved: u64,
}

/// Compile-time size and offset assertions.
const _: () = {
    assert!(std::mem::size_of::<HeapHeader>() == 32);
    assert!(std::mem::align_of::<HeapHeader>() == 16);
};

/// Element type hints for the `elem_type` field.
pub mod elem_types {
    /// Untyped or mixed-type container.
    pub const UNTYPED: u8 = 0;
    /// All elements are f64.
    pub const F64: u8 = 1;
    /// All elements are i64.
    pub const I64: u8 = 2;
    /// All elements are strings.
    pub const STRING: u8 = 3;
    /// All elements are bools.
    pub const BOOL: u8 = 4;
    /// All elements are typed objects.
    pub const TYPED_OBJECT: u8 = 5;
}

impl HeapHeader {
    /// Create a new HeapHeader with the given kind. All other fields are zeroed.
    #[inline]
    pub fn new(kind: HeapKind) -> Self {
        Self {
            kind: kind as u16,
            elem_type: 0,
            flags: 0,
            len: 0,
            cap: 0,
            _pad: 0,
            aux: 0,
            _reserved: 0,
        }
    }

    /// Create a HeapHeader with kind, length, and auxiliary data.
    #[inline]
    pub fn with_len_aux(kind: HeapKind, len: u32, aux: u64) -> Self {
        Self {
            kind: kind as u16,
            elem_type: 0,
            flags: 0,
            len,
            cap: 0,
            _pad: 0,
            aux,
            _reserved: 0,
        }
    }

    /// Build a HeapHeader from an existing HeapValue.
    ///
    /// Extracts kind, length, and auxiliary data from the HeapValue's contents.
    pub fn from_heap_value(value: &HeapValue) -> Self {
        let kind = value.kind();
        let mut header = Self::new(kind);

        match value {
            HeapValue::String(s) => {
                header.len = s.len() as u32;
            }
            HeapValue::Array(arr) => {
                header.len = arr.len() as u32;
                header.cap = arr.len() as u32;
            }
            HeapValue::TypedObject {
                schema_id, slots, ..
            } => {
                header.len = slots.len() as u32;
                header.aux = *schema_id;
            }
            HeapValue::Closure {
                function_id,
                upvalues,
            } => {
                header.len = upvalues.len() as u32;
                header.aux = *function_id as u64;
            }
            HeapValue::DataTable(dt) => {
                header.len = dt.row_count() as u32;
            }
            HeapValue::TypedTable { schema_id, table } => {
                header.len = table.row_count() as u32;
                header.aux = *schema_id;
            }
            HeapValue::RowView {
                schema_id, row_idx, ..
            } => {
                header.len = 1;
                header.aux = *schema_id;
                // Store row_idx in the lower 32 bits of _reserved via cap field
                header.cap = *row_idx as u32;
            }
            HeapValue::ColumnRef {
                schema_id, col_id, ..
            } => {
                header.aux = *schema_id;
                header.cap = *col_id;
            }
            HeapValue::IndexedTable {
                schema_id,
                table,
                index_col,
            } => {
                header.len = table.row_count() as u32;
                header.aux = *schema_id;
                header.cap = *index_col;
            }
            HeapValue::Enum(_) => {
                // Enum variant is identified by name, not index; no numeric aux needed.
            }
            HeapValue::Future(id) => {
                header.aux = *id;
            }
            HeapValue::TaskGroup { kind, task_ids } => {
                header.elem_type = *kind;
                header.len = task_ids.len() as u32;
            }
            // Remaining types: kind is sufficient, no extra metadata needed.
            _ => {}
        }

        header
    }

    /// Get the HeapKind from this header.
    #[inline]
    pub fn heap_kind(&self) -> Option<HeapKind> {
        HeapKind::from_u16(self.kind)
    }

    /// Check if a flag is set.
    #[inline]
    pub fn has_flag(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// Set a flag.
    #[inline]
    pub fn set_flag(&mut self, flag: u8) {
        self.flags |= flag;
    }

    /// Clear a flag.
    #[inline]
    pub fn clear_flag(&mut self, flag: u8) {
        self.flags &= !flag;
    }

    /// Byte offset of the `kind` field from the start of the header.
    pub const OFFSET_KIND: usize = 0;
    /// Byte offset of the `elem_type` field.
    pub const OFFSET_ELEM_TYPE: usize = 2;
    /// Byte offset of the `flags` field.
    pub const OFFSET_FLAGS: usize = 3;
    /// Byte offset of the `len` field.
    pub const OFFSET_LEN: usize = 4;
    /// Byte offset of the `cap` field.
    pub const OFFSET_CAP: usize = 8;
    /// Byte offset of the `aux` field.
    pub const OFFSET_AUX: usize = 16;
}

impl HeapKind {
    /// The last (highest-numbered) variant in HeapKind.
    /// IMPORTANT: Update this when adding new HeapKind variants.
    pub const MAX_VARIANT: Self = HeapKind::ProjectedRef;

    /// Convert a u16 discriminant to a HeapKind, returning None if out of range.
    #[inline]
    pub fn from_u16(v: u16) -> Option<Self> {
        if v <= Self::MAX_VARIANT as u16 {
            // Safety: HeapKind is repr(u8) with contiguous variants from 0..=MAX_VARIANT.
            // We checked the range, and u16 fits in u8 for valid values.
            Some(unsafe { std::mem::transmute(v as u8) })
        } else {
            None
        }
    }

    /// Convert a u8 discriminant to a HeapKind, returning None if out of range.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::from_u16(v as u16)
    }
}

/// Static assertion: HeapKind must be repr(u8), i.e. 1 byte.
const _: () = {
    assert!(
        std::mem::size_of::<HeapKind>() == 1,
        "HeapKind must be repr(u8) — transmute in from_u16 depends on this"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size_and_alignment() {
        assert_eq!(std::mem::size_of::<HeapHeader>(), 32);
        assert_eq!(std::mem::align_of::<HeapHeader>(), 16);
    }

    #[test]
    fn test_header_field_offsets() {
        // Verify offsets match the documented layout using offset_of!
        assert_eq!(HeapHeader::OFFSET_KIND, 0);
        assert_eq!(HeapHeader::OFFSET_ELEM_TYPE, 2);
        assert_eq!(HeapHeader::OFFSET_FLAGS, 3);
        assert_eq!(HeapHeader::OFFSET_LEN, 4);
        assert_eq!(HeapHeader::OFFSET_CAP, 8);
        assert_eq!(HeapHeader::OFFSET_AUX, 16);

        // Verify with actual struct field offsets
        let h = HeapHeader::new(HeapKind::String);
        let base = &h as *const _ as usize;
        assert_eq!(&h.kind as *const _ as usize - base, HeapHeader::OFFSET_KIND);
        assert_eq!(
            &h.elem_type as *const _ as usize - base,
            HeapHeader::OFFSET_ELEM_TYPE
        );
        assert_eq!(
            &h.flags as *const _ as usize - base,
            HeapHeader::OFFSET_FLAGS
        );
        assert_eq!(&h.len as *const _ as usize - base, HeapHeader::OFFSET_LEN);
        assert_eq!(&h.cap as *const _ as usize - base, HeapHeader::OFFSET_CAP);
        assert_eq!(&h.aux as *const _ as usize - base, HeapHeader::OFFSET_AUX);
    }

    #[test]
    fn test_new_header() {
        let h = HeapHeader::new(HeapKind::Array);
        assert_eq!(h.kind, HeapKind::Array as u16);
        assert_eq!(h.elem_type, 0);
        assert_eq!(h.flags, 0);
        assert_eq!(h.len, 0);
        assert_eq!(h.cap, 0);
        assert_eq!(h.aux, 0);
    }

    #[test]
    fn test_with_len_aux() {
        let h = HeapHeader::with_len_aux(HeapKind::TypedObject, 5, 0xDEAD_BEEF);
        assert_eq!(h.kind, HeapKind::TypedObject as u16);
        assert_eq!(h.len, 5);
        assert_eq!(h.aux, 0xDEAD_BEEF);
    }

    #[test]
    fn test_heap_kind_roundtrip() {
        assert_eq!(HeapKind::from_u16(0), Some(HeapKind::String));
        assert_eq!(HeapKind::from_u16(1), Some(HeapKind::Array));
        assert_eq!(HeapKind::from_u16(2), Some(HeapKind::TypedObject));
        assert_eq!(
            HeapKind::from_u16(HeapKind::F32Array as u16),
            Some(HeapKind::F32Array)
        );
        // Variants added after F32Array must also round-trip
        assert_eq!(
            HeapKind::from_u16(HeapKind::Set as u16),
            Some(HeapKind::Set)
        );
        assert_eq!(
            HeapKind::from_u16(HeapKind::Char as u16),
            Some(HeapKind::Char)
        );
        assert_eq!(
            HeapKind::from_u16(HeapKind::ProjectedRef as u16),
            Some(HeapKind::ProjectedRef)
        );
        // One past the last variant must return None
        assert_eq!(
            HeapKind::from_u16(HeapKind::MAX_VARIANT as u16 + 1),
            None
        );
        assert_eq!(HeapKind::from_u16(255), None);
    }

    #[test]
    fn test_heap_kind_from_u8() {
        assert_eq!(HeapKind::from_u8(0), Some(HeapKind::String));
        assert_eq!(
            HeapKind::from_u8(HeapKind::F32Array as u8),
            Some(HeapKind::F32Array)
        );
        assert_eq!(
            HeapKind::from_u8(HeapKind::ProjectedRef as u8),
            Some(HeapKind::ProjectedRef)
        );
        assert_eq!(HeapKind::from_u8(200), None);
    }

    /// Validates that every HeapKind discriminant from 0..=MAX_VARIANT round-trips
    /// through the unsafe transmute in `from_u16`. This catches holes in the enum
    /// (e.g. if someone inserts a variant mid-enum or reorders them).
    #[test]
    fn test_heap_kind_all_variants_roundtrip_through_transmute() {
        let max = HeapKind::MAX_VARIANT as u16;
        for i in 0..=max {
            let kind = HeapKind::from_u16(i)
                .unwrap_or_else(|| panic!("HeapKind::from_u16({i}) returned None — gap in contiguous repr(u8) enum"));
            assert_eq!(
                kind as u16, i,
                "HeapKind variant at discriminant {i} round-tripped to {}",
                kind as u16
            );
        }
    }

    #[test]
    fn test_flags() {
        let mut h = HeapHeader::new(HeapKind::Array);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_MARKED);
        assert!(h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_PINNED);
        assert!(h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));

        h.clear_flag(FLAG_MARKED);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));
    }

    #[test]
    fn test_from_heap_value_string() {
        let hv = HeapValue::String(std::sync::Arc::new("hello".to_string()));
        let h = HeapHeader::from_heap_value(&hv);
        assert_eq!(h.kind, HeapKind::String as u16);
        assert_eq!(h.len, 5);
    }

    #[test]
    fn test_from_heap_value_typed_object() {
        let hv = HeapValue::TypedObject {
            schema_id: 42,
            slots: vec![crate::slot::ValueSlot::from_number(0.0); 3].into_boxed_slice(),
            heap_mask: 0,
        };
        let h = HeapHeader::from_heap_value(&hv);
        assert_eq!(h.kind, HeapKind::TypedObject as u16);
        assert_eq!(h.len, 3);
        assert_eq!(h.aux, 42);
    }

    #[test]
    fn test_from_heap_value_closure() {
        let hv = HeapValue::Closure {
            function_id: 7,
            upvalues: vec![],
        };
        let h = HeapHeader::from_heap_value(&hv);
        assert_eq!(h.kind, HeapKind::Closure as u16);
        assert_eq!(h.len, 0);
        assert_eq!(h.aux, 7);
    }
}
