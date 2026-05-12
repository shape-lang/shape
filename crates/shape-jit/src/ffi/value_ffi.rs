//! JIT-side value-encoding helpers (NaN-box layout used by JIT-emitted code).
//!
//! Per ADR-006 §2.7.5, the JIT FFI boundary carries raw `u64` plus a parallel
//! `NativeKind` companion stamped at JIT compile time from the call signature.
//! The constants and helpers in this module are JIT-internal: they encode the
//! sentinel u64 layout that JIT-emitted Cranelift code uses for inline scalars
//! (`TAG_NULL`, `TAG_BOOL_*`, `TAG_UNIT`, `TAG_DATA_ROW`) and the JitAlloc /
//! `UnifiedValue` pointer shape for heap values.
//!
//! The deleted `shape_value::tag_bits::*`, `shape_value::ValueWord*`,
//! `shape_value::ValueBits`, `shape_value::unified_string`, and
//! `shape_value::unified_wrapper` references that this file previously
//! relied on were retired by the strict-typing bulldozer (Phase 2). The
//! tag constants below are defined locally with the exact u64 layout the
//! JIT-emitted code already targets — they are not a "tag_bits restoration
//! shim" (forbidden per W10 playbook §3) but the JIT-internal sentinel
//! encoding that survives §2.7.5's stable-FFI rule (raw u64 ABI, no
//! runtime kind discrimination from the bits themselves; consumers that
//! need a runtime-tier carrier wrap the bits as
//! `KindedSlot::new(ValueSlot::from_raw(bits), kind)` per §2.7.5/Q7).
//!
//! Heap-pointer values produced by `box_string` / `box_ok` / `box_err` /
//! `box_some` / `box_typed_object` / `box_column_ref` use the
//! `jit_kinds::unified_box` shape: a `UnifiedValue<T>` heap allocation with
//! a `kind: u16` prefix at offset 0, readable via
//! `jit_kinds::read_heap_kind` (per §2.7.5: this is *not* tag-bit dispatch —
//! it reads a field from a heap-resident struct). The `HK_*` constants
//! mirror `HeapKind` ordinals (cast to `u16`) for use as the prefix.

use shape_value::HeapKind;
use std::sync::Arc;

use super::jit_kinds::{read_heap_kind, unified_box, unified_unbox};

// ============================================================================
// JIT-internal NaN-box sentinel layout
// ============================================================================
//
// Inline scalars (null, bool, unit, data-row, function-id) ride in negative
// NaN space (sign bit = 1). The 3-bit tag at bits 50-48 selects the inline
// shape; the low 48 bits carry the payload. This layout is local to the JIT
// (no `shape_value::tag_bits` import) — it is the shape JIT-emitted Cranelift
// code references via `iconst(types::I64, TAG_NULL as i64)` etc., kept stable
// so existing JIT-emitted code keeps working through the W10 consumer
// migration cascade.

/// NaN base: all 1s in exponent (bits 62-52). Used for number detection.
pub const NAN_BASE: u64 = 0x7FF0_0000_0000_0000;

/// 16-bit tag mask -- used for legacy positive-NaN tag discrimination in translator IR.
pub const TAG_MASK: u64 = 0xFFFF_0000_0000_0000;

/// Tagged-value base: negative-NaN exponent + sign bit.
pub const TAG_BASE: u64 = 0xFFF8_0000_0000_0000;

/// Bit shift for the 3-bit inline-tag field at bits 50-48.
pub const TAG_SHIFT: u32 = 48;

/// 48-bit payload mask.
pub const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// IEEE-754 canonical quiet NaN (positive sign).
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

/// `i48` payload range — JIT inline-int encoding fits in 48 bits.
pub const I48_MAX: i64 = (1_i64 << 47) - 1;
pub const I48_MIN: i64 = -(1_i64 << 47);

