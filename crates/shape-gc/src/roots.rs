//! Root scanning infrastructure for the GC.
//!
//! The `Trace` trait is implemented by types that contain GC-managed pointers.
//! During the mark phase, the GC calls `trace()` on root objects to discover
//! all reachable heap pointers.

/// Callback type for tracing — accepts raw pointers to GC-managed objects.
pub type TraceCallback = dyn FnMut(*mut u8);

/// Trait for types that can be traced by the GC.
///
/// Implementors enumerate all GC-managed pointers they contain by calling
/// the visitor callback for each pointer.
pub trait Trace {
    /// Trace all GC-managed pointers in this value.
    ///
    /// For each pointer to a GC-managed object, call `visitor(ptr)`.
    fn trace(&self, visitor: &mut dyn FnMut(*mut u8));
}

/// Trace a NaN-boxed u64 value: if it's heap-tagged, yield the raw pointer.
///
/// This is used by the VM to trace stack slots and globals without requiring
/// ValueWord to implement Trace directly (which would create a circular dependency).
#[inline]
pub fn trace_nanboxed_bits(bits: u64, visitor: &mut dyn FnMut(*mut u8)) {
    // NaN-boxing constants (duplicated here to avoid circular dep on shape-value)
    const TAG_BASE: u64 = 0xFFF8_0000_0000_0000;
    const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
    const TAG_SHIFT: u32 = 48;
    const TAG_MASK: u64 = 0x0007_0000_0000_0000;
    const TAG_HEAP: u64 = 0b000;

    let is_tagged = (bits & TAG_BASE) == TAG_BASE;
    if is_tagged {
        let tag = (bits & TAG_MASK) >> TAG_SHIFT;
        if tag == TAG_HEAP {
            // Mask off bit 0 (ownership flag) — owned Box-backed values set it.
            const HEAP_PTR_MASK: u64 = !1;
            let ptr = (bits & PAYLOAD_MASK & HEAP_PTR_MASK) as *mut u8;
            if !ptr.is_null() {
                visitor(ptr);
            }
        }
    }
}

/// Trace a raw u64 that may be a heap pointer (for ValueSlot).
///
/// Used when we know from heap_mask that a slot contains a heap pointer.
#[inline]
pub fn trace_heap_slot(bits: u64, visitor: &mut dyn FnMut(*mut u8)) {
    let ptr = bits as *mut u8;
    if !ptr.is_null() {
        visitor(ptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_non_heap_is_noop() {
        // An i48 value should not yield any pointer
        let i48_bits: u64 = 0xFFF8_0000_0000_0001 | (0b001 << 48); // TAG_INT
        let mut found = false;
        trace_nanboxed_bits(i48_bits, &mut |_| found = true);
        assert!(!found);
    }

    #[test]
    fn test_trace_f64_is_noop() {
        let f64_bits = 3.14_f64.to_bits();
        let mut found = false;
        trace_nanboxed_bits(f64_bits, &mut |_| found = true);
        assert!(!found);
    }
}
