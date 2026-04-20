//! `ValueBits`: method-style shim over the NaN-boxed 8-byte value word.
//!
//! Phase V5.1 (nanbox-removal plan §V5): `ValueWord` is already
//! `pub type ValueWord = u64`, and the machinery that reads/writes tag
//! bits lives in free functions + the `ValueWordExt` trait on `u64`.
//! `ValueBits` is a `#[repr(transparent)]` newtype around those bits that
//! exposes the same operations as inherent methods, so V5.2–V5.6 can
//! migrate consumers file-by-file without disturbing the existing
//! free-function API (which remains the producer until V5.5 swaps it and
//! V5.6 deletes the legacy machinery).
//!
//! This module was extracted from `value_word.rs` in Phase R6.1 of the
//! v2 residuals closeout (pure reorganization — zero behavior change).

use crate::heap_value::HeapValue;
use crate::value_word::{
    ValueWord, ValueWordExt, get_heap_ptr, get_payload, get_tag, is_heap_owned, is_heap_shared,
    is_number, is_tagged, is_unified_heap, make_tagged, make_unified_heap, nan_tag_is_truthy,
    nan_tag_type_name, sign_extend_i48, unified_heap_kind, unified_heap_ptr, vw_heap_box_owned,
};

// ═══════════════════════════════════════════════════════════════════════
// ValueBits — method-style shim over the free-function NaN-box API.
//
// This type is additive — no consumer has migrated yet. The
// `#[allow(dead_code)]` on the impl block is deliberate: most methods
// have zero callers today and will acquire them across V5.2+.
// ═══════════════════════════════════════════════════════════════════════

/// Method-style shim over the NaN-boxed 8-byte value word.
///
/// `ValueBits(bits)` is byte-identical to `bits: u64` (via
/// `#[repr(transparent)]`) and every method delegates to the existing
/// free-function API in this module. See the V5 plan for the migration
/// strategy.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct ValueBits(pub u64);

#[allow(dead_code)]
impl ValueBits {
    // ── Raw bit access ────────────────────────────────────────────────

    /// Wrap a raw `u64` without interpretation.
    #[inline(always)]
    pub fn from_raw(bits: u64) -> Self {
        Self(bits)
    }

    /// Return the raw `u64` bit pattern.
    #[inline(always)]
    pub fn raw(self) -> u64 {
        self.0
    }

    // ── Tag machinery (delegates to free functions) ───────────────────

    /// Build a tagged NaN-boxed value from a tag and payload. Wraps
    /// [`make_tagged`].
    #[inline(always)]
    pub fn make_tagged(tag: u64, payload: u64) -> Self {
        Self(make_tagged(tag, payload))
    }

    /// True when these bits carry a NaN-box tag (not a plain f64).
    /// Wraps [`is_tagged`].
    #[inline(always)]
    pub fn is_tagged(self) -> bool {
        is_tagged(self.0)
    }

    /// True when these bits are a plain (untagged) f64. Wraps
    /// [`is_number`].
    #[inline(always)]
    pub fn is_number(self) -> bool {
        is_number(self.0)
    }

    /// Extract the 3-bit tag. Wraps [`get_tag`].
    #[inline(always)]
    pub fn tag(self) -> u64 {
        get_tag(self.0)
    }

    /// Extract the 48-bit payload. Wraps [`get_payload`].
    #[inline(always)]
    pub fn payload(self) -> u64 {
        get_payload(self.0)
    }

    /// Sign-extend a 48-bit value to `i64`. Wraps [`sign_extend_i48`].
    #[inline(always)]
    pub fn sign_extend_i48(bits: u64) -> i64 {
        sign_extend_i48(bits)
    }

    // ── Dual-heap ownership ───────────────────────────────────────────

    /// True when this is a heap-tagged value with the owned (Box)
    /// flag set. Wraps [`is_heap_owned`].
    #[inline(always)]
    pub fn is_heap_owned(self) -> bool {
        is_heap_owned(self.0)
    }

    /// True when this is a heap-tagged value with the shared (Arc)
    /// flag set. Wraps [`is_heap_shared`].
    #[inline(always)]
    pub fn is_heap_shared(self) -> bool {
        is_heap_shared(self.0)
    }