/// Bit-47 marker for unified-heap pointers; legacy bit retained for the
/// JIT consumer migration window where some helpers still discriminate the
/// pointer shape. Per Band 1 close (§2.7.5): the discriminator no longer
/// gates kind decode — both shapes are raw `Box::into_raw` pointers and
/// the kind flows through the parallel `NativeKind` companion.
pub const UNIFIED_HEAP_FLAG: u64 = 1 << 47;
pub const UNIFIED_PTR_MASK: u64 = PAYLOAD_MASK & !UNIFIED_HEAP_FLAG;

/// Low ownership bit cleared on heap-pointer reads.
const HEAP_OWNED_BIT: u64 = 1;
pub const HEAP_PTR_MASK: u64 = !HEAP_OWNED_BIT;

// 3-bit inline tags at bits 50-48 (private — JIT-internal naming carries
// `_BITS` suffix to free the unsuffixed names for the public sentinel values
// callers reference, e.g. `TAG_NULL` / `TAG_NONE` / `TAG_UNIT`).
const TAG_HEAP_BITS: u64 = 0b000;
const TAG_INT_BITS: u64 = 0b001;
const TAG_BOOL_BITS: u64 = 0b010;
const TAG_NONE_BITS: u64 = 0b011;
const TAG_UNIT_BITS: u64 = 0b100;
const TAG_FUNCTION_BITS: u64 = 0b101;

#[inline]
const fn make_tagged(tag: u64, payload: u64) -> u64 {
    TAG_BASE | (tag << TAG_SHIFT) | (payload & PAYLOAD_MASK)
}

#[inline]
fn is_tagged(bits: u64) -> bool {
    bits & TAG_BASE == TAG_BASE
}

#[inline]
fn get_tag(bits: u64) -> u64 {
    (bits >> TAG_SHIFT) & 0b111
}

// ============================================================================
// Inline types -- shared scheme (TAG_BASE space, sign=1, negative NaN)
// ============================================================================

/// Null/None value. Uses shared TAG_NONE (0b011).
pub const TAG_NULL: u64 = make_tagged(TAG_NONE_BITS, 0);

/// Boolean false. Uses shared TAG_BOOL (0b010) with payload 0.
pub const TAG_BOOL_FALSE: u64 = make_tagged(TAG_BOOL_BITS, 0);

/// Boolean true. Uses shared TAG_BOOL (0b010) with payload 1.
pub const TAG_BOOL_TRUE: u64 = make_tagged(TAG_BOOL_BITS, 1);

/// Unit (void return). Uses shared TAG_UNIT (0b100).
pub const TAG_UNIT: u64 = make_tagged(TAG_UNIT_BITS, 0);

/// None sentinel — alias for `TAG_NULL` (`Option::None` JIT representation).
/// Re-exported under `TAG_NONE` for legacy callers.
pub const TAG_NONE: u64 = TAG_NULL;

/// Number tag sentinel (not a real tag -- numbers are plain f64).
pub const TAG_NUMBER: u64 = 0x0000_0000_0000_0000;

// ============================================================================
// Data row encoding -- uses shared TAG_INT (negative NaN space)
// ============================================================================

/// Data row tag: uses the shared TAG_INT (0b001) encoding in negative NaN space.
/// Row indices are stored as i48 in the 48-bit payload.
pub const TAG_DATA_ROW: u64 = TAG_BASE | (TAG_INT_BITS << TAG_SHIFT);

// ============================================================================
// Heap Kind shortcuts (HK_* = HeapKind ordinal as u16)
//
// Use these as the `kind: u16` prefix on `unified_box` / `jit_box`
// allocations: `unified_box(HK_STRING, Arc::new(s))`. Match arms read the
// prefix back via `jit_kinds::read_heap_kind(bits)`.
// ============================================================================
//
// `HeapKind` is the canonical heap-shape discriminator; these `HK_*`
// constants are the `u16` form referenced by JIT-emitted Cranelift code
// (`iconst(types::I16, HK_TYPED_OBJECT as i64)` etc.). Variants the v2
// `HeapValue` enum no longer carries (Range / Enum / TraitObject / etc.)
// keep their historical ordinals for ABI stability of the JIT-emitted
// constants — the kind is just an integer prefix on a `JitAlloc<T>`
// allocation, no `HeapValue` arm is implied.

