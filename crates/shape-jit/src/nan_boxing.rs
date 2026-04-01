//! JIT NaN-boxing layer — unified tag scheme.
//!
//! All values fit in a single u64. This module uses the **shared** tag scheme from
//! `shape_value::tags` for both inline AND heap types:
//!
//! - **Inline types** (int, bool, none, unit, function, module_fn, ref) use the 3-bit
//!   shared tags from `shape_value::tags` in negative NaN space (TAG_BASE = 0xFFF8).
//! - **Unified heap types** use TAG_HEAP (tag = 0b000) with bit-47 SET and a pointer
//!   to a `UnifiedValue<T>` (or specialized type like `UnifiedWrapper`, `UnifiedString`,
//!   `UnifiedArray`). The unified header `[kind:u16][flags:u8][_:u8][refcount:AtomicU32]`
//!   stores a `u16` kind at offset 0 (matching `HEAP_KIND_*` constants), data at offset 8.
//! - **VM heap values** use TAG_HEAP with bit-47 CLEAR, pointing to `Arc<HeapValue>`.
//! - **Data rows** use the shared TAG_INT (0b001) encoding with the row index stored
//!   in the 48-bit payload.
//!
//! ## Heap value discrimination
//!
//! `heap_kind()` handles both unified (bit-47 set) and VM (bit-47 clear) heap values,
//! reading the kind u16 from offset 0 via the appropriate pointer extraction.
//!
//! ## Number detection
//!
//! `is_number()` uses the shared `shape_value::tags::is_number()` check. Plain f64
//! values have sign=0 and are not detected as tagged.

// ============================================================================
// Re-exports from shared tags
// ============================================================================

pub use shape_value::tags::{
    // Bit layout constants
    CANONICAL_NAN, UNIFIED_HEAP_FLAG, UNIFIED_PTR_MASK,
    // HeapKind constants
    HEAP_KIND_ARRAY,
    HEAP_KIND_BIG_INT,
    HEAP_KIND_BOOL,
    HEAP_KIND_CLOSURE,
    HEAP_KIND_COLUMN_REF,
    HEAP_KIND_DATA_DATETIME_REF,
    HEAP_KIND_DATA_REFERENCE,
    HEAP_KIND_DATATABLE,
    HEAP_KIND_DATETIME_EXPR,
    HEAP_KIND_DECIMAL,
    HEAP_KIND_DURATION,
    HEAP_KIND_ENUM,
    HEAP_KIND_ERR,
    HEAP_KIND_EXPR_PROXY,
    HEAP_KIND_FILTER_EXPR,
    HEAP_KIND_FUNCTION,
    HEAP_KIND_FLOAT_ARRAY,
    HEAP_KIND_FLOAT_ARRAY_SLICE,
    HEAP_KIND_FUNCTION_REF,
    HEAP_KIND_FUTURE,
    HEAP_KIND_HASHMAP,
    HEAP_KIND_INT_ARRAY,
    HEAP_KIND_HOST_CLOSURE,
    HEAP_KIND_MATRIX,
    HEAP_KIND_BOOL_ARRAY,
    HEAP_KIND_I8_ARRAY,
    HEAP_KIND_I16_ARRAY,
    HEAP_KIND_I32_ARRAY,
    HEAP_KIND_U8_ARRAY,
    HEAP_KIND_U16_ARRAY,
    HEAP_KIND_U32_ARRAY,
    HEAP_KIND_U64_ARRAY,
    HEAP_KIND_F32_ARRAY,
    HEAP_KIND_INDEXED_TABLE,
    HEAP_KIND_MODULE_FUNCTION,
    HEAP_KIND_NONE,
    HEAP_KIND_NUMBER,
    HEAP_KIND_OK,
    HEAP_KIND_PRINT_RESULT,
    HEAP_KIND_RANGE,
    HEAP_KIND_ROW_VIEW,
    HEAP_KIND_SIMULATION_CALL,
    HEAP_KIND_SOME,
    HEAP_KIND_STRING,
    HEAP_KIND_TASK_GROUP,
    HEAP_KIND_TIME,
    HEAP_KIND_TIME_REFERENCE,
    HEAP_KIND_TIMEFRAME,
    HEAP_KIND_TIMESPAN,
    HEAP_KIND_TRAIT_OBJECT,
    HEAP_KIND_TYPE_ANNOTATED_VALUE,
    HEAP_KIND_TYPE_ANNOTATION,
    HEAP_KIND_TYPED_OBJECT,
    HEAP_KIND_TYPED_TABLE,
    HEAP_KIND_UNIT,
    I48_MAX,
    I48_MIN,
    PAYLOAD_MASK,
    TAG_BASE,
    TAG_SHIFT,
    // Shared tag helpers
    make_tagged,
    sign_extend_i48,
};

use shape_value::unified_string::UnifiedString;
use shape_value::unified_wrapper::UnifiedWrapper;