    /// Heap pointer extracted from the payload (owned bit stripped).
    /// Wraps [`get_heap_ptr`].
    #[inline(always)]
    pub fn heap_ptr(self) -> *const HeapValue {
        get_heap_ptr(self.0)
    }

    /// Box-allocate `v` and return a ValueBits pointing at it with the
    /// owned flag set. Wraps [`vw_heap_box_owned`].
    #[inline]
    #[cfg(not(feature = "gc"))]
    pub fn heap_box_owned(v: HeapValue) -> Self {
        Self(vw_heap_box_owned(v))
    }

    // ── Unified heap object discrimination ────────────────────────────

    /// True when this is a heap-tagged pointer to a unified (v2) heap
    /// object. Wraps [`is_unified_heap`].
    #[inline(always)]
    pub fn is_unified_heap(self) -> bool {
        is_unified_heap(self.0)
    }

    /// Raw `*const u8` pointer for a unified heap object, unified flag
    /// stripped. Wraps [`unified_heap_ptr`].
    #[inline(always)]
    pub fn unified_heap_ptr(self) -> *const u8 {
        unified_heap_ptr(self.0)
    }

    /// Read the 2-byte kind discriminator at the front of a unified
    /// heap object. Wraps [`unified_heap_kind`].
    ///
    /// # Safety
    ///
    /// See [`unified_heap_kind`]: caller must ensure
    /// `is_unified_heap()` holds and the pointed-to block is live.
    #[inline(always)]
    pub unsafe fn unified_heap_kind(self) -> u16 {
        unsafe { unified_heap_kind(self.0) }
    }

    /// Tag a `*const u8` as a unified heap object. Wraps
    /// [`make_unified_heap`].
    #[inline(always)]
    pub fn make_unified_heap(ptr: *const u8) -> Self {
        Self(make_unified_heap(ptr))
    }

    // ── Tag classification helpers ────────────────────────────────────

    /// Type name for an inline tag (non-heap, non-f64). Wraps
    /// [`nan_tag_type_name`].
    #[inline]
    pub fn tag_type_name(tag: u64) -> &'static str {
        nan_tag_type_name(tag)
    }

    /// Truthiness for an inline tag + payload (non-heap, non-f64).
    /// Wraps [`nan_tag_is_truthy`].
    #[inline]
    pub fn tag_is_truthy(tag: u64, payload: u64) -> bool {
        nan_tag_is_truthy(tag, payload)
    }

    // ── ValueWordExt-style constructors (inline tags) ────────────────
    //
    // These mirror the V5.1 spec's `ValueBits::int` / `ValueBits::f64`
    // / `ValueBits::bool` constructors. They delegate to the existing
    // `ValueWordExt` impl on `u64`.

    /// Construct a ValueBits holding an i64 (promoted to `BigInt` if
    /// it overflows i48). Equivalent to `ValueWord::from_i64`.
    #[inline]
    pub fn int(v: i64) -> Self {
        Self(<ValueWord as ValueWordExt>::from_i64(v))
    }

    /// Construct a ValueBits holding an f64 (canonicalizing NaN).
    /// Equivalent to `ValueWord::from_f64`.
    #[inline]
    pub fn f64(v: f64) -> Self {
        Self(<ValueWord as ValueWordExt>::from_f64(v))
    }

    /// Construct a ValueBits holding a bool. Equivalent to
    /// `ValueWord::from_bool`.
    #[inline]
    pub fn bool(v: bool) -> Self {
        Self(<ValueWord as ValueWordExt>::from_bool(v))
    }

    /// The `None` singleton. Equivalent to `ValueWord::none`.
    #[inline]
    pub fn none() -> Self {
        Self(<ValueWord as ValueWordExt>::none())
    }

    /// The `Unit` singleton. Equivalent to `ValueWord::unit`.
    #[inline]
    pub fn unit() -> Self {
        Self(<ValueWord as ValueWordExt>::unit())
    }

    // ── Type checks (delegating to ValueWordExt) ──────────────────────

    /// True when this value is a plain f64.
    #[inline(always)]
    pub fn is_f64(self) -> bool {
        self.0.is_f64()
    }

    /// True when this value is an inline i48.
    #[inline(always)]
    pub fn is_i64(self) -> bool {
        self.0.is_i64()
    }

    /// True when this value is a bool.
    #[inline(always)]
    pub fn is_bool(self) -> bool {
        self.0.is_bool()
    }

    /// True when this value is `None`.
    #[inline(always)]
    pub fn is_none(self) -> bool {
        self.0.is_none()
    }

    /// True when this value is `Unit`.
    #[inline(always)]
    pub fn is_unit(self) -> bool {
        self.0.is_unit()
    }

    /// True when this value is a heap pointer.
    #[inline(always)]
    pub fn is_heap(self) -> bool {
        self.0.is_heap()
    }

    // ── Extractors ────────────────────────────────────────────────────

    /// Extract as f64 if this value is an inline f64.
    #[inline]
    pub fn as_f64(self) -> Option<f64> {
        self.0.as_f64()
    }

    /// Extract as i64 if this value is an exact signed integer.
    #[inline]
    pub fn as_i64(self) -> Option<i64> {
        self.0.as_i64()
    }

    /// Extract as bool if this value is a bool.
    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        self.0.as_bool()
    }

    /// Borrow the underlying `HeapValue`, if any.
    ///
    /// Note: the returned reference's lifetime is tied to `self` via
    /// the `ValueWordExt::as_heap_ref` signature. Callers that need a
    /// `'static` reference (e.g. when the underlying bits are part of
    /// a long-lived stack slot) should reach through the original
    /// `u64` path. Reflects the shape of the existing
    /// `ValueWordExt::as_heap_ref` API exactly.
    #[inline]
    pub fn as_heap_ref(&self) -> Option<&HeapValue> {
        self.0.as_heap_ref()
    }
}

