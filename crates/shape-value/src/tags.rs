//! Shared NaN-boxing tag constants and helpers.
//!
//! This module is the single source of truth for the NaN-boxing bit layout used by
//! `ValueWord` in the VM stack. The JIT, GC, and any other subsystem that needs to
//! inspect or construct NaN-boxed values should import constants from here.
//!
//! ## NaN-boxing scheme
//!
//! All tagged values use sign bit = 1 with a quiet NaN exponent, giving us 51 bits
//! for tag + payload. Normal f64 values (including NaN, which is canonicalized to a
//! positive quiet NaN) are stored directly and never collide with our tagged range.
//!
//! ```text
//! Tagged: 0xFFF[C-F]_XXXX_XXXX_XXXX
//!   Bit 63    = 1 (sign, marks as tagged)
//!   Bits 62-52 = 0x7FF (NaN exponent)
//!   Bit 51    = 1 (quiet NaN bit)
//!   Bits 50-48 = tag (3 bits)
//!   Bits 47-0  = payload (48 bits)
//! ```

// ===== Bit layout constants =====

/// Tagged value base: sign=1 + exponent all 1s + quiet NaN bit.
/// Binary: 1_11111111111_1000...0 = 0xFFF8_0000_0000_0000
/// All tagged values have this prefix, with the 3-bit tag in bits 50-48.
pub const TAG_BASE: u64 = 0xFFF8_0000_0000_0000;

/// Mask for extracting the 48-bit payload (bits 0-47).
pub const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Mask for extracting the 3-bit tag (bits 48-50).
pub const TAG_MASK: u64 = 0x0007_0000_0000_0000;

/// Bit shift for the tag field.
pub const TAG_SHIFT: u32 = 48;

/// Canonical NaN value used when the original f64 is NaN.
/// Positive quiet NaN: 0x7FF8_0000_0000_0000 (sign=0, exponent all 1s, quiet bit).
/// This has sign=0 so it will NOT be detected as tagged (our tagged values have sign=1).
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

/// Maximum i48 value: 2^47 - 1
pub const I48_MAX: i64 = (1_i64 << 47) - 1;

/// Minimum i48 value: -2^47
pub const I48_MIN: i64 = -(1_i64 << 47);

// ===== Tag values =====

/// Heap pointer to `Arc<HeapValue>` (48-bit pointer in payload).
pub const TAG_HEAP: u64 = 0b000;

/// Inline i48 (48-bit signed integer, sign-extended to i64).
pub const TAG_INT: u64 = 0b001;

/// Inline bool (payload bit 0: 0=false, 1=true).
pub const TAG_BOOL: u64 = 0b010;

/// None (Option::None / null).
pub const TAG_NONE: u64 = 0b011;

/// Unit (void return value).
pub const TAG_UNIT: u64 = 0b100;

/// Function reference (payload = u16 function_id).
pub const TAG_FUNCTION: u64 = 0b101;

/// Module function reference (payload = u32 index).
pub const TAG_MODULE_FN: u64 = 0b110;

/// Reference to a stack slot (payload = absolute slot index).
pub const TAG_REF: u64 = 0b111;

// ===== Inline helpers =====

/// Build a tagged NaN-boxed u64 from a tag and payload.
#[inline(always)]
pub fn make_tagged(tag: u64, payload: u64) -> u64 {
    debug_assert!(tag <= 0b111);
    debug_assert!(payload & !PAYLOAD_MASK == 0, "payload exceeds 48 bits");
    TAG_BASE | (tag << TAG_SHIFT) | payload
}

/// Check whether a u64 is a tagged NaN-boxed value (as opposed to a plain f64).
#[inline(always)]
pub fn is_tagged(bits: u64) -> bool {
    (bits & TAG_BASE) == TAG_BASE
}

/// Check whether a u64 is a plain f64 (not tagged).
#[inline(always)]
pub fn is_number(bits: u64) -> bool {
    !is_tagged(bits)
}

/// Extract the 3-bit tag from a tagged NaN-boxed u64.
#[inline(always)]
pub fn get_tag(bits: u64) -> u64 {
    (bits & TAG_MASK) >> TAG_SHIFT
}

/// Extract the 48-bit payload from a NaN-boxed u64.
#[inline(always)]
pub fn get_payload(bits: u64) -> u64 {
    bits & PAYLOAD_MASK
}

/// Sign-extend a 48-bit value to i64.
#[inline(always)]
pub fn sign_extend_i48(bits: u64) -> i64 {
    let shifted = (bits as i64) << 16;
    shifted >> 16
}

// ===== HeapKind discriminator constants (for JIT dispatch) =====
//
// These mirror the `HeapKind` enum in heap_value.rs as integer constants,
// enabling the JIT to dispatch on heap value types without linking to the enum.