// ============================================================================
// NaN-space detection
// ============================================================================

/// NaN base: all 1s in exponent (bits 62-52). Used for number detection.
pub const NAN_BASE: u64 = 0x7FF0_0000_0000_0000;

/// 16-bit tag mask — used for legacy positive-NaN tag discrimination in translator IR.
pub const TAG_MASK: u64 = 0xFFFF_0000_0000_0000;

// ============================================================================
// Inline types — shared scheme (TAG_BASE space, sign=1, negative NaN)
// ============================================================================

/// Null/None value. Uses shared TAG_NONE (0b011).
pub const TAG_NULL: u64 =
    shape_value::tags::TAG_BASE | (shape_value::tags::TAG_NONE << shape_value::tags::TAG_SHIFT);

/// Boolean false. Uses shared TAG_BOOL (0b010) with payload 0.
pub const TAG_BOOL_FALSE: u64 =
    shape_value::tags::TAG_BASE | (shape_value::tags::TAG_BOOL << shape_value::tags::TAG_SHIFT);

/// Boolean true. Uses shared TAG_BOOL (0b010) with payload 1.
pub const TAG_BOOL_TRUE: u64 =
    shape_value::tags::TAG_BASE | (shape_value::tags::TAG_BOOL << shape_value::tags::TAG_SHIFT) | 1;

/// Unit (void return). Uses shared TAG_UNIT (0b100).
pub const TAG_UNIT: u64 =
    shape_value::tags::TAG_BASE | (shape_value::tags::TAG_UNIT << shape_value::tags::TAG_SHIFT);

/// None (alias for TAG_NULL, Option::None).
pub const TAG_NONE: u64 = TAG_NULL;

/// Number tag sentinel (not a real tag — numbers are plain f64).
pub const TAG_NUMBER: u64 = 0x0000_0000_0000_0000;

// ============================================================================
// Data row encoding — uses shared TAG_INT (negative NaN space)
// ============================================================================

/// Data row tag: uses the shared TAG_INT (0b001) encoding in negative NaN space.
/// Row indices are stored as i48 in the 48-bit payload.
/// Full tagged value: TAG_BASE | (TAG_INT << TAG_SHIFT) | row_index
pub const TAG_DATA_ROW: u64 = TAG_BASE | (shape_value::tags::TAG_INT << TAG_SHIFT);

// ============================================================================
// Heap Kind shortcuts (HK_* = HEAP_KIND_* as u16)
//
// Use these in match arms: `match heap_kind(bits) { Some(HK_STRING) => ... }`
// ============================================================================

