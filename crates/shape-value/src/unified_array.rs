//! Unified array representation with `#[repr(C)]` layout.
//!
//! `UnifiedArray` is a C-ABI-compatible array that can be used by both the
//! VM and JIT without conversion. It shares the same field layout as the
//! JIT's `JitArray` but adds a unified heap header prefix (kind, flags,
//! refcount) at offset 0, pushing data fields to offset 8.
//!
//! ## Memory layout (64 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       2   kind              (HEAP_KIND_ARRAY as u16)
//!   2       1   flags
//!   3       1   _reserved
//!   4       4   refcount          (AtomicU32)
//!   8       8   data              (*mut u64, boxed element buffer)
//!  16       8   len               (u64, element count)
//!  24       8   cap               (u64, allocated capacity)
//!  32       8   typed_data        (*mut u64, optional typed payload mirror)
//!  40       1   element_kind      (ArrayElementKind tag)
//!  41       1   typed_storage_kind
//!  42       6   _padding
//!  48       8   slice_parent_arc  (*const (), leaked Arc for FloatArraySlice)
//!  56       4   slice_offset      (u32)
//!  60       4   slice_len         (u32)
//! ```

use crate::tags;
use crate::value_word::ValueWordExt;
use std::alloc::{self, Layout};
use std::slice;
use std::sync::atomic::AtomicU32;

// ── Offset constants ────────────────────────────────────────────────────────

pub const UA_KIND_OFFSET: i32 = 0;
pub const UA_FLAGS_OFFSET: i32 = 2;
pub const UA_REFCOUNT_OFFSET: i32 = 4;
pub const UA_DATA_OFFSET: i32 = 8;
pub const UA_LEN_OFFSET: i32 = 16;
pub const UA_CAP_OFFSET: i32 = 24;
pub const UA_TYPED_DATA_OFFSET: i32 = 32;
pub const UA_ELEMENT_KIND_OFFSET: i32 = 40;

// ── Flags ───────────────────────────────────────────────────────────────────

/// Flag: this array owns its element buffer and should free it on drop.
pub const UAF_OWNS_ELEMENTS: u8 = 0b0000_0001;

// ── ArrayElementKind ────────────────────────────────────────────────────────

/// Typed element kind for arrays, enabling typed fast paths in the JIT.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayElementKind {
    Untyped = 0,
    Float64 = 1,
    Int64 = 2,
    Bool = 3,
    I8 = 4,
    I16 = 5,
    I32 = 6,
    U8 = 7,
    U16 = 8,
    U32 = 9,
    U64 = 10,
    F32 = 11,
}

impl ArrayElementKind {
    #[inline]
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Float64,
            2 => Self::Int64,
            3 => Self::Bool,
            4 => Self::I8,
            5 => Self::I16,
            6 => Self::I32,
            7 => Self::U8,
            8 => Self::U16,
            9 => Self::U32,
            10 => Self::U64,
            11 => Self::F32,
            _ => Self::Untyped,
        }
    }

    #[inline]
    pub const fn as_byte(self) -> u8 {
        self as u8
    }
}

// ── UnifiedArray ────────────────────────────────────────────────────────────

/// Unified array representation shared between VM and JIT.
///
/// Uses `#[repr(C)]` for ABI-stable field access from generated code.
#[repr(C)]
pub struct UnifiedArray {
    /// Heap kind discriminator (HEAP_KIND_ARRAY).
    pub kind: u16,
    /// Bitfield flags (UAF_OWNS_ELEMENTS, etc.).
    pub flags: u8,
    /// Reserved byte for future use.
    pub _reserved: u8,
    /// Reference count for shared ownership.
    pub refcount: AtomicU32,
    /// Pointer to boxed element buffer (heap-allocated u64 values).
    pub data: *mut u64,
    /// Number of elements currently stored.
    pub len: u64,
    /// Allocated capacity (number of u64 elements).
    pub cap: u64,
    /// Optional typed payload buffer (mirrors `data` indices).
    pub typed_data: *mut u64,
    /// `ArrayElementKind` tag byte.
    pub element_kind: u8,
    /// Allocation layout kind for `typed_data`.
    pub typed_storage_kind: u8,
    /// Explicit padding for stable layout.
    pub _padding: [u8; 6],
    /// For FloatArraySlice round-trip: leaked `Arc<MatrixData>` pointer.
    pub slice_parent_arc: *const (),
    /// Row offset into parent matrix data (for FloatArraySlice).
    pub slice_offset: u32,
    /// Element count of the slice (for FloatArraySlice).
    pub slice_len: u32,
}

