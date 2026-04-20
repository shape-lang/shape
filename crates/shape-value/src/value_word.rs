//! ValueWord: 8-byte NaN-boxed value representation for the VM stack.
//!
//! Uses IEEE 754 quiet NaN space to pack type tags and payloads into 8 bytes.
//! Simple types (f64, i48, bool, None, Unit, Function) are stored inline.
//! Complex types are heap-allocated as `Arc<HeapValue>` with the raw pointer in the payload.
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
//!
//! | Tag   | Meaning                                      |
//! |-------|----------------------------------------------|
//! | 0b000 | Heap pointer to `Arc<HeapValue>` (48-bit ptr) |
//! | 0b001 | i48 (48-bit signed integer, sign-extended)   |
//! | 0b010 | Bool (payload bit 0)                         |
//! | 0b011 | None                                         |
//! | 0b100 | Unit                                         |
//! | 0b101 | Function(u16) (payload = function_id)        |
//! | 0b110 | ModuleFunction(u32) (payload = index)        |
//! | 0b111 | Ref (absolute stack slot index, 48 bits)     |

use crate::heap_value::{HeapValue, ProjectedRefData};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    Stack(usize),
    ModuleBinding(usize),
    Projected(ProjectedRefData),
}

// ═══════════════════════════════════════════════════════════════════════
// NaN-boxing bit layout constants and helpers (formerly tags.rs)
// ═══════════════════════════════════════════════════════════════════════

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

// ===== Dual-heap ownership flag (bit 0 of heap pointer payload) =====
//
// Both `Arc::into_raw` and `Box::into_raw` return pointers aligned to at least
// 8 bytes (HeapValue contains Arc<String> etc.), so the lowest 3 bits are always
// 0. We use bit 0 as the ownership flag:
//   ptr & 1 == 0  → Shared (Arc-backed, current default)
//   ptr & 1 == 1  → Owned (Box-backed, no refcount)

/// Bit 0 of heap pointer payload: 1 = owned (Box), 0 = shared (Arc).
pub const HEAP_OWNED_BIT: u64 = 1;

/// Mask to strip the owned bit from a heap pointer payload.
pub const HEAP_PTR_MASK: u64 = !HEAP_OWNED_BIT;

/// Check if a heap-tagged value is owned (Box-backed).
#[inline(always)]
pub fn is_heap_owned(bits: u64) -> bool {
    is_tagged(bits) && get_tag(bits) == TAG_HEAP && (get_payload(bits) & HEAP_OWNED_BIT) != 0
}

/// Check if a heap-tagged value is shared (Arc-backed).
#[inline(always)]
pub fn is_heap_shared(bits: u64) -> bool {
    is_tagged(bits) && get_tag(bits) == TAG_HEAP && (get_payload(bits) & HEAP_OWNED_BIT) == 0
}

/// Extract the raw pointer from heap-tagged bits, stripping the owned bit.
#[inline(always)]
pub fn get_heap_ptr(bits: u64) -> *const HeapValue {
    (get_payload(bits) & HEAP_PTR_MASK) as *const HeapValue
}

// ===== Unified heap object discrimination =====

pub const UNIFIED_HEAP_FLAG: u64 = 1 << 47;
pub const UNIFIED_PTR_MASK: u64 = PAYLOAD_MASK & !UNIFIED_HEAP_FLAG;

#[inline(always)]
pub fn is_unified_heap(bits: u64) -> bool {
    is_tagged(bits) && get_tag(bits) == TAG_HEAP && (get_payload(bits) & UNIFIED_HEAP_FLAG) != 0
}

#[inline(always)]
pub fn unified_heap_ptr(bits: u64) -> *const u8 {
    (get_payload(bits) & UNIFIED_PTR_MASK) as *const u8
}

#[inline(always)]
pub unsafe fn unified_heap_kind(bits: u64) -> u16 {
    let ptr = unified_heap_ptr(bits) as *const u16;
    unsafe { *ptr }
}