pub const HK_STRING: u16 = HEAP_KIND_STRING as u16;
pub const HK_ARRAY: u16 = HEAP_KIND_ARRAY as u16;
pub const HK_TYPED_OBJECT: u16 = HEAP_KIND_TYPED_OBJECT as u16;
pub const HK_CLOSURE: u16 = HEAP_KIND_CLOSURE as u16;
pub const HK_DECIMAL: u16 = HEAP_KIND_DECIMAL as u16;
pub const HK_BIG_INT: u16 = HEAP_KIND_BIG_INT as u16;
pub const HK_HOST_CLOSURE: u16 = HEAP_KIND_HOST_CLOSURE as u16;
pub const HK_DATATABLE: u16 = HEAP_KIND_DATATABLE as u16;
pub const HK_HASHMAP: u16 = HEAP_KIND_HASHMAP as u16;
pub const HK_TYPED_TABLE: u16 = HEAP_KIND_TYPED_TABLE as u16;
pub const HK_ROW_VIEW: u16 = HEAP_KIND_ROW_VIEW as u16;
pub const HK_COLUMN_REF: u16 = HEAP_KIND_COLUMN_REF as u16;
pub const HK_INDEXED_TABLE: u16 = HEAP_KIND_INDEXED_TABLE as u16;
pub const HK_RANGE: u16 = HEAP_KIND_RANGE as u16;
pub const HK_ENUM: u16 = HEAP_KIND_ENUM as u16;
pub const HK_SOME: u16 = HEAP_KIND_SOME as u16;
pub const HK_OK: u16 = HEAP_KIND_OK as u16;
pub const HK_ERR: u16 = HEAP_KIND_ERR as u16;
pub const HK_FUTURE: u16 = HEAP_KIND_FUTURE as u16;
pub const HK_TASK_GROUP: u16 = HEAP_KIND_TASK_GROUP as u16;
pub const HK_TRAIT_OBJECT: u16 = HEAP_KIND_TRAIT_OBJECT as u16;
pub const HK_EXPR_PROXY: u16 = HEAP_KIND_EXPR_PROXY as u16;
pub const HK_FILTER_EXPR: u16 = HEAP_KIND_FILTER_EXPR as u16;
pub const HK_TIME: u16 = HEAP_KIND_TIME as u16;
pub const HK_DURATION: u16 = HEAP_KIND_DURATION as u16;
pub const HK_TIMESPAN: u16 = HEAP_KIND_TIMESPAN as u16;
pub const HK_TIMEFRAME: u16 = HEAP_KIND_TIMEFRAME as u16;
pub const HK_TIME_REFERENCE: u16 = HEAP_KIND_TIME_REFERENCE as u16;
pub const HK_DATETIME_EXPR: u16 = HEAP_KIND_DATETIME_EXPR as u16;
pub const HK_DATA_DATETIME_REF: u16 = HEAP_KIND_DATA_DATETIME_REF as u16;
pub const HK_TYPE_ANNOTATION: u16 = HEAP_KIND_TYPE_ANNOTATION as u16;
pub const HK_TYPE_ANNOTATED_VALUE: u16 = HEAP_KIND_TYPE_ANNOTATED_VALUE as u16;
pub const HK_PRINT_RESULT: u16 = HEAP_KIND_PRINT_RESULT as u16;
pub const HK_SIMULATION_CALL: u16 = HEAP_KIND_SIMULATION_CALL as u16;
pub const HK_FUNCTION_REF: u16 = HEAP_KIND_FUNCTION_REF as u16;
pub const HK_DATA_REFERENCE: u16 = HEAP_KIND_DATA_REFERENCE as u16;
pub const HK_FLOAT_ARRAY: u16 = HEAP_KIND_FLOAT_ARRAY as u16;
pub const HK_INT_ARRAY: u16 = HEAP_KIND_INT_ARRAY as u16;
pub const HK_FLOAT_ARRAY_SLICE: u16 = HEAP_KIND_FLOAT_ARRAY_SLICE as u16;
pub const HK_MATRIX: u16 = HEAP_KIND_MATRIX as u16;
pub const HK_BOOL_ARRAY: u16 = HEAP_KIND_BOOL_ARRAY as u16;
pub const HK_I8_ARRAY: u16 = HEAP_KIND_I8_ARRAY as u16;
pub const HK_I16_ARRAY: u16 = HEAP_KIND_I16_ARRAY as u16;
pub const HK_I32_ARRAY: u16 = HEAP_KIND_I32_ARRAY as u16;
pub const HK_U8_ARRAY: u16 = HEAP_KIND_U8_ARRAY as u16;
pub const HK_U16_ARRAY: u16 = HEAP_KIND_U16_ARRAY as u16;
pub const HK_U32_ARRAY: u16 = HEAP_KIND_U32_ARRAY as u16;
pub const HK_U64_ARRAY: u16 = HEAP_KIND_U64_ARRAY as u16;
pub const HK_F32_ARRAY: u16 = HEAP_KIND_F32_ARRAY as u16;

// JIT-specific heap kinds (values >= 128, outside VM's HeapKind enum range)
pub const HK_JIT_FUNCTION: u16 = 128;
pub const HK_JIT_SIGNAL_BUILDER: u16 = 129;
pub const HK_JIT_TABLE_REF: u16 = 130;
/// Plain HashMap<String, u64> objects (JIT-only, distinct from TypedObject).
pub const HK_JIT_OBJECT: u16 = 131;

// ============================================================================
// JIT Heap Allocation Infrastructure
// ============================================================================

/// Prefix for JIT heap allocations. Stored at offset 0 of every JIT-owned
/// heap value, enabling type discrimination via `read_heap_kind()`.
///
/// Layout: `[kind: u16][_pad: 6 bytes][data: T]` — data starts at offset 8.
#[repr(C)]
pub struct JitAlloc<T> {
    /// HeapKind discriminator (matches HK_* / HEAP_KIND_* constants).
    pub kind: u16,
    _pad: [u8; 6],
    /// The actual value.
    pub data: T,
}

/// Byte offset of `data` within a `JitAlloc<T>`.
pub const JIT_ALLOC_DATA_OFFSET: usize = 8;

// Compile-time layout verification
const _: () = {
    // Verify inline types use the shared scheme (negative NaN, sign bit = 1)
    assert!(
        TAG_NULL & 0x8000_0000_0000_0000 != 0,
        "TAG_NULL must be in negative NaN space"
    );
    assert!(
        TAG_BOOL_FALSE & 0x8000_0000_0000_0000 != 0,
        "TAG_BOOL must be in negative NaN space"
    );
    assert!(
        TAG_UNIT & 0x8000_0000_0000_0000 != 0,
        "TAG_UNIT must be in negative NaN space"
    );
    // TAG_DATA_ROW now uses shared TAG_INT in negative NaN space
    assert!(
        TAG_DATA_ROW & 0x8000_0000_0000_0000 != 0,
        "TAG_DATA_ROW must be in negative NaN space"
    );
};

// ============================================================================
// Unified Heap Value (generic replacement for JitAlloc)
// ============================================================================