// Safety: UnifiedArray is a raw data structure with manual memory management.
// Send/Sync are safe because the array contents are NaN-boxed u64 values
// (not Rust references), and the refcount uses AtomicU32.
unsafe impl Send for UnifiedArray {}
unsafe impl Sync for UnifiedArray {}

// ── Compile-time layout assertions ──────────────────────────────────────────

const _: () = {
    assert!(std::mem::size_of::<UnifiedArray>() == 64);
    assert!(std::mem::offset_of!(UnifiedArray, kind) == 0);
    assert!(std::mem::offset_of!(UnifiedArray, flags) == 2);
    assert!(std::mem::offset_of!(UnifiedArray, _reserved) == 3);
    assert!(std::mem::offset_of!(UnifiedArray, refcount) == 4);
    assert!(std::mem::offset_of!(UnifiedArray, data) == 8);
    assert!(std::mem::offset_of!(UnifiedArray, len) == 16);
    assert!(std::mem::offset_of!(UnifiedArray, cap) == 24);
    assert!(std::mem::offset_of!(UnifiedArray, typed_data) == 32);
    assert!(std::mem::offset_of!(UnifiedArray, element_kind) == 40);
    assert!(std::mem::offset_of!(UnifiedArray, typed_storage_kind) == 41);
    assert!(std::mem::offset_of!(UnifiedArray, _padding) == 42);
    assert!(std::mem::offset_of!(UnifiedArray, slice_parent_arc) == 48);
    assert!(std::mem::offset_of!(UnifiedArray, slice_offset) == 56);
    assert!(std::mem::offset_of!(UnifiedArray, slice_len) == 60);
};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Check if a NaN-boxed f64 bit pattern represents a plain f64 number.
#[inline]
fn is_number(bits: u64) -> bool {
    !tags::is_tagged(bits)
}