pub const HK_STRING: u16 = HeapKind::String as u16;
pub const HK_ARRAY: u16 = 1; // legacy ordinal — no surviving HeapValue arm
pub const HK_TYPED_OBJECT: u16 = HeapKind::TypedObject as u16;
pub const HK_CLOSURE: u16 = HeapKind::Closure as u16;
pub const HK_DECIMAL: u16 = HeapKind::Decimal as u16;
pub const HK_BIG_INT: u16 = HeapKind::BigInt as u16;
pub const HK_HOST_CLOSURE: u16 = 6;  // legacy ordinal
pub const HK_DATATABLE: u16 = HeapKind::DataTable as u16;
pub const HK_HASHMAP: u16 = HeapKind::HashMap as u16;
pub const HK_TYPED_TABLE: u16 = 8; // legacy ordinal
pub const HK_ROW_VIEW: u16 = 9; // legacy ordinal
pub const HK_COLUMN_REF: u16 = 10; // legacy ordinal
pub const HK_INDEXED_TABLE: u16 = 11; // legacy ordinal
pub const HK_RANGE: u16 = 12; // legacy ordinal
pub const HK_ENUM: u16 = 13; // legacy ordinal
pub const HK_SOME: u16 = 14; // legacy ordinal
pub const HK_OK: u16 = 15; // legacy ordinal
pub const HK_ERR: u16 = 16; // legacy ordinal
pub const HK_FUTURE: u16 = HeapKind::Future as u16;
pub const HK_TASK_GROUP: u16 = HeapKind::TaskGroup as u16;
pub const HK_TRAIT_OBJECT: u16 = 19; // legacy ordinal
pub const HK_EXPR_PROXY: u16 = 20; // legacy ordinal
pub const HK_FILTER_EXPR: u16 = HeapKind::FilterExpr as u16;
pub const HK_TIME: u16 = 22; // legacy ordinal
pub const HK_DURATION: u16 = 23; // legacy ordinal
pub const HK_TIMESPAN: u16 = 24; // legacy ordinal
pub const HK_TIMEFRAME: u16 = 25; // legacy ordinal
pub const HK_TIME_REFERENCE: u16 = 26; // legacy ordinal
pub const HK_DATETIME_EXPR: u16 = 27; // legacy ordinal
pub const HK_DATA_DATETIME_REF: u16 = 28; // legacy ordinal
pub const HK_TYPE_ANNOTATION: u16 = 29; // legacy ordinal
pub const HK_TYPE_ANNOTATED_VALUE: u16 = 30; // legacy ordinal
pub const HK_PRINT_RESULT: u16 = 31; // legacy ordinal
pub const HK_SIMULATION_CALL: u16 = 32; // legacy ordinal
pub const HK_FUNCTION_REF: u16 = 33; // legacy ordinal
pub const HK_DATA_REFERENCE: u16 = 34; // legacy ordinal
pub const HK_FLOAT_ARRAY: u16 = 49; // legacy ordinal
pub const HK_INT_ARRAY: u16 = 48; // legacy ordinal
pub const HK_FLOAT_ARRAY_SLICE: u16 = 71; // legacy ordinal
pub const HK_MATRIX: u16 = 51; // legacy ordinal
pub const HK_BOOL_ARRAY: u16 = 50; // legacy ordinal
pub const HK_I8_ARRAY: u16 = 57; // legacy ordinal
pub const HK_I16_ARRAY: u16 = 58; // legacy ordinal
pub const HK_I32_ARRAY: u16 = 59; // legacy ordinal
pub const HK_U8_ARRAY: u16 = 60; // legacy ordinal
pub const HK_U16_ARRAY: u16 = 61; // legacy ordinal
pub const HK_U32_ARRAY: u16 = 62; // legacy ordinal
pub const HK_U64_ARRAY: u16 = 63; // legacy ordinal
pub const HK_F32_ARRAY: u16 = 64; // legacy ordinal

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
    assert!(
        TAG_DATA_ROW & 0x8000_0000_0000_0000 != 0,
        "TAG_DATA_ROW must be in negative NaN space"
    );
};