/// Generic unified heap value with the standard header format.
///
/// Layout: `[kind: u16][flags: u8][_reserved: u8][refcount: AtomicU32][data: T]`
/// Data starts at offset 8, same as JitAlloc, but uses bit-47 unified encoding.
///
/// This is the migration target for all JitAlloc usages. After migration,
/// all TAG_HEAP values with bit-47 set use this layout, and all TAG_HEAP
/// values with bit-47 clear are Arc<HeapValue> from the VM.
#[repr(C)]
pub struct UnifiedValue<T> {
    pub kind: u16,
    pub flags: u8,
    pub _reserved: u8,
    pub refcount: std::sync::atomic::AtomicU32,
    pub data: T,
}

impl<T> UnifiedValue<T> {
    #[inline]
    pub fn new(kind: u16, data: T) -> Self {
        Self {
            kind,
            flags: 0,
            _reserved: 0,
            refcount: std::sync::atomic::AtomicU32::new(1),
            data,
        }
    }

    #[inline]
    pub fn heap_box(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        shape_value::tags::make_unified_heap(ptr as *const u8)
    }

    #[inline]
    pub unsafe fn from_heap_bits(bits: u64) -> &'static Self {
        let ptr = shape_value::tags::unified_heap_ptr(bits) as *const Self;
        debug_assert!(!ptr.is_null(), "UnifiedValue::from_heap_bits: null pointer");
        unsafe { &*ptr }
    }

    #[inline]
    pub unsafe fn from_heap_bits_mut(bits: u64) -> &'static mut Self {
        let ptr = shape_value::tags::unified_heap_ptr(bits) as *mut Self;
        debug_assert!(
            !ptr.is_null(),
            "UnifiedValue::from_heap_bits_mut: null pointer"
        );
        unsafe { &mut *ptr }
    }

    #[inline]
    pub unsafe fn heap_drop(bits: u64) {
        let ptr = shape_value::tags::unified_heap_ptr(bits) as *mut Self;
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

/// Allocate a unified heap value with kind prefix, returning a TAG_HEAP-tagged u64
/// with bit-47 set (unified heap encoding).
///
/// This is the unified replacement for `jit_box()`.
#[inline]
pub fn unified_box<T>(kind: u16, data: T) -> u64 {
    UnifiedValue::new(kind, data).heap_box()
}

/// Get a reference to data within a unified or legacy heap allocation.
///
/// Handles both unified heap (bit-47 set) and legacy JitAlloc (bit-47 clear)
/// formats for backward compatibility during migration.
///
/// # Safety
/// `bits` must be a TAG_HEAP value pointing to a live allocation of type T.
#[inline]
pub unsafe fn unified_unbox<T>(bits: u64) -> &'static T {
    if shape_value::tags::is_unified_heap(bits) {
        &unsafe { UnifiedValue::<T>::from_heap_bits(bits) }.data
    } else {
        unsafe { jit_unbox::<T>(bits) }
    }
}

/// Get a mutable reference to data within a unified or legacy heap allocation.
///
/// # Safety
/// Same as `unified_unbox`, plus exclusive access must be guaranteed.
#[inline]
pub unsafe fn unified_unbox_mut<T>(bits: u64) -> &'static mut T {
    if shape_value::tags::is_unified_heap(bits) {
        &mut unsafe { UnifiedValue::<T>::from_heap_bits_mut(bits) }.data
    } else {
        unsafe { jit_unbox_mut::<T>(bits) }
    }
}

// ============================================================================
// Core Helper Functions
// ============================================================================

/// Check if a value is a plain f64 number (not NaN-boxed with any tag).
/// All tags live in negative NaN space (sign bit = 1).
#[inline]
pub fn is_number(bits: u64) -> bool {
    !shape_value::tags::is_tagged(bits)
}

