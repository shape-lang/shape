//! ValueSlot: 8-byte raw value storage for TypedObject fields
//!
//! Each slot stores exactly 8 bytes of raw bits. Simple types (f64, i64, bool)
//! use their native bit representation. Complex types (strings, arrays, objects)
//! are stored as heap-allocated Box<HeapValue> raw pointers.
//!
//! The slot itself does NOT self-describe its type. TypedObject's `heap_mask`
//! bitmap identifies which slots contain heap pointers (bit N set = slot N is heap).

use crate::heap_value::HeapValue;
use crate::value_word::ValueWord;

/// A raw 8-byte value slot for TypedObject field storage.
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct ValueSlot(u64);

impl ValueSlot {
    /// Store a f64 as raw IEEE 754 bits.
    pub fn from_number(n: f64) -> Self {
        Self(n.to_bits())
    }

    /// Store an i64 as raw two's complement bits. Full 64-bit range, no precision loss.
    pub fn from_int(i: i64) -> Self {
        Self(i as u64)
    }

    /// Store a u64 directly. Only meaningful when the FieldType is known to be U64.
    pub fn from_u64(v: u64) -> Self {
        Self(v)
    }

    /// Read as u64 (caller must know this slot is u64 type).
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Store a bool as 1/0.
    pub fn from_bool(b: bool) -> Self {
        Self(if b { 1 } else { 0 })
    }

    /// Store None as zero bits.
    pub fn none() -> Self {
        Self(0)
    }

    /// Store any HeapValue on the heap. The caller MUST set the corresponding
    /// bit in `heap_mask` so Drop knows to free this.
    ///
    /// Without `gc` feature: allocates via Box (freed by drop_heap).
    /// With `gc` feature: allocates via GcHeap (freed by garbage collector).
    #[cfg(not(feature = "gc"))]
    pub fn from_heap(value: HeapValue) -> Self {
        let ptr = Box::into_raw(Box::new(value)) as u64;
        Self(ptr)
    }

    /// Store any HeapValue on the GC heap.
    #[cfg(feature = "gc")]
    pub fn from_heap(value: HeapValue) -> Self {
        let heap = shape_gc::thread_gc_heap();
        let ptr = heap.alloc(value) as u64;
        Self(ptr)
    }

    /// Read as f64 (caller must know this slot is f64 type).
    pub fn as_f64(&self) -> f64 {
        f64::from_bits(self.0)
    }

    /// Read as i64 (caller must know this slot is i64 type).
    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    /// Read as bool (caller must know this slot is bool type).
    pub fn as_bool(&self) -> bool {
        self.0 != 0
    }

    /// Read as heap HeapValue reference (caller must know this slot is a heap pointer).
    /// Returns a reference to the pointed-to HeapValue.
    pub fn as_heap_value(&self) -> &HeapValue {
        let ptr = self.0 as *const HeapValue;
        unsafe { &*ptr }
    }

    /// Create a ValueWord directly from this heap slot (no intermediate conversion).
    /// Caller must know this slot is a heap pointer.
    pub fn as_heap_nb(&self) -> ValueWord {
        ValueWord::from_heap_value(self.as_heap_value().clone())
    }

    /// Store a ValueWord losslessly. For inline types (f64, i48, bool,
    /// none, unit, function, module_function), stores the raw NaN-boxed tag bits
    /// directly. For heap-tagged values, clones the HeapValue into a new Box.
    /// Returns `(slot, is_heap)` — caller must set the heap_mask bit if `is_heap`.
    pub fn from_value_word(nb: &ValueWord) -> (Self, bool) {
        use crate::value_word::NanTag;
        if nb.tag() == NanTag::Heap {
            if let Some(hv) = nb.as_heap_ref() {
                return (Self::from_heap(hv.clone()), true);
            }
            return (Self(0), false);
        }
        (Self(nb.raw_bits()), false)
    }

    /// Backward-compatibility alias.
    pub fn from_nanboxed(nb: &ValueWord) -> (Self, bool) {
        Self::from_value_word(nb)
    }