#[inline(always)]
pub fn make_unified_heap(ptr: *const u8) -> u64 {
    let addr = ptr as u64;
    debug_assert!(addr & UNIFIED_HEAP_FLAG == 0, "pointer already has bit 47 set");
    make_tagged(TAG_HEAP, addr | UNIFIED_HEAP_FLAG)
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
// New consolidated ordinals
pub const HEAP_KIND_TYPED_ARRAY: u8 = 72;
pub const HEAP_KIND_TEMPORAL: u8 = 73;
pub const HEAP_KIND_RARE: u8 = 74;
pub const HEAP_KIND_CONCURRENCY: u8 = 75;
pub const HEAP_KIND_TABLE_VIEW: u8 = 76;

// ═══════════════════════════════════════════════════════════════════════
// End of NaN-boxing bit layout constants and helpers
// ═══════════════════════════════════════════════════════════════════════

/// Single source of truth for tag variants and their inline type dispatch.
///
/// Generates:
/// - `nan_tag_type_name(tag)` — type name string for an inline (non-F64, non-Heap) tag
/// - `nan_tag_is_truthy(tag, payload)` — truthiness for an inline (non-F64, non-Heap) tag
///
/// F64 is handled before the tag match (via `!is_tagged()`), and Heap delegates
/// to HeapValue. Both are kept out of the inline dispatch.
/// Map a raw tag constant to its type name string.
///
/// Internal helper: external callers should use [`ValueBits::tag_type_name`].
#[inline]
pub(crate) fn nan_tag_type_name(tag: u64) -> &'static str {
    match tag {
        TAG_INT => "int",
        TAG_BOOL => "bool",
        TAG_NONE => "option",
        TAG_UNIT => "unit",
        TAG_FUNCTION => "function",
        TAG_MODULE_FN => "module_function",
        TAG_REF => "reference",
        _ => "unknown",
    }
}


/// Evaluate truthiness for an inline tag value.
///
/// Internal helper: external callers should use [`ValueBits::tag_is_truthy`].
#[inline]
pub(crate) fn nan_tag_is_truthy(tag: u64, payload: u64) -> bool {
    match tag {
        TAG_INT => sign_extend_i48(payload) != 0,
        TAG_BOOL => payload != 0,
        TAG_NONE => false,
        TAG_UNIT => false,
        TAG_FUNCTION | TAG_MODULE_FN | TAG_REF => true,
        _ => true,
    }
}

// ArrayView and ArrayViewMut are in crate::array_view
pub use crate::array_view::{ArrayView, ArrayViewMut};

/// An 8-byte value word for the VM stack (NaN-boxed encoding).
/// Type alias for u64.
pub type ValueWord = u64;

/// Wrapper for Display/Debug formatting of ValueWord values.
pub struct ValueWordDisplay(pub u64);