/// Unbox a number (assumes value is a number — check with `is_number()` first).
#[inline]
pub fn unbox_number(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// Box a number into a NaN-boxed u64.
#[inline]
pub const fn box_number(n: f64) -> u64 {
    f64::to_bits(n)
}

/// Box a boolean into a NaN-boxed u64 (shared scheme).
#[inline]
pub const fn box_bool(b: bool) -> u64 {
    if b { TAG_BOOL_TRUE } else { TAG_BOOL_FALSE }
}

/// Box an inline function reference (shared TAG_FUNCTION, payload = function_id).
#[inline]
pub fn box_function(fn_id: u16) -> u64 {
    make_tagged(shape_value::tags::TAG_FUNCTION, fn_id as u64)
}

/// Check if a value is an inline function reference.
#[inline]
pub fn is_inline_function(bits: u64) -> bool {
    shape_value::tags::is_tagged(bits)
        && shape_value::tags::get_tag(bits) == shape_value::tags::TAG_FUNCTION
}

/// Extract function_id from an inline function reference.
#[inline]
pub fn unbox_function_id(bits: u64) -> u16 {
    (bits & PAYLOAD_MASK) as u16
}

// ============================================================================
// TAG_HEAP helpers — unified heap value management
// ============================================================================

/// Allocate a heap value with kind prefix, returning a TAG_HEAP-tagged u64.
///
/// The returned value has tag bits = TAG_HEAP (0b000) and payload = pointer to
/// `JitAlloc<T>` whose first 2 bytes are the `kind` discriminator.
#[inline]
pub fn jit_box<T>(kind: u16, data: T) -> u64 {
    let alloc = Box::new(JitAlloc {
        kind,
        _pad: [0; 6],
        data,
    });
    let ptr = Box::into_raw(alloc);
    // TAG_HEAP = 0b000, so TAG_BASE | (0 << TAG_SHIFT) | ptr = TAG_BASE | ptr
    TAG_BASE | ((ptr as u64) & PAYLOAD_MASK)
}

/// Read the heap kind (u16) from a TAG_HEAP-tagged value.
///
/// # Safety
/// `bits` must be a valid TAG_HEAP value whose payload pointer is non-null and
/// points to a `JitAlloc`-prefixed allocation (kind at offset 0).
#[inline]
pub unsafe fn read_heap_kind(bits: u64) -> u16 {
    let ptr = (bits & PAYLOAD_MASK) as *const u16;
    unsafe { *ptr }
}

/// Get a reference to the data within a JIT heap allocation.
///
/// The returned reference borrows from the heap allocation with an unbounded
/// lifetime. Callers MUST either:
/// - Use the reference only within the current scope (do not store it), OR
/// - Immediately clone/copy the data if it needs to outlive the current call.
///
/// The reference is only valid as long as the `JitAlloc` has not been freed
/// via `jit_drop`. Holding this reference across a `jit_drop` call on the
/// same `bits` value is undefined behavior.
///
/// # Safety
/// - `bits` must be a TAG_HEAP value pointing to a live `JitAlloc<T>`.
/// - The caller must not hold the returned reference past the lifetime of
///   the allocation (i.e., must not use it after `jit_drop` is called).
/// - The pointee must have been allocated as `JitAlloc<T>` (correct type).
#[inline]
pub unsafe fn jit_unbox<T>(bits: u64) -> &'static T {
    let ptr = (bits & PAYLOAD_MASK) as *const JitAlloc<T>;
    debug_assert!(!ptr.is_null(), "jit_unbox called with null payload pointer");
    unsafe { &(*ptr).data }
}

/// Get a mutable reference to the data within a JIT heap allocation.
///
/// Same safety requirements as `jit_unbox`, plus:
/// - The caller must ensure exclusive access (no other references exist).
///
/// # Safety
/// - `bits` must be a TAG_HEAP value pointing to a live `JitAlloc<T>`.
/// - No other references (mutable or shared) to the same allocation may exist.
/// - The caller must not hold the returned reference past the lifetime of
///   the allocation.
#[inline]
pub unsafe fn jit_unbox_mut<T>(bits: u64) -> &'static mut T {
    let ptr = (bits & PAYLOAD_MASK) as *mut JitAlloc<T>;
    debug_assert!(
        !ptr.is_null(),
        "jit_unbox_mut called with null payload pointer"
    );
    unsafe { &mut (*ptr).data }
}

/// Deallocate a JIT heap value.
///
/// # Safety
/// Must only be called once per allocation. `bits` must be a TAG_HEAP value
/// pointing to `JitAlloc<T>`.
#[inline]
pub unsafe fn jit_drop<T>(bits: u64) {
    let ptr = (bits & PAYLOAD_MASK) as *mut JitAlloc<T>;
    unsafe { drop(Box::from_raw(ptr)) };
}

/// Check if a value has TAG_HEAP (tag bits 50-48 == 0, in negative NaN space).
#[inline]
pub fn is_heap(bits: u64) -> bool {
    shape_value::tags::is_tagged(bits)
        && shape_value::tags::get_tag(bits) == shape_value::tags::TAG_HEAP
}

/// Get the heap kind of a value, or None if not a heap value.
///
/// Handles both unified heap (bit-47 set) and legacy JitAlloc (bit-47 clear) formats.
#[inline]
pub fn heap_kind(bits: u64) -> Option<u16> {
    if !is_heap(bits) {
        return None;
    }
    if shape_value::tags::is_unified_heap(bits) {
        Some(unsafe { shape_value::tags::unified_heap_kind(bits) })
    } else {
        Some(unsafe { read_heap_kind(bits) })
    }
}

/// Check if a value is a heap value with a specific kind.
///
/// Handles both unified heap (bit-47 set) and legacy JitAlloc (bit-47 clear) formats.
#[inline]
pub fn is_heap_kind(bits: u64, expected_kind: u16) -> bool {
    heap_kind(bits) == Some(expected_kind)
}