impl From<u64> for ValueBits {
    #[inline(always)]
    fn from(bits: u64) -> Self {
        Self(bits)
    }
}

impl From<ValueBits> for u64 {
    #[inline(always)]
    fn from(v: ValueBits) -> u64 {
        v.0
    }
}

impl std::fmt::Debug for ValueBits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ValueBits(0x{:016x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_word::{TAG_BOOL, TAG_FUNCTION, TAG_INT, TAG_MODULE_FN, TAG_NONE, TAG_REF, TAG_UNIT};
    use std::sync::Arc;

    // ── ValueBits shim (V5.1) ───────────────────────────────────────────
    //
    // The shim is strictly a wrapper over the free-function/trait API
    // defined above. These tests pin the wrap parity for every category
    // the shim covers: raw round-trip, constructors, tag access, heap
    // classification, and heap detection. Each test deliberately asserts
    // the shim method's result against the underlying free-function or
    // `ValueWordExt` method, so any divergence in V5.2+ would break one
    // of these.

    #[test]
    fn test_value_bits_from_u64_roundtrip() {
        // Zero, a non-trivial f64 bit pattern, and a tagged int all round-trip
        // unchanged through `from_raw` / `raw`.
        let patterns = [
            0u64,
            f64::to_bits(3.14159),
            <ValueWord as ValueWordExt>::from_i64(42),
            <ValueWord as ValueWordExt>::from_bool(true),
            <ValueWord as ValueWordExt>::none(),
            <ValueWord as ValueWordExt>::unit(),
        ];
        for &bits in &patterns {
            assert_eq!(ValueBits::from_raw(bits).raw(), bits);
            // `From`/`Into` conversions must agree with the explicit methods.
            let vb: ValueBits = bits.into();
            let back: u64 = vb.into();
            assert_eq!(back, bits);
        }
    }

    #[test]
    fn test_value_bits_int_construction() {
        // Small i48 values round-trip without going through the heap.
        let vb = ValueBits::int(42);
        assert!(vb.is_i64());
        assert_eq!(vb.as_i64(), Some(42));
        // Parity: the shim's raw bits match the ValueWord trait method.
        assert_eq!(vb.raw(), <ValueWord as ValueWordExt>::from_i64(42));

        // Negative i48 boundary.
        let vb = ValueBits::int(-1);
        assert_eq!(vb.as_i64(), Some(-1));
    }

    #[test]
    fn test_value_bits_f64_construction() {
        // f64 construction canonicalizes NaN just like the free-function path.
        let vb = ValueBits::f64(2.718);
        assert!(vb.is_f64());
        assert_eq!(vb.as_f64(), Some(2.718));
        assert_eq!(vb.raw(), <ValueWord as ValueWordExt>::from_f64(2.718));

        // NaN is canonicalized — result should still be an f64 (not tagged).
        let vb_nan = ValueBits::f64(f64::NAN);
        assert!(vb_nan.is_f64());
        assert!(vb_nan.as_f64().unwrap().is_nan());
    }

    #[test]
    fn test_value_bits_bool_none_unit_construction() {
        let vb_true = ValueBits::bool(true);
        assert!(vb_true.is_bool());
        assert_eq!(vb_true.as_bool(), Some(true));
        assert_eq!(vb_true.raw(), <ValueWord as ValueWordExt>::from_bool(true));

        let vb_false = ValueBits::bool(false);
        assert_eq!(vb_false.as_bool(), Some(false));

        let vb_none = ValueBits::none();
        assert!(vb_none.is_none());
        assert_eq!(vb_none.raw(), <ValueWord as ValueWordExt>::none());

        let vb_unit = ValueBits::unit();
        assert!(vb_unit.is_unit());
        assert_eq!(vb_unit.raw(), <ValueWord as ValueWordExt>::unit());
    }

    #[test]
    fn test_value_bits_tag_matches_vw_function() {
        // For every distinct inline tag, the shim's `tag()` must match the
        // underlying `get_tag()` free function, and `payload()` must match
        // `get_payload()`.
        let cases = [
            <ValueWord as ValueWordExt>::from_i64(7),
            <ValueWord as ValueWordExt>::from_bool(true),
            <ValueWord as ValueWordExt>::none(),
            <ValueWord as ValueWordExt>::unit(),
            <ValueWord as ValueWordExt>::from_function(13),
            <ValueWord as ValueWordExt>::from_module_function(99),
        ];
        for bits in cases {
            let vb = ValueBits::from_raw(bits);
            assert_eq!(vb.tag(), get_tag(bits));
            assert_eq!(vb.payload(), get_payload(bits));
            assert_eq!(vb.is_tagged(), is_tagged(bits));
            assert_eq!(vb.is_number(), is_number(bits));
        }
    }

    #[test]
    fn test_value_bits_tagged_vs_number_partition() {
        // Plain f64 bits must satisfy `is_number` and NOT `is_tagged`.
        let vb_num = ValueBits::f64(1.0);
        assert!(vb_num.is_number());
        assert!(!vb_num.is_tagged());

        // Tagged values are the inverse.
        let vb_tagged = ValueBits::int(1);
        assert!(vb_tagged.is_tagged());
        assert!(!vb_tagged.is_number());

        // Make-tagged constructor must agree with the free-function call.
        let from_shim = ValueBits::make_tagged(TAG_INT, 0x1234);
        let from_free = make_tagged(TAG_INT, 0x1234);
        assert_eq!(from_shim.raw(), from_free);
    }

    #[test]
    fn test_value_bits_heap_detection() {
        // Shared heap: a BigInt promoted from an out-of-range i64.
        let shared_bits = <ValueWord as ValueWordExt>::from_i64(i64::MAX);
        let vb_shared = ValueBits::from_raw(shared_bits);
        assert!(vb_shared.is_heap());
        assert!(vb_shared.is_heap_shared());
        assert!(!vb_shared.is_heap_owned());
        // Parity with free functions.
        assert_eq!(vb_shared.is_heap_shared(), is_heap_shared(shared_bits));
        assert_eq!(vb_shared.is_heap_owned(), is_heap_owned(shared_bits));
        assert_eq!(vb_shared.heap_ptr(), get_heap_ptr(shared_bits));

        // Non-heap values fail both is_heap_shared and is_heap_owned.
        let vb_int = ValueBits::int(42);
        assert!(!vb_int.is_heap());
        assert!(!vb_int.is_heap_shared());
        assert!(!vb_int.is_heap_owned());

        let vb_f64 = ValueBits::f64(1.5);
        assert!(!vb_f64.is_heap());
        assert!(!vb_f64.is_heap_shared());
        assert!(!vb_f64.is_heap_owned());
    }

    #[test]
    fn test_value_bits_owned_heap_detection() {
        // `heap_box_owned` sets the OWNED bit on the payload. Verify the
        // shim detects it and the pointer round-trips with the bit stripped.
        let vb = ValueBits::heap_box_owned(HeapValue::BigInt(i64::MAX));
        assert!(vb.is_heap());
        assert!(vb.is_heap_owned());
        assert!(!vb.is_heap_shared());

        // The returned pointer (with the owned bit stripped) must point to
        // a valid HeapValue we can deref.
        let ptr = vb.heap_ptr();
        unsafe {
            let hv = &*ptr;
            assert!(matches!(hv, HeapValue::BigInt(v) if *v == i64::MAX));
        }

        // Clean up: reconstruct the Box to drop the allocation.
        unsafe {
            let raw_ptr = ptr as *mut HeapValue;
            drop(Box::from_raw(raw_ptr));
        }
    }

    #[test]
    fn test_value_bits_as_heap_ref_matches_value_word() {
        // String heap values are shared; the shim's `as_heap_ref` must return
        // the same HeapValue variant as the underlying ValueWord path.
        let s = Arc::new("vb_test".to_string());
        let vw = <ValueWord as ValueWordExt>::from_string(s.clone());
        let vb = ValueBits::from_raw(vw);

        let hv_ref = vb.as_heap_ref().expect("heap ref via shim");
        match hv_ref {
            HeapValue::String(got) => assert_eq!(&***got, "vb_test"),
            _ => panic!("expected String variant"),
        }

        // Inline values return None.
        assert!(ValueBits::int(5).as_heap_ref().is_none());
        assert!(ValueBits::f64(1.0).as_heap_ref().is_none());
        assert!(ValueBits::none().as_heap_ref().is_none());
    }

    #[test]
    fn test_value_bits_unified_heap_roundtrip() {
        // `make_unified_heap` + `is_unified_heap` + `unified_heap_ptr` form
        // a round-trip; the shim must agree with the free-function path.
        // Use a boxed u16 as the "unified object" so we can read its kind.
        let kind: Box<u16> = Box::new(0xBEEF);
        let ptr = Box::into_raw(kind) as *const u8;

        let vb = ValueBits::make_unified_heap(ptr);
        assert!(vb.is_unified_heap());
        assert_eq!(vb.unified_heap_ptr(), ptr);
        unsafe {
            assert_eq!(vb.unified_heap_kind(), 0xBEEF);
        }

        // Parity with free-function API.
        let free = make_unified_heap(ptr);
        assert_eq!(vb.raw(), free);

        // Cleanup.
        unsafe { drop(Box::from_raw(ptr as *mut u16)) };
    }

    #[test]
    fn test_value_bits_tag_type_name_and_truthiness() {
        // Tag classification helpers are pure delegations — verify for every
        // non-heap inline tag constant.
        let tags = [
            TAG_INT,
            TAG_BOOL,
            TAG_NONE,
            TAG_UNIT,
            TAG_FUNCTION,
            TAG_MODULE_FN,
            TAG_REF,
        ];
        for tag in tags {
            assert_eq!(ValueBits::tag_type_name(tag), nan_tag_type_name(tag));
        }

        // Truthiness delegation.
        for tag in tags {
            for payload in [0u64, 1u64, 42u64] {
                assert_eq!(
                    ValueBits::tag_is_truthy(tag, payload),
                    nan_tag_is_truthy(tag, payload)
                );
            }
        }
    }

    #[test]
    fn test_value_bits_repr_transparent() {
        // `#[repr(transparent)]` means ValueBits and u64 have identical
        // size and alignment. This check is load-bearing for the V5 plan —
        // it lets callers at the FFI boundary pass a `ValueBits` where a
        // `u64` is expected, and lets slices of `u64` be reinterpreted as
        // slices of `ValueBits` for the V5.2+ migration.
        assert_eq!(std::mem::size_of::<ValueBits>(), std::mem::size_of::<u64>());
        assert_eq!(std::mem::align_of::<ValueBits>(), std::mem::align_of::<u64>());
    }
}