// ═══════════════════════════════════════════════════════════════════════
// Small-string interning (Phase D.4)
// ═══════════════════════════════════════════════════════════════════════
//
// ## Design rationale
//
// True small-string optimization (SSO) in the sense of "pack the bytes inline
// in the 8-byte ValueWord" is not feasible in the current layout: all 8
// NaN-boxing tag values (0b000..0b111) are already consumed (see tag table
// at the top of this file) and only 48 bits of payload are available, which
// is too few bytes to be useful (strings <= 6 bytes is a rounding error).
//
// Multi-slot SSO (spreading bytes across 2-3 adjacent stack slots) would
// require compiler support for multi-slot string bindings and invasive
// changes to the executor + JIT — not worth it as an isolated change.
//
// Instead, we collapse the common case of **repeated short strings** via
// a process-global intern pool. Programs allocate `Arc<String>` over and
// over for the same content (field names, enum tags, short literals like
// "ok", "id", "name"). With interning, N copies share a single allocation
// and the Arc refcount does the rest.
//
// ## Behavioural contract
//
// - `ValueWord::from_string(s)` still returns a `ValueWord` wrapping
//   `Arc<String>`. Callers observe no change: `as_string()` / `as_heap_ref()`
//   return the same `&str` content. Mutation is already impossible via
//   `Arc<String>` (no `Arc::make_mut` is called on interned strings in the
//   codebase — all string ops produce a new `String`).
// - Long strings (len > `INTERN_THRESHOLD`) bypass the pool entirely: the
//   hash/lookup cost isn't justified for long unique payloads, and the
//   memory win would be marginal.
// - The pool is bounded by `INTERN_CAP` entries. When full, new lookups
//   fall through to the no-intern path — we never evict, keeping all live
//   `Arc<String>` refs valid.
// - The pool uses `std::sync::LazyLock<Mutex<...>>` (same pattern as
//   `shape_graph::GLOBAL_SHAPE_TABLE`). A `HashMap<Arc<String>, ()>` (set
//   semantics keyed by the Arc's string content) would work, but using
//   `HashMap<Arc<String>, Arc<String>>` lets us return the *canonical* Arc
//   without rebuilding one.
//
// ## Future work
//
// A fully-inline SSO (store up to ~22 bytes inline across a 24-byte heap
// object with its own refcount) would eliminate the outer `Arc` allocation
// entirely for short strings. That's a bigger change — it touches the
// HeapValue representation, VM executor string reads, JIT FFI, and wire
// serialization. Revisit once the `StringObj` / `UnifiedString` v2 paths
// are the primary runtime representation.
pub(crate) mod string_intern {
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    /// Strings with byte length <= this value are candidates for interning.
    /// Chosen to cover common field names, enum tags, and short literals
    /// (e.g. "ok", "err", "id", "name", "type", "value") while excluding
    /// long user content where the hash cost dominates.
    pub const INTERN_THRESHOLD: usize = 32;

    /// Hard cap on pool size. When reached, new strings bypass interning.
    /// Sized to comfortably fit all stdlib field names + enum tags + common
    /// literals across a large program. Entries are never evicted once
    /// inserted (the pool owns an Arc ref keeping the string alive).
    pub const INTERN_CAP: usize = 8192;

    static POOL: LazyLock<Mutex<HashMap<Arc<String>, Arc<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::with_capacity(256)));

    /// Return the canonical `Arc<String>` for `s` if `s` is short enough to
    /// intern; otherwise return `s` unchanged. Callers should always use
    /// the returned Arc — it may be a different (shared) pointer than the
    /// input.
    #[inline]
    pub fn intern_short_string(s: Arc<String>) -> Arc<String> {
        if s.len() > INTERN_THRESHOLD {
            return s;
        }
        // Acquire the lock. If the mutex is poisoned (another thread panicked
        // while holding it), fall through without interning rather than
        // propagating the panic — interning is an optimization, not a
        // correctness requirement.
        let mut pool = match POOL.lock() {
            Ok(guard) => guard,
            Err(_) => return s,
        };
        if let Some(existing) = pool.get(&s) {
            return existing.clone();
        }
        if pool.len() >= INTERN_CAP {
            return s;
        }
        pool.insert(s.clone(), s.clone());
        s
    }

    /// Test-only: current pool size. Pool entries are never cleared, so
    /// tests should use deltas (not absolute values) to verify growth.
    #[cfg(test)]
    pub(crate) fn __test_pool_len() -> usize {
        POOL.lock().map(|p| p.len()).unwrap_or(0)
    }
}