/// Extract the raw pointer from a TAG_HEAP value (points to JitAlloc header).
#[inline]
pub fn unbox_heap_pointer(bits: u64) -> *const u8 {
    (bits & PAYLOAD_MASK) as *const u8
}

// ============================================================================
// Result Type (Ok/Err) Helper Functions
// ============================================================================

#[inline]
pub fn is_ok_tag(bits: u64) -> bool {
    is_heap_kind(bits, HK_OK)
}

#[inline]
pub fn is_err_tag(bits: u64) -> bool {
    is_heap_kind(bits, HK_ERR)
}

#[inline]
pub fn is_result_tag(bits: u64) -> bool {
    is_ok_tag(bits) || is_err_tag(bits)
}

#[inline]
pub fn box_ok(inner_bits: u64) -> u64 {
    UnifiedWrapper::new_ok(inner_bits).heap_box()
}

#[inline]
pub fn box_err(inner_bits: u64) -> u64 {
    UnifiedWrapper::new_err(inner_bits).heap_box()
}

#[inline]
pub unsafe fn unbox_result_inner(bits: u64) -> u64 {
    unsafe { UnifiedWrapper::from_heap_bits(bits) }.inner
}

#[inline]
pub fn unbox_result_pointer(bits: u64) -> *const u64 {
    let ptr = shape_value::tags::unified_heap_ptr(bits) as *const UnifiedWrapper;
    if ptr.is_null() {
        std::ptr::null()
    } else {
        unsafe { &(*ptr).inner as *const u64 }
    }
}

// ============================================================================
// Option Type (Some/None) Helper Functions
// ============================================================================

#[inline]
pub fn is_some_tag(bits: u64) -> bool {
    is_heap_kind(bits, HK_SOME)
}

#[inline]
pub fn is_none_tag(bits: u64) -> bool {
    bits == TAG_NONE
}

#[inline]
pub fn is_option_tag(bits: u64) -> bool {
    is_some_tag(bits) || is_none_tag(bits)
}

#[inline]
pub fn box_some(inner_bits: u64) -> u64 {
    UnifiedWrapper::new_some(inner_bits).heap_box()
}

#[inline]
pub unsafe fn unbox_some_inner(bits: u64) -> u64 {
    unsafe { UnifiedWrapper::from_heap_bits(bits) }.inner
}

// ============================================================================
// Data Row Helper Functions
// ============================================================================

/// Box a row index as a data row reference using shared TAG_INT encoding.
#[inline]
pub const fn box_data_row(row_index: usize) -> u64 {
    TAG_DATA_ROW | ((row_index as u64) & PAYLOAD_MASK)
}

/// Extract the row index from a data row reference (TAG_INT payload).
#[inline]
pub const fn unbox_data_row(bits: u64) -> usize {
    (bits & PAYLOAD_MASK) as usize
}

/// Check if a value is a data row reference.
/// Data rows use the shared TAG_INT encoding (tag bits 50-48 == 0b001).
#[inline]
pub fn is_data_row(bits: u64) -> bool {
    shape_value::tags::is_tagged(bits)
        && shape_value::tags::get_tag(bits) == shape_value::tags::TAG_INT
}

// ============================================================================
// Column Reference Helper Functions
// ============================================================================

#[inline]
pub fn box_column_ref(ptr: *const f64, len: usize) -> u64 {
    unified_box(HK_COLUMN_REF, (ptr, len))
}

#[inline]
pub unsafe fn unbox_column_ref(bits: u64) -> (*const f64, usize) {
    *unsafe { unified_unbox::<(*const f64, usize)>(bits) }
}

#[inline]
pub fn is_column_ref(bits: u64) -> bool {
    is_heap_kind(bits, HK_COLUMN_REF)
}

/// Extract a `&[f64]` slice from a NaN-boxed column reference.
///
/// Returns `None` if `bits` is not a valid column reference, or if the
/// underlying pointer is null or the length is zero.
///
/// # Safety
/// `bits` must be a TAG_HEAP value whose payload points to a live
/// `JitAlloc<(*const f64, usize)>`. The returned slice borrows from
/// the column data and must not outlive the column allocation.
#[inline]
pub unsafe fn extract_column(bits: u64) -> Option<&'static [f64]> {
    if !is_column_ref(bits) {
        return None;
    }
    let (ptr, len) = unsafe { unbox_column_ref(bits) };
    if ptr.is_null() || len == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Box a `Vec<f64>` as a new column reference.
///
/// Leaks the vector into a heap-allocated boxed slice and returns a
/// NaN-boxed column reference pointing to it. The caller is responsible
/// for eventually freeing the column via `jit_drop`.
#[inline]
pub fn box_column_result(data: Vec<f64>) -> u64 {
    let len = data.len();
    let leaked = Box::leak(data.into_boxed_slice());
    box_column_ref(leaked.as_ptr(), len)
}