    /// Reconstruct a ValueWord from this slot. `is_heap` must match the value
    /// returned by `from_value_word` (i.e., whether heap_mask bit is set).
    pub fn as_value_word(&self, is_heap: bool) -> ValueWord {
        if is_heap {
            ValueWord::from_heap_value(self.as_heap_value().clone())
        } else {
            // Safety: bits were stored by from_value_word from a valid inline ValueWord.
            // No heap pointer involved, so no refcount management needed.
            unsafe { ValueWord::clone_from_bits(self.0) }
        }
    }

    /// Backward-compatibility alias.
    pub fn as_nanboxed(&self, is_heap: bool) -> ValueWord {
        self.as_value_word(is_heap)
    }

    /// Raw bits for simple copy.
    pub fn raw(&self) -> u64 {
        self.0
    }

    /// Construct from raw bits.
    pub fn from_raw(bits: u64) -> Self {
        Self(bits)
    }

    /// Drop the heap value. MUST only be called on heap slots.
    ///
    /// Without `gc` feature: frees via Box deallocation.
    /// With `gc` feature: no-op (GC handles deallocation).
    ///
    /// # Safety
    /// Caller must ensure this slot actually contains a valid heap pointer.
    #[cfg(not(feature = "gc"))]
    pub unsafe fn drop_heap(&mut self) {
        if self.0 != 0 {
            let ptr = self.0 as *mut HeapValue;
            let _ = unsafe { Box::from_raw(ptr) };
            self.0 = 0;
        }
    }

    /// Drop the heap value (GC path: no-op).
    #[cfg(feature = "gc")]
    pub unsafe fn drop_heap(&mut self) {
        // No-op: garbage collector handles deallocation
        self.0 = 0;
    }

    /// Clone a heap slot by cloning the pointed-to HeapValue into a new Box.
    ///
    /// Without `gc` feature: deep clones into a new Box allocation.
    /// With `gc` feature: bitwise copy (GC tracks all references).
    ///
    /// # Safety
    /// Caller must ensure this slot actually contains a valid heap pointer.
    #[cfg(not(feature = "gc"))]
    pub unsafe fn clone_heap(&self) -> Self {
        if self.0 == 0 {
            return Self(0);
        }
        let ptr = self.0 as *const HeapValue;
        let cloned = unsafe { (*ptr).clone() };
        Self::from_heap(cloned)
    }

    /// Clone a heap slot (GC path: bitwise copy).
    #[cfg(feature = "gc")]
    pub unsafe fn clone_heap(&self) -> Self {
        // Under GC, just copy the pointer — GC traces all live references
        Self(self.0)
    }
}

impl std::fmt::Debug for ValueSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ValueSlot(0x{:016x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_number_roundtrip() {
        let slot = ValueSlot::from_number(3.14);
        assert_eq!(slot.as_f64(), 3.14);
    }

    #[test]
    fn test_int_roundtrip() {
        let slot = ValueSlot::from_int(-42);
        assert_eq!(slot.as_i64(), -42);

        let slot = ValueSlot::from_int(i64::MAX);
        assert_eq!(slot.as_i64(), i64::MAX);

        let slot = ValueSlot::from_int(i64::MIN);
        assert_eq!(slot.as_i64(), i64::MIN);
    }

    #[test]
    fn test_bool_roundtrip() {
        assert!(ValueSlot::from_bool(true).as_bool());
        assert!(!ValueSlot::from_bool(false).as_bool());
    }

    #[test]
    fn test_heap_string_roundtrip() {
        let original = HeapValue::String(Arc::new("hello".to_string()));
        let slot = ValueSlot::from_heap(original.clone());
        let recovered = slot.as_heap_value();
        match recovered {
            HeapValue::String(s) => assert_eq!(s.as_str(), "hello"),
            other => panic!("Expected HeapValue::String, got {:?}", other),
        }
        // Clean up
        unsafe {
            let mut slot = slot;
            slot.drop_heap();
        }
    }
}