/// Heap-box a HeapValue (non-GC).
#[inline] #[cfg(not(feature = "gc"))]
pub(crate) fn vw_heap_box(v: HeapValue) -> ValueWord {
    let arc = Arc::new(v);
    let ptr = Arc::into_raw(arc) as u64;
    debug_assert!(ptr & !PAYLOAD_MASK == 0, "pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK)
}
#[inline] #[cfg(feature = "gc")]
pub(crate) fn vw_heap_box(v: HeapValue) -> ValueWord {
    let heap = shape_gc::thread_gc_heap();
    let ptr = heap.alloc(v) as u64;
    debug_assert!(ptr & !PAYLOAD_MASK == 0, "GC pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK)
}

/// Heap-box a HeapValue as uniquely owned (Box, no refcount).
/// Use for values proven to have a single owner by the compiler.
///
/// Internal implementation detail: external callers should go through
/// [`ValueBits::heap_box_owned`] instead. V5.6 scoped this down from
/// `pub` to `pub(crate)` so the free-function API surface outside
/// `value_word.rs` is limited to the `ValueBits` / `ValueWordExt` surface.
#[inline]
#[cfg(not(feature = "gc"))]
pub(crate) fn vw_heap_box_owned(v: HeapValue) -> ValueWord {
    let ptr = Box::into_raw(Box::new(v));
    let addr = ptr as u64;
    debug_assert!(addr & !PAYLOAD_MASK == 0, "pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, (addr & PAYLOAD_MASK) | HEAP_OWNED_BIT)
}






impl std::fmt::Display for ValueWordDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_f64() {
            let n = unsafe { self.0.as_f64_unchecked() };
            if n == n.trunc() && n.abs() < 1e15 {
                write!(f, "{}.0", n as i64)
            } else {
                write!(f, "{}", n)
            }
        } else if self.0.is_i64() {
            write!(f, "{}", unsafe { self.0.as_i64_unchecked() })
        } else if self.0.is_bool() {
            write!(f, "{}", unsafe { self.0.as_bool_unchecked() })
        } else if self.0.is_none() {
            write!(f, "none")
        } else if self.0.is_unit() {
            write!(f, "()")
        } else if self.0.is_function() {
            write!(f, "<function:{}>", unsafe { self.0.as_function_unchecked() })
        } else if self.0.is_module_function() {
            write!(f, "<module_function>")
        } else if let Some(target) = self.0.as_ref_target() {
            match target {
                RefTarget::Stack(slot) => write!(f, "&slot_{}", slot),
                RefTarget::ModuleBinding(slot) => write!(f, "&module_{}", slot),
                RefTarget::Projected(_) => write!(f, "&ref"),
            }
        } else if let Some(hv) = self.0.as_heap_ref() {
            // Delegate to HeapValue's Display impl
            write!(f, "{}", hv)
        } else {
            write!(f, "<unknown>")
        }
    }
}

impl std::fmt::Debug for ValueWordDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_f64() {
            write!(f, "ValueWord(f64: {})", unsafe { self.0.as_f64_unchecked() })
        } else if self.0.is_i64() {
            write!(f, "ValueWord(i64: {})", unsafe { self.0.as_i64_unchecked() })
        } else if self.0.is_bool() {
            write!(f, "ValueWord(bool: {})", unsafe {
                self.0.as_bool_unchecked()
            })
        } else if self.0.is_none() {
            write!(f, "ValueWord(None)")
        } else if self.0.is_unit() {
            write!(f, "ValueWord(Unit)")
        } else if self.0.is_function() {
            write!(f, "ValueWord(Function({}))", unsafe {
                self.0.as_function_unchecked()
            })
        } else if let Some(target) = self.0.as_ref_target() {
            write!(f, "ValueWord(Ref({:?}))", target)
        } else if self.0.is_heap() {
            let ptr = (get_payload(self.0) & HEAP_PTR_MASK) as *const HeapValue;
            let hv = unsafe { &*ptr };
            write!(f, "ValueWord(heap: {:?})", hv)
        } else {
            write!(f, "ValueWord(0x{:016x})", self.0)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// ValueBits — extracted to `crate::value_bits` in Phase R6.1.
//
// The re-export below preserves the legacy `value_word::ValueBits` path so
// existing callers continue to resolve without churn. R6.3 will collapse
// the legacy path.
// ═══════════════════════════════════════════════════════════════════════

pub use crate::value_bits::ValueBits;


// ═══════════════════════════════════════════════════════════════════════
// ValueWordExt — extracted to `crate::value_word_ext` in Phase R6.2.
//
// The re-export below preserves the legacy `value_word::ValueWordExt` path
// so existing callers continue to resolve without churn. R6.3 will collapse
// the legacy path.
// ═══════════════════════════════════════════════════════════════════════

pub use crate::value_word_ext::ValueWordExt;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ===== HeapValue size (structural invariant — stays with the type) =====

    #[test]
    fn test_heap_value_size() {
        use crate::heap_value::HeapValue;
        let hv_size = std::mem::size_of::<HeapValue>();
        // Largest payload is TypedObject (32 bytes) or FunctionRef (String 24 + Option<Box> 8 = 32),
        // plus discriminant → ~40 bytes. Allow up to 48 for alignment padding.
        assert!(
            hv_size <= 48,
            "HeapValue grew beyond expected 48 bytes: {} bytes",
            hv_size
        );
    }

    // ===== Tests formerly in tags.rs =====

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

    // ===== Small-string interning (Phase D.4) =====
    //
    // The intern pool is a process-global resource and tests run in parallel,
    // so assertions must be robust to cross-test pollution. We use string
    // contents that are unlikely to appear in other tests (unique prefixes
    // per test) and check relative behavior ("two calls with the same content
    // return Arc-equal results") rather than absolute pool sizes where
    // possible.

    #[test]
    fn test_intern_short_strings_share_allocation() {
        // Two separately-allocated `Arc<String>` with identical content should
        // deduplicate to a single canonical Arc after going through the pool.
        let a = string_intern::intern_short_string(Arc::new("intern_test_share_name".to_string()));
        let b = string_intern::intern_short_string(Arc::new("intern_test_share_name".to_string()));
        assert!(Arc::ptr_eq(&a, &b), "interned short strings must share allocation");
        assert_eq!(&*a, "intern_test_share_name");
    }

    #[test]
    fn test_intern_long_strings_bypass_pool() {
        // A string longer than INTERN_THRESHOLD bypasses the pool.
        // Use a unique prefix so other parallel tests can't collide.
        let long = format!("intern_test_long_{}", "x".repeat(string_intern::INTERN_THRESHOLD + 1));
        assert!(long.len() > string_intern::INTERN_THRESHOLD);
        let a = string_intern::intern_short_string(Arc::new(long.clone()));
        let b = string_intern::intern_short_string(Arc::new(long.clone()));
        // Both pass through untouched; they are NOT the same allocation.
        assert!(!Arc::ptr_eq(&a, &b), "long strings must not be interned");
        assert_eq!(&*a, &*b);
    }

    #[test]
    fn test_intern_threshold_boundary() {
        // Exactly at the threshold: interned. (Use a fixed-length unique
        // string — "aaaa..." padded to exactly THRESHOLD bytes.)
        let at: String = std::iter::repeat('a').take(string_intern::INTERN_THRESHOLD).collect();
        let a1 = string_intern::intern_short_string(Arc::new(at.clone()));
        let a2 = string_intern::intern_short_string(Arc::new(at.clone()));
        assert!(Arc::ptr_eq(&a1, &a2), "len == threshold must intern");

        // One past the threshold: NOT interned.
        let over: String = std::iter::repeat('b').take(string_intern::INTERN_THRESHOLD + 1).collect();
        let b1 = string_intern::intern_short_string(Arc::new(over.clone()));
        let b2 = string_intern::intern_short_string(Arc::new(over.clone()));
        assert!(!Arc::ptr_eq(&b1, &b2), "len > threshold must not intern");
    }

    #[test]
    fn test_intern_preserves_content_across_many_calls() {
        // Repeatedly interning varied short strings returns correct content
        // on every call, including for repeated inputs.
        let inputs = [
            "intern_test_many_a",
            "intern_test_many_b",
            "intern_test_many_c",
            "intern_test_many_a", // duplicate
            "intern_test_many_b", // duplicate
        ];
        let mut results = Vec::new();
        for s in inputs {
            results.push(string_intern::intern_short_string(Arc::new(s.to_string())));
        }
        for (r, s) in results.iter().zip(inputs.iter()) {
            assert_eq!(&***r, *s);
        }
        // Duplicates must be Arc-equal to their first occurrence.
        assert!(Arc::ptr_eq(&results[0], &results[3]), "dup 'a' must share Arc");
        assert!(Arc::ptr_eq(&results[1], &results[4]), "dup 'b' must share Arc");
    }

    // ── ValueBits shim tests moved to `crate::value_bits` (R6.1).
    // ── ValueWordExt tests moved to `crate::value_word_ext` (R6.2).
}
