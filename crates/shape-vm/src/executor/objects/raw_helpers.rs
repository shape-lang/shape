//! Raw u64 extraction helpers for v2 method handlers.
//!
//! These functions extract typed values directly from raw u64 bits WITHOUT
//! constructing ValueWord. They use the NaN-boxing tag layout from shape_value::tags.
//!
//! Safety contract: callers must know the type of the value (via receiver_type_tag
//! from the opcode, or via HeapKind from the dispatch cascade). Passing bits of
//! the wrong type is undefined behavior for heap pointer extraction.

use shape_value::heap_value::HeapValue;
use shape_value::tags::{get_payload, get_tag, is_tagged, sign_extend_i48, TAG_HEAP, TAG_INT};
use shape_value::VMError;

// ─── Inline scalar extraction ─────────────────────────────────────────────

/// Extract f64 from raw bits. Assumes the value is an untagged f64.
#[inline(always)]
pub fn extract_f64(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// Extract i64 from raw NaN-boxed i48 bits. Assumes TAG_INT.
#[inline(always)]
pub fn extract_i48(bits: u64) -> i64 {
    sign_extend_i48(get_payload(bits))
}

/// Extract a number as f64, coercing from int if needed.
/// Returns None if the bits are not a number or int.
#[inline]
pub fn extract_number_coerce(bits: u64) -> Option<f64> {
    if !is_tagged(bits) {
        Some(f64::from_bits(bits))
    } else if get_tag(bits) == TAG_INT {
        Some(sign_extend_i48(get_payload(bits)) as f64)
    } else {
        None
    }
}

/// Extract bool from raw bits. Assumes the value is a tagged bool.
#[inline(always)]
pub fn extract_bool(bits: u64) -> bool {
    get_payload(bits) != 0
}

// ─── Heap pointer extraction ──────────────────────────────────────────────

/// Extract a raw const pointer to the HeapValue from tagged heap bits.
/// Returns None if not heap-tagged.
#[inline(always)]
pub fn extract_heap_ptr(bits: u64) -> Option<*const HeapValue> {
    if is_tagged(bits) && get_tag(bits) == TAG_HEAP {
        let ptr = get_payload(bits) as *const HeapValue;
        if !ptr.is_null() {
            return Some(ptr);
        }
    }
    None
}

/// Extract a &HeapValue reference from heap-tagged bits.
/// SAFETY: The pointer must be valid for the duration of the returned reference.
/// This is safe when called on stack/arg bits that haven't been dropped.
#[inline(always)]
pub unsafe fn extract_heap_ref(bits: u64) -> Option<&'static HeapValue> {
    extract_heap_ptr(bits).map(|ptr| unsafe { &*ptr })
}

/// Extract a &str from heap-tagged string bits.
/// Returns None if not a heap string.
#[inline]
pub fn extract_str(bits: u64) -> Option<&'static str> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::String(s) => Some(s.as_str()),
            _ => None,
        })
    }
}

/// Extract &DateTime<FixedOffset> from heap-tagged DateTime bits.
#[inline]
pub fn extract_datetime(bits: u64) -> Option<&'static chrono::DateTime<chrono::FixedOffset>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Time(dt) => Some(dt),
            _ => None,
        })
    }
}

/// Extract &std::time::Instant from heap-tagged Instant bits.
#[inline]
pub fn extract_instant(bits: u64) -> Option<&'static std::time::Instant> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Instant(inst) => Some(&**inst),
            _ => None,
        })
    }
}

// ─── Error helpers ────────────────────────────────────────────────────────

/// Get the type name string for error messages, without constructing ValueWord.
#[inline]
pub fn type_name_from_bits(bits: u64) -> &'static str {
    if !is_tagged(bits) {
        return "number";
    }
    let tag = get_tag(bits);
    if tag == TAG_INT {
        return "int";
    }
    if tag == TAG_HEAP {
        if let Some(hv) = unsafe { extract_heap_ref(bits) } {
            return hv.type_name();
        }
    }
    if tag == shape_value::tags::TAG_BOOL {
        return "bool";
    }
    if tag == shape_value::tags::TAG_NONE {
        return "null";
    }
    if tag == shape_value::tags::TAG_UNIT {
        return "unit";
    }
    if tag == shape_value::tags::TAG_FUNCTION || tag == shape_value::tags::TAG_MODULE_FN {
        return "function";
    }
    "unknown"
}

/// Create a TypeError with expected/got from raw bits.
#[inline]
pub fn type_error(expected: &'static str, bits: u64) -> VMError {
    VMError::TypeError {
        expected,
        got: type_name_from_bits(bits),
    }
}