// ============================================================================
// Typed Object Helper Functions
// ============================================================================

#[inline]
pub fn box_typed_object(ptr: *const u8) -> u64 {
    unified_box(HK_TYPED_OBJECT, ptr)
}

#[inline]
pub fn unbox_typed_object(bits: u64) -> *const u8 {
    *unsafe { unified_unbox::<*const u8>(bits) }
}

#[inline]
pub fn is_typed_object(bits: u64) -> bool {
    is_heap_kind(bits, HK_TYPED_OBJECT)
}

// ============================================================================
// Unified String Helper Functions
// ============================================================================

/// Box a String as a unified heap string value.
#[inline]
pub fn box_string(s: String) -> u64 {
    UnifiedString::from_string(s).heap_box()
}

/// Box a &str as a unified heap string value.
#[inline]
pub fn box_str(s: &str) -> u64 {
    UnifiedString::from_str(s).heap_box()
}

/// Read a string from a NaN-boxed heap value.
///
/// Handles both unified heap (bit-47 set, UnifiedString) and legacy JitAlloc
/// (bit-47 clear) formats for backward compatibility during migration.
///
/// # Safety
/// `bits` must be a TAG_HEAP value pointing to a live string allocation.
#[inline]
pub unsafe fn unbox_string(bits: u64) -> &'static str {
    if shape_value::tags::is_unified_heap(bits) {
        unsafe { UnifiedString::from_heap_bits(bits) }.as_str()
    } else {
        unsafe { jit_unbox::<String>(bits) }.as_str()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inline_types_in_negative_nan_space() {
        assert!(TAG_NULL & 0x8000_0000_0000_0000 != 0);
        assert!(TAG_BOOL_FALSE & 0x8000_0000_0000_0000 != 0);
        assert!(TAG_BOOL_TRUE & 0x8000_0000_0000_0000 != 0);
        assert!(TAG_UNIT & 0x8000_0000_0000_0000 != 0);
    }

    #[test]
    fn test_data_row_in_negative_nan_space() {
        assert!(TAG_DATA_ROW & 0x8000_0000_0000_0000 != 0);
        assert!(!is_number(TAG_DATA_ROW));
    }

    #[test]
    fn test_nan_base_detects_all_tags() {
        assert!(!is_number(TAG_NULL), "TAG_NULL should not be a number");
        assert!(
            !is_number(TAG_DATA_ROW),
            "TAG_DATA_ROW should not be a number"
        );
        assert!(
            !is_number(TAG_BOOL_TRUE),
            "TAG_BOOL_TRUE should not be a number"
        );

        // Plain f64 values should be detected as numbers
        assert!(is_number(box_number(3.14)));
        assert!(is_number(box_number(0.0)));
        assert!(is_number(box_number(-1.0)));
        assert!(is_number(box_number(f64::MAX)));
        assert!(is_number(box_number(f64::MIN)));
    }

    #[test]
    fn test_inline_constants_match_shared_scheme() {
        assert_eq!(TAG_NULL, make_tagged(shape_value::tags::TAG_NONE, 0));
        assert_eq!(TAG_BOOL_FALSE, make_tagged(shape_value::tags::TAG_BOOL, 0));
        assert_eq!(TAG_BOOL_TRUE, make_tagged(shape_value::tags::TAG_BOOL, 1));
        assert_eq!(TAG_UNIT, make_tagged(shape_value::tags::TAG_UNIT, 0));
    }

    #[test]
    fn test_box_unbox_number() {
        let n = 3.14f64;
        let boxed = box_number(n);
        assert!(is_number(boxed));
        assert_eq!(unbox_number(boxed), n);
    }

    #[test]
    fn test_box_unbox_bool() {
        assert_eq!(box_bool(true), TAG_BOOL_TRUE);
        assert_eq!(box_bool(false), TAG_BOOL_FALSE);
    }

    #[test]
    fn test_box_function() {
        let bits = box_function(42);
        assert!(is_inline_function(bits));
        assert_eq!(unbox_function_id(bits), 42);
        assert!(!is_number(bits));
        assert!(!is_heap(bits));
    }

    #[test]
    fn test_unified_string() {
        let bits = box_string("hello".to_string());
        assert!(is_heap(bits));
        assert!(is_heap_kind(bits, HK_STRING));
        assert!(!is_number(bits));
        assert_eq!(heap_kind(bits), Some(HK_STRING));
        assert!(shape_value::tags::is_unified_heap(bits));
        let s = unsafe { unbox_string(bits) };
        assert_eq!(s, "hello");
        unsafe { UnifiedString::heap_drop(bits) };
    }

    #[test]
    fn test_box_str() {
        let bits = box_str("world");
        assert!(is_heap_kind(bits, HK_STRING));
        assert!(shape_value::tags::is_unified_heap(bits));
        let s = unsafe { unbox_string(bits) };
        assert_eq!(s, "world");
        unsafe { UnifiedString::heap_drop(bits) };
    }

    #[test]
    fn test_unified_value_generic() {
        let bits = unified_box(HK_ARRAY, vec![1u64, 2, 3]);
        assert!(is_heap(bits));
        assert!(is_heap_kind(bits, HK_ARRAY));
        assert_eq!(heap_kind(bits), Some(HK_ARRAY));
        assert!(shape_value::tags::is_unified_heap(bits));
        let arr = unsafe { unified_unbox::<Vec<u64>>(bits) };
        assert_eq!(arr.len(), 3);
        unsafe { UnifiedValue::<Vec<u64>>::heap_drop(bits) };
    }

    #[test]
    fn test_heap_kind_none_for_non_heap() {
        assert_eq!(heap_kind(TAG_NULL), None);
        assert_eq!(heap_kind(TAG_BOOL_TRUE), None);
        assert_eq!(heap_kind(box_number(42.0)), None);
        assert_eq!(heap_kind(TAG_DATA_ROW | 5), None);
    }

    #[test]
    fn test_data_row_round_trip() {
        let bits = box_data_row(999);
        assert!(is_data_row(bits));
        assert_eq!(unbox_data_row(bits), 999);
        assert!(!is_number(bits));
        assert!(!is_heap(bits));
    }

    #[test]
    fn test_result_tag_discrimination() {
        assert!(!is_ok_tag(TAG_NULL));
        assert!(!is_err_tag(TAG_NULL));
        assert!(!is_result_tag(box_number(1.0)));

        let ok_val = box_ok(box_number(1.0));
        assert!(is_ok_tag(ok_val));
        assert!(!is_err_tag(ok_val));
        assert!(is_result_tag(ok_val));

        // Verify unified heap encoding (bit 47 set)
        assert!(shape_value::tags::is_unified_heap(ok_val));

        let err_val = box_err(box_number(42.0));
        assert!(is_err_tag(err_val));
        assert!(!is_ok_tag(err_val));
        assert!(is_result_tag(err_val));
        assert!(shape_value::tags::is_unified_heap(err_val));

        // TAG_BOOL values must not be detected as ERR
        assert!(!is_err_tag(TAG_BOOL_FALSE));
        assert!(!is_err_tag(TAG_BOOL_TRUE));

        // Clean up via unified heap drop
        unsafe { UnifiedWrapper::heap_drop(ok_val) };
        unsafe { UnifiedWrapper::heap_drop(err_val) };
    }

    #[test]
    fn test_result_round_trip() {
        let inner = box_number(99.5);
        let ok_val = box_ok(inner);
        assert!(is_ok_tag(ok_val));
        let recovered = unsafe { unbox_result_inner(ok_val) };
        assert_eq!(unbox_number(recovered), 99.5);
        unsafe { UnifiedWrapper::heap_drop(ok_val) };
    }

    #[test]
    fn test_option_tag_discrimination() {
        assert!(is_none_tag(TAG_NONE));
        assert!(is_option_tag(TAG_NONE));
        assert!(!is_some_tag(TAG_NONE));

        let some_val = box_some(box_number(3.14));
        assert!(is_some_tag(some_val));
        assert!(is_option_tag(some_val));
        assert!(!is_none_tag(some_val));
        assert!(shape_value::tags::is_unified_heap(some_val));

        // Round-trip
        let inner = unsafe { unbox_some_inner(some_val) };
        assert_eq!(unbox_number(inner), 3.14);

        // Clean up via unified heap drop
        unsafe { UnifiedWrapper::heap_drop(some_val) };
    }

    #[test]
    fn test_typed_object_encoding() {
        let fake_ptr = 0x0000_1234_5678_0000u64 as *const u8;
        let boxed = box_typed_object(fake_ptr);
        assert!(is_typed_object(boxed));
        assert!(!is_number(boxed));

        // Round-trip: recover the pointer
        let recovered = unbox_typed_object(boxed);
        assert_eq!(recovered, fake_ptr);

        // Non-typed-object values should not match
        assert!(!is_typed_object(TAG_NULL));
        assert!(!is_typed_object(box_number(42.0)));

        // Clean up
        unsafe { UnifiedValue::<*const u8>::heap_drop(boxed) };
    }

    #[test]
    fn test_column_ref_round_trip() {
        let data = vec![1.0f64, 2.0, 3.0];
        let bits = box_column_ref(data.as_ptr(), data.len());
        assert!(is_column_ref(bits));
        assert!(!is_number(bits));
        assert!(shape_value::tags::is_unified_heap(bits));

        let (ptr, len) = unsafe { unbox_column_ref(bits) };
        assert_eq!(ptr, data.as_ptr());
        assert_eq!(len, 3);

        // Clean up
        unsafe { UnifiedValue::<(*const f64, usize)>::heap_drop(bits) };
    }
}