// ============================================================================
// Core Helper Functions
// ============================================================================

/// Check if a value is a plain f64 number (not NaN-boxed with any tag).
/// All tags live in negative NaN space (sign bit = 1).
#[inline]
pub fn is_number(bits: u64) -> bool {
    !is_tagged(bits)
}

/// Unbox a number (assumes value is a number -- check with `is_number()` first).
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
    make_tagged(TAG_FUNCTION_BITS, fn_id as u64)
}

/// Check if a value is an inline function reference.
#[inline]
pub fn is_inline_function(bits: u64) -> bool {
    is_tagged(bits) && get_tag(bits) == TAG_FUNCTION_BITS
}

/// Extract function_id from an inline function reference.
#[inline]
pub fn unbox_function_id(bits: u64) -> u16 {
    (bits & PAYLOAD_MASK) as u16
}

// ============================================================================
// TAG_HEAP helpers -- unified heap value management
// ============================================================================

/// Check if a value has TAG_HEAP (tag bits 50-48 == 0, in negative NaN space).
#[inline]
pub fn is_heap(bits: u64) -> bool {
    is_tagged(bits) && get_tag(bits) == TAG_HEAP_BITS
}

/// Get the heap kind of a value, or None if not a heap value.
///
/// Reads the `kind: u16` prefix at offset 0 of the underlying `JitAlloc` /
/// `UnifiedValue` allocation per ADR-006 §2.7.5 (this is *not* tag-bit
/// dispatch — it reads a field from a heap-resident struct that the
/// producing call placed there).
#[inline]
pub fn heap_kind(bits: u64) -> Option<u16> {
    if !is_heap(bits) {
        return None;
    }
    Some(unsafe { read_heap_kind(unbox_heap_pointer(bits) as u64) })
}

/// Check if a value is a heap value with a specific kind.
#[inline]
pub fn is_heap_kind(bits: u64, expected_kind: u16) -> bool {
    heap_kind(bits) == Some(expected_kind)
}

/// Extract the raw pointer from a TAG_HEAP value (points to JitAlloc header).
#[inline]
pub fn unbox_heap_pointer(bits: u64) -> *const u8 {
    // Mask off the ownership bit (bit 0): owned Box-backed values have bit 0
    // set, which would offset the pointer by 1 byte. Per Band 1 close
    // (§2.7.5), the bit-47 unified-heap discriminator no longer gates kind
    // decode — both shapes are raw `Box::into_raw` pointers, so we strip
    // the unified flag too to recover the canonical pointer.
    (bits & PAYLOAD_MASK & HEAP_PTR_MASK & !UNIFIED_HEAP_FLAG) as *const u8
}

// ============================================================================
// Result Type (Ok/Err) Helper Functions
// ============================================================================
//
// JIT-internal Ok/Err carriers. Each wraps a single u64 inner-bits payload
// in a `UnifiedValue<u64>` heap allocation with prefix kind=HK_OK/HK_ERR.
// The strict-typed `HeapValue::Reference` / typed-Result rebuild is in a
// later W10/Phase-2c sub-cluster; until then, JIT-emitted code stays on
// the raw-u64 wrapper shape per §2.7.5 stable-FFI rule.

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
    unified_box(HK_OK, inner_bits)
}

#[inline]
pub fn box_err(inner_bits: u64) -> u64 {
    unified_box(HK_ERR, inner_bits)
}

