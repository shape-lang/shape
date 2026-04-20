//! `ValueWord` — the 8-byte NaN-boxed value representation for the VM stack.
//!
//! `ValueWord` is `pub type ValueWord = u64`. All method-style access lives on
//! the [`ValueWordExt`] extension trait ([`crate::value_word_ext`]); raw tag
//! bit layout + helpers live in [`crate::tag_bits`]; the
//! `#[repr(transparent)]` shim used by the V5 migration lives in
//! [`crate::value_bits`]. This file holds only:
//!
//! - [`RefTarget`] — decoded payload for `TAG_REF` stack / module / projected refs
//! - the [`ValueWord`] type alias
//! - [`ValueWordDisplay`] and its `Display`/`Debug` impls
//! - the `vw_heap_box` / `vw_heap_box_owned` heap-construction primitives
//! - re-exports that preserve the legacy `value_word::*` import path
//!
//! The historical size was ~3800 lines; R6.1–R6.3 pared this down in stages
//! (ValueBits → value_bits.rs, ValueWordExt → value_word_ext.rs, tag bits →
//! tag_bits.rs, string interning → string_intern.rs).

use crate::heap_value::{HeapValue, ProjectedRefData};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    Stack(usize),
    ModuleBinding(usize),
    Projected(ProjectedRefData),
}

// ArrayView and ArrayViewMut are in crate::array_view
pub use crate::array_view::{ArrayView, ArrayViewMut};

/// An 8-byte value word for the VM stack (NaN-boxed encoding).
/// Type alias for u64.
pub type ValueWord = u64;

/// Wrapper for Display/Debug formatting of ValueWord values.
pub struct ValueWordDisplay(pub u64);

/// Heap-box a HeapValue (non-GC).
#[inline] #[cfg(not(feature = "gc"))]
pub(crate) fn vw_heap_box(v: HeapValue) -> ValueWord {
    use crate::tag_bits::{PAYLOAD_MASK, TAG_HEAP, make_tagged};
    let arc = Arc::new(v);
    let ptr = Arc::into_raw(arc) as u64;
    debug_assert!(ptr & !PAYLOAD_MASK == 0, "pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK)
}
#[inline] #[cfg(feature = "gc")]
pub(crate) fn vw_heap_box(v: HeapValue) -> ValueWord {
    use crate::tag_bits::{PAYLOAD_MASK, TAG_HEAP, make_tagged};
    let heap = shape_gc::thread_gc_heap();
    let ptr = heap.alloc(v) as u64;
    debug_assert!(ptr & !PAYLOAD_MASK == 0, "GC pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK)
}

/// Heap-box a HeapValue as uniquely owned (Box, no refcount).
/// Use for values proven to have a single owner by the compiler.
///
/// Internal implementation detail: external callers should go through
/// [`crate::ValueBits::heap_box_owned`] instead.
#[inline]
#[cfg(not(feature = "gc"))]
pub(crate) fn vw_heap_box_owned(v: HeapValue) -> ValueWord {
    use crate::tag_bits::{HEAP_OWNED_BIT, PAYLOAD_MASK, TAG_HEAP, make_tagged};
    let ptr = Box::into_raw(Box::new(v));
    let addr = ptr as u64;
    debug_assert!(addr & !PAYLOAD_MASK == 0, "pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, (addr & PAYLOAD_MASK) | HEAP_OWNED_BIT)
}

// ─── Legacy re-exports ────────────────────────────────────────────────────
//
// Preserve the `value_word::X` import path that dates back to before the
// R6.1/R6.2/R6.3 extractions. Users outside this crate import tag constants
// and helpers via `shape_value::tags::*` (see lib.rs), which already pulls
// from `tag_bits`. Internal `shape-value` modules use `crate::tag_bits::*`
// directly. These re-exports cover the remaining callers that refer to the
// old `crate::value_word::{TAG_*, is_*, ...}` path.

pub use crate::tag_bits::*;
pub use crate::value_bits::ValueBits;
pub use crate::value_word_ext::ValueWordExt;

#[cfg(test)]
mod tests {
    use crate::heap_value::HeapValue;

    // ===== HeapValue size (structural invariant — stays with the type) =====

    #[test]
    fn test_heap_value_size() {
        let hv_size = std::mem::size_of::<HeapValue>();
        // Largest payload is TypedObject (32 bytes) or FunctionRef (String 24 + Option<Box> 8 = 32),
        // plus discriminant → ~40 bytes. Allow up to 48 for alignment padding.
        assert!(
            hv_size <= 48,
            "HeapValue grew beyond expected 48 bytes: {} bytes",
            hv_size
        );
    }

    // ── Bit-layout tests moved to `crate::tag_bits` (R6.3).
    // ── String-intern tests moved to `crate::string_intern` (R6.3).
    // ── ValueBits shim tests live in `crate::value_bits` (R6.1).
    // ── ValueWordExt method tests live in `crate::value_word_ext` (R6.2).
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
            use crate::tag_bits::{HEAP_PTR_MASK, get_payload};
            let ptr = (get_payload(self.0) & HEAP_PTR_MASK) as *const HeapValue;
            let hv = unsafe { &*ptr };
            write!(f, "ValueWord(heap: {:?})", hv)
        } else {
            write!(f, "ValueWord(0x{:016x})", self.0)
        }
    }
}