/// Reinterpret a NaN-boxed u64 as an f64.
#[inline]
fn unbox_number(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// NaN-boxed boolean true.
const TAG_BOOL_TRUE: u64 =
    tags::TAG_BASE | (tags::TAG_BOOL << tags::TAG_SHIFT) | 1;

/// NaN-boxed boolean false.
const TAG_BOOL_FALSE: u64 =
    tags::TAG_BASE | (tags::TAG_BOOL << tags::TAG_SHIFT);

impl UnifiedArray {
    // ── Constructors ────────────────────────────────────────────────────

    /// Create an empty array.
    pub fn new() -> Self {
        Self {
            kind: tags::HEAP_KIND_ARRAY as u16,
            flags: 0,
            _reserved: 0,
            refcount: AtomicU32::new(1),
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
            slice_parent_arc: std::ptr::null(),
            slice_offset: 0,
            slice_len: 0,
        }
    }

    /// Create an array with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        if cap == 0 {
            return Self::new();
        }
        let data = Self::alloc_u64_buffer(cap);
        Self {
            kind: tags::HEAP_KIND_ARRAY as u16,
            flags: 0,
            _reserved: 0,
            refcount: AtomicU32::new(1),
            data,
            len: 0,
            cap: cap as u64,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
            slice_parent_arc: std::ptr::null(),
            slice_offset: 0,
            slice_len: 0,
        }
    }

    /// Create an array by copying from a slice.
    pub fn from_slice(elements: &[u64]) -> Self {
        if elements.is_empty() {
            return Self::new();
        }

        let cap = elements.len();
        let data = Self::alloc_u64_buffer(cap);
        unsafe {
            std::ptr::copy_nonoverlapping(elements.as_ptr(), data, elements.len());
        }

        let mut arr = Self {
            kind: tags::HEAP_KIND_ARRAY as u16,
            flags: 0,
            _reserved: 0,
            refcount: AtomicU32::new(1),
            data,
            len: elements.len() as u64,
            cap: cap as u64,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
            slice_parent_arc: std::ptr::null(),
            slice_offset: 0,
            slice_len: 0,
        };
        arr.initialize_typed_from_boxed(elements);
        arr
    }

    /// Create an array from an owned `Vec<u64>` (takes ownership of the data).
    pub fn from_vec(vec: Vec<u64>) -> Self {
        if vec.is_empty() {
            return Self::new();
        }

        let mut boxed = vec.into_boxed_slice();
        let len = boxed.len();
        let cap = len;
        let data = boxed.as_mut_ptr();
        std::mem::forget(boxed);

        let mut arr = Self {
            kind: tags::HEAP_KIND_ARRAY as u16,
            flags: 0,
            _reserved: 0,
            refcount: AtomicU32::new(1),
            data,
            len: len as u64,
            cap: cap as u64,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
            slice_parent_arc: std::ptr::null(),
            slice_offset: 0,
            slice_len: 0,
        };

        let elements = unsafe { slice::from_raw_parts(data, len) };
        arr.initialize_typed_from_boxed(elements);
        arr
    }

    /// Builder: set the heap kind to a typed array variant.
    ///
    /// E.g., `UnifiedArray::new().with_kind(tags::HEAP_KIND_INT_ARRAY as u16)`
    #[inline]
    pub fn with_kind(mut self, kind: u16) -> Self {
        self.kind = kind;
        self
    }

    // ── NaN-boxing ────────────────────────────────────────────────────────

    /// Extract the raw pointer from NaN-boxed TAG_HEAP bits.
    ///
    /// Handles both VM format (bit-47 set, uses `unified_heap_ptr`) and
    /// JIT format (no bit-47, uses `PAYLOAD_MASK`).
    #[inline]
    fn extract_ptr(bits: u64) -> *const Self {
        if tags::is_unified_heap(bits) {
            tags::unified_heap_ptr(bits) as *const Self
        } else {
            (bits & tags::PAYLOAD_MASK) as *const Self
        }
    }

    /// Box this array into a NaN-boxed TAG_HEAP u64 (JIT format, NO bit 47).
    ///
    /// Used by JIT-created arrays. The JIT manages these independently.
    #[inline]
    pub fn heap_box(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        tags::TAG_BASE | ((ptr as u64) & tags::PAYLOAD_MASK)
    }

    /// Box this array into a NaN-boxed TAG_HEAP u64 (VM format, bit 47 set).
    ///
    /// Used by `ValueWord::from_array()`. ValueWord Clone/Drop manage the refcount.
    #[inline]
    pub fn heap_box_unified(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        tags::make_unified_heap(ptr as *const u8)
    }

    /// Get a reference from NaN-boxed TAG_HEAP bits (any format).
    ///
    /// # Safety
    /// `bits` must be a valid TAG_HEAP value pointing to a live UnifiedArray.
    #[inline]
    pub unsafe fn from_heap_bits(bits: u64) -> &'static Self {
        let ptr = Self::extract_ptr(bits);
        unsafe { &*ptr }
    }

    /// Get a mutable reference from NaN-boxed TAG_HEAP bits (any format).
    ///
    /// # Safety
    /// `bits` must be a valid TAG_HEAP value pointing to a live UnifiedArray.
    /// Caller must ensure exclusive access.
    #[inline]
    pub unsafe fn from_heap_bits_mut(bits: u64) -> &'static mut Self {
        let ptr = Self::extract_ptr(bits) as *mut Self;
        unsafe { &mut *ptr }
    }

    /// Drop a heap-boxed UnifiedArray from its NaN-boxed bits (any format).
    ///
    /// # Safety
    /// Must only be called once per allocation.
    pub unsafe fn heap_drop(bits: u64) {
        let ptr = Self::extract_ptr(bits) as *mut Self;
        unsafe { drop(Box::from_raw(ptr)) };
    }

    // ── Accessors ───────────────────────────────────────────────────────

    /// Number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// View elements as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[u64] {
        if self.data.is_null() || self.len == 0 {
            return &[];
        }
        unsafe { slice::from_raw_parts(self.data, self.len as usize) }
    }

    /// View elements as a mutable slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u64] {
        if self.data.is_null() || self.len == 0 {
            return &mut [];
        }
        unsafe { slice::from_raw_parts_mut(self.data, self.len as usize) }
    }

    /// Get element by index (bounds-checked).
    #[inline]
    pub fn get(&self, index: usize) -> Option<&u64> {
        if index < self.len as usize {
            unsafe { Some(&*self.data.add(index)) }
        } else {
            None
        }
    }

    /// Get first element.
    #[inline]
    pub fn first(&self) -> Option<&u64> {
        if self.len > 0 {
            unsafe { Some(&*self.data) }
        } else {
            None
        }
    }

    /// Get last element.
    #[inline]
    pub fn last(&self) -> Option<&u64> {
        if self.len > 0 {
            unsafe { Some(&*self.data.add(self.len as usize - 1)) }
        } else {
            None
        }
    }

    /// Iterate over elements.
    #[inline]
    pub fn iter(&self) -> slice::Iter<'_, u64> {
        self.as_slice().iter()
    }

    /// Raw pointer to data buffer.
    #[inline]
    pub fn as_ptr(&self) -> *const u64 {
        self.data
    }

    /// Element kind accessor.
    #[inline]
    pub fn element_kind(&self) -> ArrayElementKind {
        ArrayElementKind::from_byte(self.element_kind)
    }

    /// Typed data pointer accessor.
    #[inline]
    pub fn typed_data_ptr(&self) -> *const u64 {
        self.typed_data
    }

    // ── Mutation ────────────────────────────────────────────────────────

    /// Set an element by index (bounds-checked).
    /// Returns true when the write succeeded.
    pub fn set_boxed(&mut self, index: usize, value: u64) -> bool {
        if index >= self.len as usize {
            return false;
        }
        unsafe {
            *self.data.add(index) = value;
        }
        self.update_typed_on_write(index, value);
        true
    }

    /// Push an element (amortized O(1) with doubling growth).
    pub fn push(&mut self, value: u64) {
        if self.len == self.cap {
            self.grow();
        }
        let index = self.len as usize;
        unsafe {
            *self.data.add(index) = value;
        }
        self.update_typed_on_write(index, value);
        self.len += 1;
    }

    /// Pop the last element.
    pub fn pop(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        unsafe { Some(*self.data.add(self.len as usize)) }
    }

    /// Ensure capacity is at least `min_capacity` elements.
    pub fn reserve(&mut self, min_capacity: usize) {
        if min_capacity <= self.cap as usize {
            return;
        }
        let mut new_cap = if self.cap == 0 {
            4usize
        } else {
            self.cap as usize
        };
        while new_cap < min_capacity {
            new_cap = new_cap.saturating_mul(2);
        }
        self.grow_to(new_cap);
    }

    /// Deep copy of element buffer.
    pub fn clone_data(&self) -> Self {
        Self::from_slice(self.as_slice())
    }

    /// Convert to Vec<u64> for interop with remaining Rust code paths.
    pub fn into_vec(self) -> Vec<u64> {
        let vec = self.as_slice().to_vec();
        vec
    }

    // ── Internal: memory management ─────────────────────────────────────

    #[inline]
    fn alloc_u64_buffer(cap: usize) -> *mut u64 {
        let layout = Layout::array::<u64>(cap).unwrap();
        let data = unsafe { alloc::alloc(layout) as *mut u64 };
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        data
    }

    #[inline]
    fn realloc_u64_buffer(ptr: *mut u64, old_cap: usize, new_cap: usize) -> *mut u64 {
        let old_layout = Layout::array::<u64>(old_cap).unwrap();
        let new_layout = Layout::array::<u64>(new_cap).unwrap();
        let data =
            unsafe { alloc::realloc(ptr as *mut u8, old_layout, new_layout.size()) as *mut u64 };
        if data.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        data
    }

    #[inline]
    fn dealloc_u64_buffer(ptr: *mut u64, cap: usize) {
        let layout = Layout::array::<u64>(cap).unwrap();
        unsafe {
            alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    // ── Internal: typed buffer management ───────────────────────────────

    #[inline]
    fn typed_layout(kind: ArrayElementKind, cap: usize) -> Option<Layout> {
        if cap == 0 {
            return None;
        }
        match kind {
            ArrayElementKind::Untyped => None,
            ArrayElementKind::Bool | ArrayElementKind::I8 | ArrayElementKind::U8 => {
                Layout::array::<u8>(cap.div_ceil(if kind == ArrayElementKind::Bool { 8 } else { 1 })).ok()
            }
            ArrayElementKind::I16 | ArrayElementKind::U16 => Layout::array::<u16>(cap).ok(),
            ArrayElementKind::I32 | ArrayElementKind::U32 | ArrayElementKind::F32 => {
                Layout::array::<u32>(cap).ok()
            }
            ArrayElementKind::Float64 | ArrayElementKind::Int64 | ArrayElementKind::U64 => {
                Layout::array::<u64>(cap).ok()
            }
        }
    }

    #[inline]
    fn alloc_typed_buffer(kind: ArrayElementKind, cap: usize) -> *mut u64 {
        let Some(layout) = Self::typed_layout(kind, cap) else {
            return std::ptr::null_mut();
        };
        let data = unsafe { alloc::alloc(layout) } as *mut u64;
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        data
    }

    #[inline]
    fn realloc_typed_buffer(
        ptr: *mut u64,
        kind: ArrayElementKind,
        old_cap: usize,
        new_cap: usize,
    ) -> *mut u64 {
        let old_layout = Self::typed_layout(kind, old_cap)
            .expect("typed_layout must exist for old typed allocation");
        let new_layout = Self::typed_layout(kind, new_cap)
            .expect("typed_layout must exist for new typed allocation");
        let data =
            unsafe { alloc::realloc(ptr as *mut u8, old_layout, new_layout.size()) } as *mut u64;
        if data.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        data
    }

    #[inline]
    fn dealloc_typed_buffer(ptr: *mut u64, kind: ArrayElementKind, cap: usize) {
        if ptr.is_null() {
            return;
        }
        if let Some(layout) = Self::typed_layout(kind, cap) {
            unsafe {
                alloc::dealloc(ptr as *mut u8, layout);
            }
        }
    }

    #[inline]
    fn kind_field(&self) -> ArrayElementKind {
        ArrayElementKind::from_byte(self.element_kind)
    }

    #[inline]
    fn set_kind_field(&mut self, kind: ArrayElementKind) {
        self.element_kind = kind.as_byte();
    }

    #[inline]
    fn typed_storage_kind_field(&self) -> ArrayElementKind {
        ArrayElementKind::from_byte(self.typed_storage_kind)
    }

    // ── Internal: typed element tracking ────────────────────────────────

    #[inline]
    fn try_number_to_i64(bits: u64) -> Option<i64> {
        if !is_number(bits) {
            return None;
        }
        let n = unbox_number(bits);
        if !n.is_finite() || n < i64::MIN as f64 || n > i64::MAX as f64 {
            return None;
        }
        let i = n as i64;
        if (i as f64) == n { Some(i) } else { None }
    }

    fn infer_kind(elements: &[u64]) -> ArrayElementKind {
        if elements.is_empty() {
            return ArrayElementKind::Untyped;
        }

        if elements
            .iter()
            .all(|&v| v == TAG_BOOL_TRUE || v == TAG_BOOL_FALSE)
        {
            return ArrayElementKind::Bool;
        }

        let all_numbers = elements.iter().all(|&v| is_number(v));
        if !all_numbers {
            return ArrayElementKind::Untyped;
        }

        if elements
            .iter()
            .all(|&v| Self::try_number_to_i64(v).is_some())
        {
            ArrayElementKind::Int64
        } else {
            ArrayElementKind::Float64
        }
    }

    fn bootstrap_kind_from_first_value(value: u64) -> ArrayElementKind {
        if value == TAG_BOOL_TRUE || value == TAG_BOOL_FALSE {
            ArrayElementKind::Bool
        } else if is_number(value) {
            ArrayElementKind::Float64
        } else {
            ArrayElementKind::Untyped
        }
    }

    fn ensure_typed_buffer(&mut self, kind: ArrayElementKind) {
        if self.cap == 0 || kind == ArrayElementKind::Untyped {
            return;
        }
        if self.typed_data.is_null() {
            self.typed_data = Self::alloc_typed_buffer(kind, self.cap as usize);
            self.typed_storage_kind = kind.as_byte();
            return;
        }
        let current = self.typed_storage_kind_field();
        if current != kind {
            Self::dealloc_typed_buffer(self.typed_data, current, self.cap as usize);
            self.typed_data = Self::alloc_typed_buffer(kind, self.cap as usize);
            self.typed_storage_kind = kind.as_byte();
        }
    }

    fn write_typed_slot(&mut self, index: usize, boxed_value: u64) -> bool {
        if self.typed_data.is_null() || index >= self.cap as usize {
            return false;
        }

        let kind = self.kind_field();
        let raw = match kind {
            ArrayElementKind::Untyped => return false,
            ArrayElementKind::Float64 => {
                if !is_number(boxed_value) {
                    return false;
                }
                boxed_value
            }
            ArrayElementKind::Int64 => match Self::try_number_to_i64(boxed_value) {
                Some(v) => v as u64,
                None => return false,
            },
            ArrayElementKind::Bool => {
                if boxed_value == TAG_BOOL_TRUE {
                    1
                } else if boxed_value == TAG_BOOL_FALSE {
                    0
                } else {
                    return false;
                }
            }
            ArrayElementKind::I8 | ArrayElementKind::I16 | ArrayElementKind::I32
            | ArrayElementKind::U8 | ArrayElementKind::U16 | ArrayElementKind::U32
            | ArrayElementKind::U64 | ArrayElementKind::F32 => {
                if !is_number(boxed_value) {
                    return false;
                }
                let f = unbox_number(boxed_value);
                f as i64 as u64
            }
        };

        match kind {
            ArrayElementKind::Bool => {
                let byte_idx = index >> 3;
                let bit_idx = (index & 7) as u8;
                let mask = 1u8 << bit_idx;
                let byte_ptr = self.typed_data as *mut u8;
                unsafe {
                    let prev = *byte_ptr.add(byte_idx);
                    let next = if raw == 0 { prev & !mask } else { prev | mask };
                    *byte_ptr.add(byte_idx) = next;
                }
                true
            }
            _ => {
                unsafe {
                    *self.typed_data.add(index) = raw;
                }
                true
            }
        }
    }

    fn initialize_typed_from_boxed(&mut self, elements: &[u64]) {
        let kind = Self::infer_kind(elements);
        if kind == ArrayElementKind::Untyped {
            self.set_kind_field(ArrayElementKind::Untyped);
            return;
        }

        self.ensure_typed_buffer(kind);
        if self.typed_data.is_null() {
            self.set_kind_field(ArrayElementKind::Untyped);
            return;
        }

        self.set_kind_field(kind);
        for (idx, &value) in elements.iter().enumerate() {
            if !self.write_typed_slot(idx, value) {
                self.set_kind_field(ArrayElementKind::Untyped);
                return;
            }
        }
    }

    fn update_typed_on_write(&mut self, index: usize, boxed_value: u64) {
        let kind = self.kind_field();

        if kind == ArrayElementKind::Untyped {
            if self.len == 0 && index == 0 {
                let bootstrap = Self::bootstrap_kind_from_first_value(boxed_value);
                if bootstrap != ArrayElementKind::Untyped {
                    self.ensure_typed_buffer(bootstrap);
                    if !self.typed_data.is_null() {
                        self.set_kind_field(bootstrap);
                        if !self.write_typed_slot(index, boxed_value) {
                            self.set_kind_field(ArrayElementKind::Untyped);
                        }
                    }
                }
            }
            return;
        }

        if !self.write_typed_slot(index, boxed_value) {
            self.set_kind_field(ArrayElementKind::Untyped);
        }
    }

    /// Grow the buffer using amortized doubling.
    fn grow(&mut self) {
        let new_cap = if self.cap == 0 { 4 } else { self.cap * 2 };
        self.grow_to(new_cap as usize);
    }

    /// Reallocate element storage to `new_cap` entries.
    fn grow_to(&mut self, new_cap: usize) {
        let old_cap = self.cap as usize;

        self.data = if self.data.is_null() {
            Self::alloc_u64_buffer(new_cap)
        } else {
            Self::realloc_u64_buffer(self.data, old_cap, new_cap)
        };

        if !self.typed_data.is_null() {
            let typed_kind = self.typed_storage_kind_field();
            self.typed_data = if old_cap == 0 {
                Self::alloc_typed_buffer(typed_kind, new_cap)
            } else {
                Self::realloc_typed_buffer(self.typed_data, typed_kind, old_cap, new_cap)
            };
        }

        self.cap = new_cap as u64;
    }
}

// ── Index access ────────────────────────────────────────────────────────────

impl std::ops::Index<usize> for UnifiedArray {
    type Output = u64;

    #[inline]
    fn index(&self, index: usize) -> &u64 {
        assert!(index < self.len as usize, "UnifiedArray index out of bounds");
        unsafe { &*self.data.add(index) }
    }
}

impl std::ops::IndexMut<usize> for UnifiedArray {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut u64 {
        assert!(index < self.len as usize, "UnifiedArray index out of bounds");
        unsafe { &mut *self.data.add(index) }
    }
}

// ── Drop ────────────────────────────────────────────────────────────────────

impl Drop for UnifiedArray {
    fn drop(&mut self) {
        // Drop owned elements' refcounts before freeing the buffer.
        if self.flags & UAF_OWNS_ELEMENTS != 0 && !self.data.is_null() {
            for i in 0..self.len as usize {
                let elem_bits = unsafe { *self.data.add(i) };
                if tags::is_tagged(elem_bits) && tags::get_tag(elem_bits) == tags::TAG_HEAP {
                    if tags::is_unified_heap(elem_bits) {
                        let ptr = tags::unified_heap_ptr(elem_bits);
                        if !ptr.is_null() {
                            let rc = unsafe {
                                (ptr.add(4)) as *const std::sync::atomic::AtomicU32
                            };
                            let prev = unsafe {
                                (*rc).fetch_sub(1, std::sync::atomic::Ordering::Release)
                            };
                            if prev == 1 {
                                std::sync::atomic::fence(
                                    std::sync::atomic::Ordering::Acquire,
                                );
                                let kind = unsafe { *(ptr as *const u16) };
                                if kind == tags::HEAP_KIND_ARRAY as u16 {
                                    unsafe { UnifiedArray::heap_drop(elem_bits) };
                                } else if kind == tags::HEAP_KIND_MATRIX as u16 {
                                    unsafe {
                                        crate::unified_matrix::UnifiedMatrix::heap_drop(
                                            elem_bits,
                                        )
                                    };
                                }
                            }
                        }
                    } else {
                        let ptr =
                            tags::get_payload(elem_bits) as *const crate::heap_value::HeapValue;
                        if !ptr.is_null() {
                            unsafe { std::sync::Arc::decrement_strong_count(ptr) };
                        }
                    }
                }
            }
        }
        // Free the element data buffer.
        if !self.data.is_null() && self.cap > 0 {
            Self::dealloc_u64_buffer(self.data, self.cap as usize);
        }
        // Free the typed data buffer.
        if !self.typed_data.is_null() && self.cap > 0 {
            let typed_kind = self.typed_storage_kind_field();
            Self::dealloc_typed_buffer(self.typed_data, typed_kind, self.cap as usize);
        }
        // Drop the leaked Arc<MatrixData> if this was a FloatArraySlice.
        if !self.slice_parent_arc.is_null() {
            unsafe {
                let _ = std::sync::Arc::from_raw(
                    self.slice_parent_arc as *const crate::heap_value::MatrixData,
                );
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_size() {
        assert_eq!(std::mem::size_of::<UnifiedArray>(), 64);
    }

    #[test]
    fn test_layout_offsets() {
        assert_eq!(std::mem::offset_of!(UnifiedArray, kind), UA_KIND_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, flags), UA_FLAGS_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, refcount), UA_REFCOUNT_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, data), UA_DATA_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, len), UA_LEN_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, cap), UA_CAP_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, typed_data), UA_TYPED_DATA_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(UnifiedArray, element_kind), UA_ELEMENT_KIND_OFFSET as usize);
    }

    #[test]
    fn test_new_empty() {
        let arr = UnifiedArray::new();
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        let empty: &[u64] = &[];
        assert_eq!(arr.as_slice(), empty);
        assert_eq!(arr.element_kind(), ArrayElementKind::Untyped);
        assert_eq!(arr.kind, tags::HEAP_KIND_ARRAY as u16);
    }

    #[test]
    fn test_from_slice() {
        let arr = UnifiedArray::from_slice(&[1u64, 2, 3]);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.as_slice(), &[1u64, 2, 3]);
    }

    #[test]
    fn test_from_vec() {
        let arr = UnifiedArray::from_vec(vec![10, 20, 30]);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.as_slice(), &[10, 20, 30]);
    }

    #[test]
    fn test_push_pop() {
        let mut arr = UnifiedArray::new();
        arr.push(1);
        arr.push(2);
        arr.push(3);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.as_slice(), &[1, 2, 3]);

        assert_eq!(arr.pop(), Some(3));
        assert_eq!(arr.pop(), Some(2));
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.pop(), Some(1));
        assert_eq!(arr.pop(), None);
    }

    #[test]
    fn test_get() {
        let arr = UnifiedArray::from_slice(&[10, 20, 30]);
        assert_eq!(arr.get(0), Some(&10));
        assert_eq!(arr.get(2), Some(&30));
        assert_eq!(arr.get(3), None);
    }

    #[test]
    fn test_first_last() {
        let arr = UnifiedArray::from_slice(&[10, 20, 30]);
        assert_eq!(arr.first(), Some(&10));
        assert_eq!(arr.last(), Some(&30));

        let empty = UnifiedArray::new();
        assert_eq!(empty.first(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn test_clone_data() {
        let arr = UnifiedArray::from_slice(&[1, 2, 3]);
        let cloned = arr.clone_data();
        assert_eq!(cloned.as_slice(), arr.as_slice());
        assert_ne!(arr.data, cloned.data);
    }

    #[test]
    fn test_into_vec() {
        let arr = UnifiedArray::from_slice(&[5, 10, 15]);
        let vec = arr.into_vec();
        assert_eq!(vec, vec![5, 10, 15]);
    }

    #[test]
    fn test_growth() {
        let mut arr = UnifiedArray::new();
        for i in 0..100 {
            arr.push(i);
        }
        assert_eq!(arr.len(), 100);
        for i in 0..100 {
            assert_eq!(arr[i], i as u64);
        }
    }

    #[test]
    fn test_index_access() {
        let mut arr = UnifiedArray::from_slice(&[10, 20, 30]);
        assert_eq!(arr[0], 10);
        assert_eq!(arr[1], 20);
        arr[1] = 99;
        assert_eq!(arr[1], 99);
    }

    #[test]
    fn test_set_boxed_updates_value() {
        let mut arr = UnifiedArray::from_slice(&[10, 20, 30]);
        assert!(arr.set_boxed(1, 99));
        assert_eq!(arr[1], 99);
        assert!(!arr.set_boxed(4, 123));
    }

    #[test]
    fn test_with_capacity() {
        let mut arr = UnifiedArray::with_capacity(10);
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        arr.push(42);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0], 42);
    }

    #[test]
    fn test_reserve_preserves_existing_elements() {
        let mut arr = UnifiedArray::from_slice(&[1, 2, 3]);
        let old_cap = arr.cap;
        arr.reserve(64);
        assert!(arr.cap >= 64);
        assert!(arr.cap >= old_cap);
        assert_eq!(arr.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn test_iter() {
        let arr = UnifiedArray::from_slice(&[1, 2, 3]);
        let sum: u64 = arr.iter().sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn test_with_kind() {
        let arr = UnifiedArray::new().with_kind(tags::HEAP_KIND_INT_ARRAY as u16);
        assert_eq!(arr.kind, tags::HEAP_KIND_INT_ARRAY as u16);
    }

    #[test]
    fn test_heap_box_round_trip() {
        let arr = UnifiedArray::from_slice(&[100, 200, 300]);
        let bits = arr.heap_box();

        // Verify it's a TAG_HEAP value
        assert!(tags::is_tagged(bits));
        assert_eq!(tags::get_tag(bits), tags::TAG_HEAP);

        // Verify bit 47 is NOT set (JIT format)
        let payload = tags::get_payload(bits);
        assert_eq!(payload & (1u64 << 47), 0, "bit 47 must not be set");

        // Round-trip
        let recovered = unsafe { UnifiedArray::from_heap_bits(bits) };
        assert_eq!(recovered.len(), 3);
        assert_eq!(recovered.as_slice(), &[100, 200, 300]);

        // Clean up
        unsafe { UnifiedArray::heap_drop(bits) };
    }

    #[test]
    fn test_bootstrap_float_kind_on_first_push() {
        let mut arr = UnifiedArray::new();
        arr.push(f64::to_bits(1.5));
        assert_eq!(arr.element_kind(), ArrayElementKind::Float64);
        assert!(!arr.typed_data_ptr().is_null());
    }

    #[test]
    fn test_bootstrap_bool_kind_on_first_push() {
        let mut arr = UnifiedArray::new();
        arr.push(TAG_BOOL_TRUE);
        assert_eq!(arr.element_kind(), ArrayElementKind::Bool);
        assert!(!arr.typed_data_ptr().is_null());
    }

    #[test]
    fn test_invalidate_bool_kind_on_non_bool_write() {
        let mut arr = UnifiedArray::new();
        arr.push(TAG_BOOL_TRUE);
        arr.push(TAG_BOOL_FALSE);
        assert_eq!(arr.element_kind(), ArrayElementKind::Bool);
        arr.push(f64::to_bits(2.0));
        assert_eq!(arr.element_kind(), ArrayElementKind::Untyped);
    }

    #[test]
    fn test_default_kind_is_array() {
        let arr = UnifiedArray::new();
        assert_eq!(arr.kind, tags::HEAP_KIND_ARRAY as u16);
    }

    #[test]
    fn test_refcount_default() {
        let arr = UnifiedArray::new();
        assert_eq!(arr.refcount.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
