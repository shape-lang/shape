//! Typed closure layout for v2 runtime.
//!
//! A `TypedClosure` parallels `TypedStruct`: it has an 8-byte `HeapHeader`
//! followed by a `function_id: u32` / `type_id: u32` pair, then a compact
//! C-style capture area with compile-time-known offsets.
//!
//! ## Memory layout
//!
//! ```text
//! Heap variant (escaping closure):
//!   Offset  Size  Field
//!   ------  ----  -----
//!     0       8   HeapHeader
//!     8       4   function_id (u32)
//!    12       4   type_id (u32, ClosureTypeId.0)
//!    16+      ..  captures[] (C-laid-out per ClosureLayout)
//!
//! Stack variant (non-escaping closure, Cranelift StackSlot):
//!   Offset  Size  Field
//!   ------  ----  -----
//!     0       4   function_id (u32)
//!     4       4   type_id (u32, ClosureTypeId.0)
//!     8+      ..  captures[]
//! ```
//!
//! Captures start 8-byte aligned in both variants (HeapHeader and the
//! function_id+type_id pair are both 8 bytes). The relative offset of each
//! capture inside the captures area is the same for both variants — only
//! the preceding header differs.
//!
//! ## Keying
//!
//! `ClosureTypeId`s are minted per **capture signature** (`Vec<ConcreteType>`),
//! not per closure literal. The closure body is carried separately by
//! `function_id`. Two literals with identical captures (e.g. two `|x| x + 1`
//! expressions with no captures) share `ClosureTypeId(0)`. See
//! `docs/v2-closure-specialization.md` §1.2.

use super::concrete_type::{ClosureTypeId, ConcreteType};
use super::struct_layout::{FieldInfo, FieldKind};
use std::collections::HashMap;

/// Byte size of the heap closure header: `HeapHeader (8) + function_id (4) + type_id (4)`.
pub const HEAP_CLOSURE_HEADER_SIZE: usize = 16;

/// Byte size of the stack closure header: `function_id (4) + type_id (4)`.
pub const STACK_CLOSURE_HEADER_SIZE: usize = 8;

/// Heap-allocated closure. The `HeapHeader` is at offset 0; captures follow
/// the `function_id`/`type_id` pair at offset 16.
///
/// This is a layout marker used by JIT/VM codegen — captures are not declared
/// as Rust fields because their number and types are only known per
/// `ClosureTypeId`.
#[repr(C)]
pub struct TypedClosureHeader {
    pub header: super::heap_header::HeapHeader, // offset 0, 8 bytes
    pub function_id: u32,                       // offset 8, 4 bytes
    pub type_id: u32,                           // offset 12, 4 bytes
                                                // captures follow starting at offset 16
}

/// Stack-allocated closure. No `HeapHeader`; captures follow the
/// `function_id`/`type_id` pair at offset 8.
#[repr(C)]
pub struct StackClosure {
    pub function_id: u32, // offset 0, 4 bytes
    pub type_id: u32,     // offset 4, 4 bytes
                          // captures follow starting at offset 8
}

const _: () = {
    assert!(std::mem::size_of::<StackClosure>() == 8);
    assert!(std::mem::size_of::<TypedClosureHeader>() == 16);
};

/// Computed layout for a closure's captures.
///
/// Offsets in `captures` are relative to the **captures area start** (i.e.
/// offset 0 = first byte after the header). Use [`ClosureLayout::heap_capture_offset`]
/// or [`ClosureLayout::stack_capture_offset`] for absolute offsets from the
/// corresponding closure base pointer.
#[derive(Debug, Clone)]
pub struct ClosureLayout {
    /// The `ConcreteType` of each capture, in declaration order. Also the
    /// registry key for this layout.
    pub capture_types: Vec<ConcreteType>,
    /// Per-capture field info. `offset` is relative to the captures area start.
    pub captures: Vec<FieldInfo>,
    /// Bitmap: bit N = capture N is a heap-refcounted pointer (`Ptr`).
    /// Used by Drop glue to know which captures to release.
    pub heap_capture_mask: u64,
    /// Size in bytes of the captures area (rounded up to 8-byte alignment).
    /// Does NOT include the header.
    pub captures_size: usize,
    /// Alignment of the captures area (always 8 in practice).
    pub captures_align: usize,
}