#[inline]
pub unsafe fn unbox_result_inner(bits: u64) -> u64 {
    *unsafe { unified_unbox::<u64>(bits) }
}

#[inline]
pub fn unbox_result_pointer(bits: u64) -> *const u64 {
    let ptr = unbox_heap_pointer(bits);
    if ptr.is_null() {
        std::ptr::null()
    } else {
        // Inner u64 sits at the `data` offset of the `UnifiedValue<u64>`
        // allocation per `jit_kinds::JIT_ALLOC_DATA_OFFSET`.
        unsafe { (ptr.add(super::jit_kinds::JIT_ALLOC_DATA_OFFSET)) as *const u64 }
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
    bits == TAG_NULL
}

#[inline]
pub fn is_option_tag(bits: u64) -> bool {
    is_some_tag(bits) || is_none_tag(bits)
}

#[inline]
pub fn box_some(inner_bits: u64) -> u64 {
    unified_box(HK_SOME, inner_bits)
}

#[inline]
pub unsafe fn unbox_some_inner(bits: u64) -> u64 {
    *unsafe { unified_unbox::<u64>(bits) }
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
    is_tagged(bits) && get_tag(bits) == TAG_INT_BITS
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
/// `UnifiedValue<(*const f64, usize)>`. The returned slice borrows from
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
/// for eventually freeing the column.
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
// String Helper Functions
// ============================================================================
//
// Per ADR-006 §2.2 / §2.3, strings live as `Arc<String>` in the v2 heap.
// JIT-side `box_string` wraps an `Arc<String>` in a `UnifiedValue<Arc<String>>`
// allocation with prefix kind=HK_STRING. `unbox_string` reads the prefix
// to recover the inner `Arc<String>` and borrows its `&str`.

/// Box a String as a unified heap string value.
#[inline]
pub fn box_string(s: String) -> u64 {
    unified_box(HK_STRING, Arc::new(s))
}

/// Box a &str as a unified heap string value.
#[inline]
pub fn box_str(s: &str) -> u64 {
    unified_box(HK_STRING, Arc::new(s.to_string()))
}

/// Read a string from a NaN-boxed heap value.
///
/// # Safety
/// `bits` must be a TAG_HEAP value pointing to a live
/// `UnifiedValue<Arc<String>>` allocation produced by `box_string` /
/// `box_str`, or a legacy `JitAlloc<String>` allocation.
#[inline]
pub unsafe fn unbox_string(bits: u64) -> &'static str {
    // The strict-typed JIT-FFI carries `Arc<String>` for HK_STRING-kinded
    // bits per §2.7.5 stable-FFI rule; the legacy `JitAlloc<String>` shape
    // remains for already-emitted JIT code that hasn't migrated to the
    // unified shape. Distinguish on the `kind: u16` prefix at offset 0
    // (which both shapes share — see `jit_kinds::read_heap_kind`).
    let arc: &Arc<String> = unsafe { unified_unbox::<Arc<String>>(bits) };
    arc.as_str()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::jit_kinds::UnifiedValue;

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
    fn test_data_row_round_trip() {
        let bits = box_data_row(999);
        assert!(is_data_row(bits));
        assert_eq!(unbox_data_row(bits), 999);
        assert!(!is_number(bits));
        assert!(!is_heap(bits));
    }

    #[test]
    #[ignore = "phase-2c §2.7.5 / W11-jit-new-array: asserts the deleted \
                ValueWord-shape `is_number` / `box_typed_object` NaN-box \
                encoding. Under strict typing TypedObject pointers are \
                typed `Arc<TypedObjectStorage>` carried directly via the \
                §2.7.5 (bits, NativeKind) JIT-FFI carrier — no NaN-box \
                tagging. Re-enable after the §2.7.5 carrier rebuild lands."]
    fn test_typed_object_encoding() {
        let fake_ptr = 0x0000_1234_5678_0000u64 as *const u8;
        let boxed = box_typed_object(fake_ptr);
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
}