pub const HEAP_KIND_STRING: u8 = 0;
pub const HEAP_KIND_ARRAY: u8 = 1;
pub const HEAP_KIND_TYPED_OBJECT: u8 = 2;
pub const HEAP_KIND_CLOSURE: u8 = 3;
pub const HEAP_KIND_DECIMAL: u8 = 4;
pub const HEAP_KIND_BIG_INT: u8 = 5;
pub const HEAP_KIND_HOST_CLOSURE: u8 = 6;
pub const HEAP_KIND_DATATABLE: u8 = 7;
pub const HEAP_KIND_TYPED_TABLE: u8 = 8;
pub const HEAP_KIND_ROW_VIEW: u8 = 9;
pub const HEAP_KIND_COLUMN_REF: u8 = 10;
pub const HEAP_KIND_INDEXED_TABLE: u8 = 11;
pub const HEAP_KIND_RANGE: u8 = 12;
pub const HEAP_KIND_ENUM: u8 = 13;
pub const HEAP_KIND_SOME: u8 = 14;
pub const HEAP_KIND_OK: u8 = 15;
pub const HEAP_KIND_ERR: u8 = 16;
pub const HEAP_KIND_FUTURE: u8 = 17;
pub const HEAP_KIND_TASK_GROUP: u8 = 18;
pub const HEAP_KIND_TRAIT_OBJECT: u8 = 19;
pub const HEAP_KIND_EXPR_PROXY: u8 = 20;
pub const HEAP_KIND_FILTER_EXPR: u8 = 21;
pub const HEAP_KIND_TIME: u8 = 22;
pub const HEAP_KIND_DURATION: u8 = 23;
pub const HEAP_KIND_TIMESPAN: u8 = 24;
pub const HEAP_KIND_TIMEFRAME: u8 = 25;
pub const HEAP_KIND_TIME_REFERENCE: u8 = 26;
pub const HEAP_KIND_DATETIME_EXPR: u8 = 27;
pub const HEAP_KIND_DATA_DATETIME_REF: u8 = 28;
pub const HEAP_KIND_TYPE_ANNOTATION: u8 = 29;
pub const HEAP_KIND_TYPE_ANNOTATED_VALUE: u8 = 30;
pub const HEAP_KIND_PRINT_RESULT: u8 = 31;
pub const HEAP_KIND_SIMULATION_CALL: u8 = 32;
pub const HEAP_KIND_FUNCTION_REF: u8 = 33;
pub const HEAP_KIND_DATA_REFERENCE: u8 = 34;
pub const HEAP_KIND_NUMBER: u8 = 35;
pub const HEAP_KIND_BOOL: u8 = 36;
pub const HEAP_KIND_NONE: u8 = 37;
pub const HEAP_KIND_UNIT: u8 = 38;
pub const HEAP_KIND_FUNCTION: u8 = 39;
pub const HEAP_KIND_MODULE_FUNCTION: u8 = 40;
pub const HEAP_KIND_HASHMAP: u8 = 41;
pub const HEAP_KIND_CONTENT: u8 = 42;
pub const HEAP_KIND_INSTANT: u8 = 43;
pub const HEAP_KIND_IO_HANDLE: u8 = 44;
pub const HEAP_KIND_SHARED_CELL: u8 = 45;
pub const HEAP_KIND_NATIVE_SCALAR: u8 = 46;
pub const HEAP_KIND_NATIVE_VIEW: u8 = 47;
pub const HEAP_KIND_INT_ARRAY: u8 = 48;
pub const HEAP_KIND_FLOAT_ARRAY: u8 = 49;
pub const HEAP_KIND_BOOL_ARRAY: u8 = 50;
pub const HEAP_KIND_MATRIX: u8 = 51;
pub const HEAP_KIND_ITERATOR: u8 = 52;
pub const HEAP_KIND_GENERATOR: u8 = 53;
pub const HEAP_KIND_MUTEX: u8 = 54;
pub const HEAP_KIND_ATOMIC: u8 = 55;
pub const HEAP_KIND_LAZY: u8 = 56;
pub const HEAP_KIND_I8_ARRAY: u8 = 57;
pub const HEAP_KIND_I16_ARRAY: u8 = 58;
pub const HEAP_KIND_I32_ARRAY: u8 = 59;
pub const HEAP_KIND_U8_ARRAY: u8 = 60;
pub const HEAP_KIND_U16_ARRAY: u8 = 61;
pub const HEAP_KIND_U32_ARRAY: u8 = 62;
pub const HEAP_KIND_U64_ARRAY: u8 = 63;
pub const HEAP_KIND_F32_ARRAY: u8 = 64;
pub const HEAP_KIND_SET: u8 = 65;
pub const HEAP_KIND_DEQUE: u8 = 66;
pub const HEAP_KIND_PRIORITY_QUEUE: u8 = 67;
pub const HEAP_KIND_CHANNEL: u8 = 68;
pub const HEAP_KIND_CHAR: u8 = 69;
pub const HEAP_KIND_PROJECTED_REF: u8 = 70;
pub const HEAP_KIND_FLOAT_ARRAY_SLICE: u8 = 71;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_round_trip() {
        for tag in 0..=7u64 {
            let payload = 0x1234_5678_ABCDu64;
            let bits = make_tagged(tag, payload);
            assert!(is_tagged(bits));
            assert!(!is_number(bits));
            assert_eq!(get_tag(bits), tag);
            assert_eq!(get_payload(bits), payload);
        }
    }

    #[test]
    fn test_f64_not_tagged() {
        let f = 3.14f64;
        assert!(!is_tagged(f.to_bits()));
        assert!(is_number(f.to_bits()));
    }

    #[test]
    fn test_canonical_nan_not_tagged() {
        assert!(!is_tagged(CANONICAL_NAN));
    }

    #[test]
    fn test_sign_extend_positive() {
        assert_eq!(sign_extend_i48(42), 42);
    }

    #[test]
    fn test_sign_extend_negative() {
        // -1 as 48 bits: 0x0000_FFFF_FFFF_FFFF
        let neg1_48 = PAYLOAD_MASK; // all 48 bits set
        assert_eq!(sign_extend_i48(neg1_48), -1);
    }

    #[test]
    fn test_sign_extend_boundary() {
        // i48 max = 2^47 - 1
        let max_48 = I48_MAX as u64;
        assert_eq!(sign_extend_i48(max_48), I48_MAX);

        // i48 min = -2^47 (bit 47 set, all lower bits zero)
        let min_48 = (I48_MIN as u64) & PAYLOAD_MASK;
        assert_eq!(sign_extend_i48(min_48), I48_MIN);
    }

    #[test]
    fn test_heap_kind_constants_match_enum_order() {
        // Verify the HEAP_KIND constants match the HeapKind enum discriminant order.
        use crate::heap_value::HeapKind;
        assert_eq!(HEAP_KIND_STRING, HeapKind::String as u8);
        assert_eq!(HEAP_KIND_ARRAY, HeapKind::Array as u8);
        assert_eq!(HEAP_KIND_TYPED_OBJECT, HeapKind::TypedObject as u8);
        assert_eq!(HEAP_KIND_CLOSURE, HeapKind::Closure as u8);
        assert_eq!(HEAP_KIND_DECIMAL, HeapKind::Decimal as u8);
        assert_eq!(HEAP_KIND_BIG_INT, HeapKind::BigInt as u8);
        assert_eq!(HEAP_KIND_HOST_CLOSURE, HeapKind::HostClosure as u8);
        assert_eq!(HEAP_KIND_DATATABLE, HeapKind::DataTable as u8);
        assert_eq!(HEAP_KIND_TYPED_TABLE, HeapKind::TypedTable as u8);
        assert_eq!(HEAP_KIND_ROW_VIEW, HeapKind::RowView as u8);
        assert_eq!(HEAP_KIND_COLUMN_REF, HeapKind::ColumnRef as u8);
        assert_eq!(HEAP_KIND_INDEXED_TABLE, HeapKind::IndexedTable as u8);
        assert_eq!(HEAP_KIND_RANGE, HeapKind::Range as u8);
        assert_eq!(HEAP_KIND_ENUM, HeapKind::Enum as u8);
        assert_eq!(HEAP_KIND_SOME, HeapKind::Some as u8);
        assert_eq!(HEAP_KIND_OK, HeapKind::Ok as u8);
        assert_eq!(HEAP_KIND_ERR, HeapKind::Err as u8);
        assert_eq!(HEAP_KIND_FUTURE, HeapKind::Future as u8);
        assert_eq!(HEAP_KIND_TASK_GROUP, HeapKind::TaskGroup as u8);
        assert_eq!(HEAP_KIND_TRAIT_OBJECT, HeapKind::TraitObject as u8);
        assert_eq!(HEAP_KIND_EXPR_PROXY, HeapKind::ExprProxy as u8);
        assert_eq!(HEAP_KIND_FILTER_EXPR, HeapKind::FilterExpr as u8);
        assert_eq!(HEAP_KIND_TIME, HeapKind::Time as u8);
        assert_eq!(HEAP_KIND_DURATION, HeapKind::Duration as u8);
        assert_eq!(HEAP_KIND_TIMESPAN, HeapKind::TimeSpan as u8);
        assert_eq!(HEAP_KIND_TIMEFRAME, HeapKind::Timeframe as u8);
        assert_eq!(HEAP_KIND_TIME_REFERENCE, HeapKind::TimeReference as u8);
        assert_eq!(HEAP_KIND_DATETIME_EXPR, HeapKind::DateTimeExpr as u8);
        assert_eq!(HEAP_KIND_DATA_DATETIME_REF, HeapKind::DataDateTimeRef as u8);
        assert_eq!(HEAP_KIND_TYPE_ANNOTATION, HeapKind::TypeAnnotation as u8);
        assert_eq!(
            HEAP_KIND_TYPE_ANNOTATED_VALUE,
            HeapKind::TypeAnnotatedValue as u8
        );
        assert_eq!(HEAP_KIND_PRINT_RESULT, HeapKind::PrintResult as u8);
        assert_eq!(HEAP_KIND_SIMULATION_CALL, HeapKind::SimulationCall as u8);
        assert_eq!(HEAP_KIND_FUNCTION_REF, HeapKind::FunctionRef as u8);
        assert_eq!(HEAP_KIND_DATA_REFERENCE, HeapKind::DataReference as u8);
        assert_eq!(HEAP_KIND_NUMBER, HeapKind::Number as u8);
        assert_eq!(HEAP_KIND_BOOL, HeapKind::Bool as u8);
        assert_eq!(HEAP_KIND_NONE, HeapKind::None as u8);
        assert_eq!(HEAP_KIND_UNIT, HeapKind::Unit as u8);
        assert_eq!(HEAP_KIND_FUNCTION, HeapKind::Function as u8);
        assert_eq!(HEAP_KIND_MODULE_FUNCTION, HeapKind::ModuleFunction as u8);
        assert_eq!(HEAP_KIND_HASHMAP, HeapKind::HashMap as u8);
        assert_eq!(HEAP_KIND_CONTENT, HeapKind::Content as u8);
        assert_eq!(HEAP_KIND_INSTANT, HeapKind::Instant as u8);
        assert_eq!(HEAP_KIND_IO_HANDLE, HeapKind::IoHandle as u8);
        assert_eq!(HEAP_KIND_SHARED_CELL, HeapKind::SharedCell as u8);
        assert_eq!(HEAP_KIND_NATIVE_SCALAR, HeapKind::NativeScalar as u8);
        assert_eq!(HEAP_KIND_NATIVE_VIEW, HeapKind::NativeView as u8);
        assert_eq!(HEAP_KIND_INT_ARRAY, HeapKind::IntArray as u8);
        assert_eq!(HEAP_KIND_FLOAT_ARRAY, HeapKind::FloatArray as u8);
        assert_eq!(HEAP_KIND_BOOL_ARRAY, HeapKind::BoolArray as u8);
        assert_eq!(HEAP_KIND_MATRIX, HeapKind::Matrix as u8);
        assert_eq!(HEAP_KIND_ITERATOR, HeapKind::Iterator as u8);
        assert_eq!(HEAP_KIND_GENERATOR, HeapKind::Generator as u8);
        assert_eq!(HEAP_KIND_MUTEX, HeapKind::Mutex as u8);
        assert_eq!(HEAP_KIND_ATOMIC, HeapKind::Atomic as u8);
        assert_eq!(HEAP_KIND_LAZY, HeapKind::Lazy as u8);
        assert_eq!(HEAP_KIND_I8_ARRAY, HeapKind::I8Array as u8);
        assert_eq!(HEAP_KIND_I16_ARRAY, HeapKind::I16Array as u8);
        assert_eq!(HEAP_KIND_I32_ARRAY, HeapKind::I32Array as u8);
        assert_eq!(HEAP_KIND_U8_ARRAY, HeapKind::U8Array as u8);
        assert_eq!(HEAP_KIND_U16_ARRAY, HeapKind::U16Array as u8);
        assert_eq!(HEAP_KIND_U32_ARRAY, HeapKind::U32Array as u8);
        assert_eq!(HEAP_KIND_U64_ARRAY, HeapKind::U64Array as u8);
        assert_eq!(HEAP_KIND_F32_ARRAY, HeapKind::F32Array as u8);
        assert_eq!(HEAP_KIND_SET, HeapKind::Set as u8);
        assert_eq!(HEAP_KIND_DEQUE, HeapKind::Deque as u8);
        assert_eq!(HEAP_KIND_PRIORITY_QUEUE, HeapKind::PriorityQueue as u8);
        assert_eq!(HEAP_KIND_CHANNEL, HeapKind::Channel as u8);
        assert_eq!(HEAP_KIND_CHAR, HeapKind::Char as u8);
        assert_eq!(HEAP_KIND_PROJECTED_REF, HeapKind::ProjectedRef as u8);
        assert_eq!(
            HEAP_KIND_FLOAT_ARRAY_SLICE,
            HeapKind::FloatArraySlice as u8
        );
    }
}