impl ClosureLayout {
    /// Build a layout from an ordered list of capture types.
    ///
    /// Captures are laid out in declaration order with natural alignment
    /// padding, starting from offset 0 of the captures area. The total size
    /// is rounded up to 8 bytes so the whole closure object is 8-aligned.
    pub fn from_capture_types(capture_types: &[ConcreteType]) -> Self {
        if capture_types.len() > 64 {
            panic!(
                "closure has {} captures; heap_capture_mask is limited to 64 captures",
                capture_types.len()
            );
        }

        let mut current_offset: usize = 0;
        let mut captures = Vec::with_capacity(capture_types.len());
        let mut heap_mask: u64 = 0;
        let mut max_align: usize = 1;

        for (i, ty) in capture_types.iter().enumerate() {
            let kind = ty.to_field_kind();
            let align = kind.alignment();
            let size = kind.size();
            current_offset = (current_offset + align - 1) & !(align - 1);
            captures.push(FieldInfo {
                name: format!("capture_{i}"),
                kind,
                offset: current_offset,
                size,
            });
            if kind == FieldKind::Ptr {
                heap_mask |= 1u64 << i;
            }
            if align > max_align {
                max_align = align;
            }
            current_offset += size;
        }

        let captures_align = if capture_types.is_empty() { 8 } else { max_align.max(8) };
        let captures_size = (current_offset + captures_align - 1) & !(captures_align - 1);

        ClosureLayout {
            capture_types: capture_types.to_vec(),
            captures,
            heap_capture_mask: heap_mask,
            captures_size,
            captures_align,
        }
    }

    /// Number of captures.
    #[inline]
    pub fn capture_count(&self) -> usize {
        self.captures.len()
    }

    /// Offset of capture `i` from the captures area start (not from the
    /// heap / stack base pointer).
    #[inline]
    pub fn capture_offset(&self, i: usize) -> usize {
        self.captures[i].offset
    }

    /// `FieldKind` of capture `i`.
    #[inline]
    pub fn capture_kind(&self, i: usize) -> FieldKind {
        self.captures[i].kind
    }

    /// Absolute offset of capture `i` from the start of a heap-allocated
    /// `TypedClosureHeader` (i.e. add 16 for the header).
    #[inline]
    pub fn heap_capture_offset(&self, i: usize) -> usize {
        HEAP_CLOSURE_HEADER_SIZE + self.captures[i].offset
    }

    /// Absolute offset of capture `i` from the start of a `StackClosure`
    /// (i.e. add 8 for the function_id/type_id pair).
    #[inline]
    pub fn stack_capture_offset(&self, i: usize) -> usize {
        STACK_CLOSURE_HEADER_SIZE + self.captures[i].offset
    }

    /// Total size of a heap-allocated closure with this layout:
    /// `HeapHeader + function_id + type_id + captures`.
    #[inline]
    pub fn total_heap_size(&self) -> usize {
        HEAP_CLOSURE_HEADER_SIZE + self.captures_size
    }

    /// Total size of a stack-allocated closure with this layout:
    /// `function_id + type_id + captures`.
    #[inline]
    pub fn total_stack_size(&self) -> usize {
        STACK_CLOSURE_HEADER_SIZE + self.captures_size
    }

    /// Whether capture `i` is a heap-refcounted pointer.
    #[inline]
    pub fn is_heap_capture(&self, i: usize) -> bool {
        self.heap_capture_mask & (1u64 << i) != 0
    }
}

/// Registry of closure capture layouts, keyed on capture signature.
///
/// Two closures with identical capture signatures share a `ClosureTypeId`.
/// The body is identified separately by `function_id` in the program's
/// function table.
#[derive(Debug, Default, Clone)]
pub struct ClosureRegistry {
    layouts: Vec<ClosureLayout>,
    signature_to_id: HashMap<Vec<ConcreteType>, ClosureTypeId>,
}

impl ClosureRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a capture signature and return its `ClosureTypeId`. If the
    /// signature has been seen before, returns the existing id; otherwise
    /// allocates a fresh one and records the layout.
    pub fn intern(&mut self, capture_types: Vec<ConcreteType>) -> ClosureTypeId {
        if let Some(&id) = self.signature_to_id.get(&capture_types) {
            return id;
        }
        let id = ClosureTypeId(self.layouts.len() as u32);
        let layout = ClosureLayout::from_capture_types(&capture_types);
        self.layouts.push(layout);
        self.signature_to_id.insert(capture_types, id);
        id
    }

    /// Get the layout for a previously interned `ClosureTypeId`.
    pub fn get(&self, id: ClosureTypeId) -> Option<&ClosureLayout> {
        self.layouts.get(id.0 as usize)
    }

    /// Number of distinct capture signatures interned.
    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    /// Iterate over all `(ClosureTypeId, ClosureLayout)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (ClosureTypeId, &ClosureLayout)> {
        self.layouts
            .iter()
            .enumerate()
            .map(|(i, l)| (ClosureTypeId(i as u32), l))
    }

    /// Look up a `ClosureTypeId` by capture signature without interning
    /// (returns `None` if not seen before).
    pub fn lookup(&self, capture_types: &[ConcreteType]) -> Option<ClosureTypeId> {
        self.signature_to_id.get(capture_types).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::concrete_type::{ConcreteType, StructLayoutId};

    // ---- ClosureLayout layout tests ----

    #[test]
    fn test_empty_captures() {
        let layout = ClosureLayout::from_capture_types(&[]);
        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.captures_size, 0);
        assert_eq!(layout.captures_align, 8);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.total_heap_size(), 16);
        assert_eq!(layout.total_stack_size(), 8);
    }

    #[test]
    fn test_single_f64_capture() {
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::F64]);
        assert_eq!(layout.capture_count(), 1);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_kind(0), FieldKind::F64);
        assert_eq!(layout.heap_capture_offset(0), 16);
        assert_eq!(layout.stack_capture_offset(0), 8);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    #[test]
    fn test_two_f64_captures() {
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::F64, ConcreteType::F64]);
        assert_eq!(layout.capture_count(), 2);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 8);
        assert_eq!(layout.captures_size, 16);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.total_heap_size(), 32);
        assert_eq!(layout.total_stack_size(), 24);
    }

    #[test]
    fn test_single_i64_capture() {
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64]);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_kind(0), FieldKind::I64);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    #[test]
    fn test_mixed_f64_i32_ptr() {
        // (F64, I32, String) — String is a heap pointer.
        // f64 @ 0  (size 8)
        // i32 @ 8  (size 4)
        // ptr @ 16 (needs 8-align from offset 12, pad to 16; size 8)
        // captures_size = 24
        let layout = ClosureLayout::from_capture_types(&[
            ConcreteType::F64,
            ConcreteType::I32,
            ConcreteType::String,
        ]);
        assert_eq!(layout.capture_count(), 3);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 8);
        assert_eq!(layout.capture_offset(2), 16);
        assert_eq!(layout.capture_kind(0), FieldKind::F64);
        assert_eq!(layout.capture_kind(1), FieldKind::I32);
        assert_eq!(layout.capture_kind(2), FieldKind::Ptr);
        assert_eq!(layout.captures_size, 24);
        assert_eq!(layout.heap_capture_mask, 0b100);
        assert!(layout.is_heap_capture(2));
        assert!(!layout.is_heap_capture(0));
        assert!(!layout.is_heap_capture(1));
        assert_eq!(layout.total_heap_size(), 40);
        assert_eq!(layout.total_stack_size(), 32);
    }

    #[test]
    fn test_single_heap_typed_capture_string() {
        // Single String (Ptr) capture: captures area = 8 bytes, mask bit 0 set.
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String]);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.heap_capture_mask, 0b1);
        assert!(layout.is_heap_capture(0));
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    #[test]
    fn test_array_capture_is_heap() {
        // Array<int> is a heap-typed pointer.
        let arr = ConcreteType::Array(Box::new(ConcreteType::I64));
        let layout = ClosureLayout::from_capture_types(&[arr]);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b1);
    }

    #[test]
    fn test_struct_capture_is_heap() {
        let s = ConcreteType::Struct(StructLayoutId(42));
        let layout = ClosureLayout::from_capture_types(&[s]);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b1);
    }

    #[test]
    fn test_small_field_packing() {
        // (Bool, I8, I16, I32) — small fields pack tightly.
        // bool @ 0 (size 1)
        // i8   @ 1 (size 1)
        // i16  @ 2 (size 2)  — 2 is already 2-aligned
        // i32  @ 4 (size 4)  — 4 is 4-aligned
        // captures_size = 8 (rounded up to 8)
        let layout = ClosureLayout::from_capture_types(&[
            ConcreteType::Bool,
            ConcreteType::I8,
            ConcreteType::I16,
            ConcreteType::I32,
        ]);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 1);
        assert_eq!(layout.capture_offset(2), 2);
        assert_eq!(layout.capture_offset(3), 4);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.heap_capture_mask, 0);
    }

    #[test]
    fn test_heap_mask_positions() {
        // (I32, String, F64, Array<F64>) → Ptr at positions 1 and 3.
        let arr = ConcreteType::Array(Box::new(ConcreteType::F64));
        let layout = ClosureLayout::from_capture_types(&[
            ConcreteType::I32,
            ConcreteType::String,
            ConcreteType::F64,
            arr,
        ]);
        assert_eq!(layout.heap_capture_mask, 0b1010);
        assert!(!layout.is_heap_capture(0));
        assert!(layout.is_heap_capture(1));
        assert!(!layout.is_heap_capture(2));
        assert!(layout.is_heap_capture(3));
    }

    #[test]
    fn test_offsets_relative_and_absolute_agree() {
        let layout = ClosureLayout::from_capture_types(&[
            ConcreteType::F64,
            ConcreteType::I64,
            ConcreteType::String,
        ]);
        for i in 0..layout.capture_count() {
            assert_eq!(layout.heap_capture_offset(i), 16 + layout.capture_offset(i));
            assert_eq!(layout.stack_capture_offset(i), 8 + layout.capture_offset(i));
        }
    }

    #[test]
    fn test_size_rounded_up_for_trailing_small_field() {
        // Single Bool: 1 byte, rounded up to 8.
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::Bool]);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    // ---- ClosureRegistry tests ----

    #[test]
    fn test_registry_empty() {
        let r = ClosureRegistry::new();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn test_registry_same_signature_returns_same_id() {
        let mut r = ClosureRegistry::new();
        let id_a = r.intern(vec![ConcreteType::I64]);
        let id_b = r.intern(vec![ConcreteType::I64]);
        assert_eq!(id_a, id_b);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn test_registry_different_signatures_returns_different_ids() {
        let mut r = ClosureRegistry::new();
        let id_empty = r.intern(vec![]);
        let id_i64 = r.intern(vec![ConcreteType::I64]);
        let id_f64 = r.intern(vec![ConcreteType::F64]);
        let id_i64_f64 = r.intern(vec![ConcreteType::I64, ConcreteType::F64]);
        let id_f64_i64 = r.intern(vec![ConcreteType::F64, ConcreteType::I64]);

        assert_ne!(id_empty, id_i64);
        assert_ne!(id_i64, id_f64);
        assert_ne!(id_i64_f64, id_f64_i64, "order matters in the signature");
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn test_registry_roundtrip_and_layout_retrieval() {
        let mut r = ClosureRegistry::new();
        let id = r.intern(vec![ConcreteType::F64, ConcreteType::String]);
        let layout = r.get(id).expect("layout should exist");
        assert_eq!(layout.capture_count(), 2);
        assert_eq!(layout.capture_kind(0), FieldKind::F64);
        assert_eq!(layout.capture_kind(1), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b10);
    }

    #[test]
    fn test_registry_lookup_without_intern() {
        let mut r = ClosureRegistry::new();
        assert_eq!(r.lookup(&[ConcreteType::I64]), None);
        let id = r.intern(vec![ConcreteType::I64]);
        assert_eq!(r.lookup(&[ConcreteType::I64]), Some(id));
        assert_eq!(r.lookup(&[ConcreteType::F64]), None);
    }

    #[test]
    fn test_registry_iter() {
        let mut r = ClosureRegistry::new();
        r.intern(vec![]);
        r.intern(vec![ConcreteType::I64]);
        r.intern(vec![ConcreteType::F64]);
        let collected: Vec<_> = r.iter().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0].0, ClosureTypeId(0));
        assert_eq!(collected[1].0, ClosureTypeId(1));
        assert_eq!(collected[2].0, ClosureTypeId(2));
    }

    #[test]
    fn test_registry_ids_are_sequential_from_zero() {
        let mut r = ClosureRegistry::new();
        let a = r.intern(vec![ConcreteType::I64]);
        let b = r.intern(vec![ConcreteType::F64]);
        let c = r.intern(vec![ConcreteType::Bool]);
        assert_eq!(a, ClosureTypeId(0));
        assert_eq!(b, ClosureTypeId(1));
        assert_eq!(c, ClosureTypeId(2));
    }

    #[test]
    fn test_registry_nested_types_are_distinct() {
        let mut r = ClosureRegistry::new();
        let arr_i64 = ConcreteType::Array(Box::new(ConcreteType::I64));
        let arr_f64 = ConcreteType::Array(Box::new(ConcreteType::F64));
        let id1 = r.intern(vec![arr_i64]);
        let id2 = r.intern(vec![arr_f64]);
        assert_ne!(id1, id2);
    }

    // ---- Compile-time size / repr checks ----

    #[test]
    fn test_sizeof_stack_closure_is_8() {
        assert_eq!(std::mem::size_of::<StackClosure>(), 8);
    }

    #[test]
    fn test_sizeof_typed_closure_header_is_16() {
        assert_eq!(std::mem::size_of::<TypedClosureHeader>(), 16);
    }

    #[test]
    fn test_header_constants() {
        assert_eq!(HEAP_CLOSURE_HEADER_SIZE, 16);
        assert_eq!(STACK_CLOSURE_HEADER_SIZE, 8);
    }
}
