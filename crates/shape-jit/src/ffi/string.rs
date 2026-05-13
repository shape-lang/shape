//! `Arc<String>` strict-typed carrier FFI for JIT-emitted code
//! (W12-jit-string-carrier-unification, Phase 3 cluster-0 Round 12 T2/T3,
//! 2026-05-13).
//!
//! ADR-006 §2.7.5 (producing-site classification) names `NativeKind::String`
//! as the §2.7.5 String carrier with shape `Arc::into_raw(Arc<String>) as
//! u64` — the standard Rust Arc layout with refcount at offset -16 of the
//! data pointer. The VM-side consumer (`crates/shape-vm/src/executor/objects/
//! set_methods.rs:136-155::result_slot_to_string_arc`, mirrors in
//! `hashmap_methods.rs`) and `KindedSlot::Drop` for `NativeKind::String`
//! (`crates/shape-value/src/kinded_slot.rs:500-502`) both decode this exact
//! shape via `Arc::increment_strong_count::<String>` / `Arc::from_raw(bits
//! as *const String)`.
//!
//! ## Carrier-shape rule (binding)
//!
//! - **`NativeKind::String` slot**: `Arc::into_raw(Arc<String>) as u64`,
//!   refcount at offset -16. Retain/release dispatches through this
//!   module's `jit_arc_string_retain` / `jit_arc_string_release` — bumps
//!   the Rust Arc control-block refcount.
//!
//! - **JIT-internal NaN-box string carrier**: `Box::into_raw(Box::new(
//!   UnifiedValue<Arc<String>>)) as u64`, refcount at offset +4 inside
//!   the UnifiedValue allocation. Retained/released via the legacy
//!   `jit_arc_retain` / `jit_arc_release` in `ffi/arc.rs`. Stays for
//!   JIT-internal pathways (the dispatch shell's method-name push at
//!   `terminators.rs:235`, `call_string_method` returns, etc.) that
//!   pair the bits with their own JIT-internal decode contract.
//!
//! Mixing the two segfaults at every retain/release reclaim:
//! - `jit_arc_release` on an `Arc::into_raw(Arc<String>) as u64` slot
//!   reads `*(bits + 4) as *const AtomicU32` — offset 4 inside the
//!   `String` payload (`String`'s `ptr/cap/len` words), corrupting the
//!   data on `fetch_sub`.
//! - `Arc::decrement_strong_count::<String>(bits)` on a `Box::into_raw(
//!   Box::new(UnifiedValue<Arc<String>>))` slot decrements `*(bits - 16)`
//!   as if it were the Arc control block — but offset -16 from the
//!   UnifiedValue start is whatever the allocator placed there. UB.
//!
//! ## Round 7A precedent
//!
//! The Result/Option Arc carriers in `ffi/result.rs::jit_arc_result_retain`
//! / `_release` / `jit_arc_option_retain` / `_release` (Round 7A close
//! commit `d01d83b7` + `9f27edcd`) and the Round 9 typed-Arc collection
//! retain/release pairs in `ffi/v2/collection_arc.rs` are the bound
//! precedent shape for every body in this module.
//!
//! ## Round 12 T2/T3 surface closures
//!
//! - Smoke 4 JIT: `let mut s = Set(); s.add("a"); s.add("b"); print(
//!   s.size())` → `2` VM == JIT. The `"a"` / `"b"` constants flow as
//!   `MirConstant::Str` operands stamped `NativeKind::String`; the VM
//!   trampoline's `KindedSlot::Drop` decodes via `Arc::from_raw(bits as
//!   *const String)`. Pre-Round-12 `box_string` returned NaN-box bits →
//!   UB at the VM consumer's `Arc::from_raw`.
//! - `print("hello")` JIT: was clean SURFACE at the print Call-terminator's
//!   `NativeKind::String` arm in `terminators.rs::466` (Round 8A reopen
//!   surfaced). Post-Round-12 the §2.7.5 producer emits the matching
//!   carrier shape and `jit_print_str` reads `&String` directly.

use std::sync::Arc;

// ============================================================================
// Per-NativeKind::String kinded retain / release
// ============================================================================

/// Retain (clone) an `Arc<String>` strong-count share. Bumps the standard
/// Rust Arc refcount at offset -16 of the `Arc::into_raw` pointer via
/// `Arc::increment_strong_count::<String>` — NOT the W-series
/// `UnifiedValue<T>` refcount at offset 4 (`jit_arc_retain`'s shape).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<String>) as u64` produced by
/// the `MirConstant::Str` / `MirConstant::StringId` lowering in
/// `mir_compiler/ownership.rs::compile_constant`, or by the VM-side
/// `KindedSlot::from_string_arc` producer. Null is silently no-op'd
/// (mirror of Round 7A's `jit_arc_result_retain` null-bits safety).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_string_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    // SAFETY: see fn docs. The §2.7.5 String carrier contract names the
    // bits as `Arc::into_raw(Arc<String>) as u64`; `Arc::increment_strong_
    // count` operates on the Arc control block at offset -16.
    unsafe {
        Arc::increment_strong_count(bits as *const String);
    }
}

/// Release an `Arc<String>` strong-count share. Mirrors
/// `jit_arc_string_retain`'s increment — uses
/// `Arc::decrement_strong_count::<String>` per Rust Arc contract.
/// Reaching refcount zero runs `String::Drop` (drops the inner buffer).
///
/// SAFETY: same as `jit_arc_string_retain`. Null is silently no-op'd.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_string_release(bits: u64) {
    if bits == 0 {
        return;
    }
    // SAFETY: see fn docs.
    unsafe {
        Arc::decrement_strong_count(bits as *const String);
    }
}

// ============================================================================
// §2.7.5 String carrier compile-time-emitted-constant helper
// ============================================================================

/// Compile-time helper: allocate an `Arc<String>` for a `MirConstant::Str`
/// / `MirConstant::StringId` site and return the raw `Arc::into_raw(arc) as
/// u64` carrier bits per ADR-006 §2.7.5.
///
/// The constant is embedded as an `iconst I64` in the JIT-emitted code, so
/// the bits are static across every runtime occurrence of the site. To
/// keep the constant alive for the JIT-compiled function's full lifetime,
/// we boost the initial refcount to 2: one share represents the
/// "constant's permanent ownership" (never released by the JIT-emitted
/// retain/release pairs), and one share represents the "active share"
/// that the JIT's per-occurrence retain/release pairs manipulate.
///
/// Without the boost, a single use-then-drop pattern would decrement the
/// refcount to 0 and free the constant; the next call to the JIT function
/// would dereference freed memory.
///
/// The "leaked" extra share is a deliberate per-constant-site one-time
/// memory cost — at most `O(distinct string constants × Arc<String> size)`
/// per JIT-compiled function. Same lifecycle as the legacy NaN-box
/// `box_string` path (which also leaked the constant via `Box::into_raw`
/// without a paired `Box::from_raw`).
#[inline]
pub fn arc_string_constant(s: String) -> u64 {
    let arc = Arc::new(s);
    let ptr = Arc::into_raw(arc);
    // SAFETY: `ptr` was just produced by `Arc::into_raw`. The increment
    // bumps the Arc control-block refcount from 1 to 2 — the "constant's
    // permanent share" is now logically owned and never released.
    unsafe {
        Arc::increment_strong_count(ptr);
    }
    ptr as u64
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Round 7A test pattern: arc_string_constant produces a stable
    /// `Arc::into_raw` pointer with refcount=2 (the constant's permanent
    /// share + the active share). Both retain and release leave a live
    /// allocation.
    #[test]
    fn test_arc_string_constant_refcount_boosted() {
        let bits = arc_string_constant("hello".to_string());
        assert_ne!(bits, 0);

        // Two strong shares exist post-construction (boost from 1 → 2).
        // SAFETY: bits is a fresh Arc::into_raw producer's output.
        let count_after_ctor = unsafe { Arc::strong_count(&Arc::from_raw(bits as *const String)) };
        // `Arc::from_raw` adopted one share; bump back so the count
        // reflects what JIT-emitted code observes.
        unsafe { Arc::increment_strong_count(bits as *const String) };
        assert_eq!(count_after_ctor, 2);

        // Manually retire both shares for cleanup of the test allocation.
        unsafe {
            Arc::decrement_strong_count(bits as *const String);
            Arc::decrement_strong_count(bits as *const String);
        }
    }

    #[test]
    fn test_jit_arc_string_retain_bumps_refcount() {
        let arc = Arc::new("test".to_string());
        let bits = Arc::into_raw(arc) as u64;

        // refcount: 1
        jit_arc_string_retain(bits);
        // refcount: 2

        unsafe {
            let recovered = Arc::from_raw(bits as *const String);
            assert_eq!(Arc::strong_count(&recovered), 2);
            // Adopt restores: from_raw took 1 share; restore by bumping.
            Arc::increment_strong_count(bits as *const String);
            // Now refcount is back to 2 with `recovered` holding one.
            drop(recovered);
            // refcount: 1
            Arc::decrement_strong_count(bits as *const String);
            // refcount: 0 — allocation freed.
        }
    }

    #[test]
    fn test_jit_arc_string_release_drops_refcount() {
        let arc = Arc::new("test".to_string());
        // SAFETY: `arc` is alive; bumping its strong count is sound.
        unsafe {
            Arc::increment_strong_count(Arc::as_ptr(&arc));
        }
        let bits = Arc::into_raw(arc) as u64;
        // refcount: 2 (original Arc + the increment)

        jit_arc_string_release(bits);
        // refcount: 1 — still alive

        unsafe {
            let recovered = Arc::from_raw(bits as *const String);
            assert_eq!(Arc::strong_count(&recovered), 1);
            // `recovered` drops here, retires the last share.
        }
    }

    /// Null-bits safety: retain/release on bits=0 silently no-op.
    /// Mirrors Round 7A's `jit_arc_result_retain` null-bits guard.
    #[test]
    fn test_jit_arc_string_retain_release_null_bits_noop() {
        jit_arc_string_retain(0);
        jit_arc_string_release(0);
        // No segfault, no UB — null is the documented producer-site
        // sentinel for an unallocated String slot.
    }

    /// Round-trip: a constant produced by `arc_string_constant` survives
    /// multiple retain/release cycles without underflowing to 0. The
    /// refcount-boost discipline holds.
    #[test]
    fn test_arc_string_constant_survives_use_drop_cycle() {
        let bits = arc_string_constant("hello".to_string());

        // Simulate JIT-emitted retain/release pairs on the constant.
        for _ in 0..10 {
            jit_arc_string_retain(bits);
            jit_arc_string_release(bits);
        }

        // The constant's permanent share keeps the allocation alive.
        // Read the string via `&*ptr` to verify it's still valid.
        let s: &String = unsafe { &*(bits as *const String) };
        assert_eq!(s, "hello");

        // Simulate the "single use-then-drop" pattern (release without
        // prior retain). The constant's permanent share keeps the
        // allocation alive.
        jit_arc_string_release(bits);
        let s: &String = unsafe { &*(bits as *const String) };
        assert_eq!(s, "hello");

        // Manual cleanup: drop the constant's permanent share.
        unsafe {
            Arc::decrement_strong_count(bits as *const String);
        }
    }

    /// VM-side consumer interop: `Arc::from_raw(bits as *const String)`
    /// must recover the original String content. Same shape as
    /// `set_methods.rs::result_slot_to_string_arc`.
    #[test]
    fn test_arc_string_constant_arc_from_raw_recovers_content() {
        let bits = arc_string_constant("world".to_string());

        // Bump refcount once so the VM-side `Arc::from_raw` consumer
        // can adopt a share without underflowing.
        jit_arc_string_retain(bits);

        unsafe {
            let recovered: Arc<String> = Arc::from_raw(bits as *const String);
            assert_eq!(*recovered, "world");
            // `recovered` retires its share here.
        }

        // Cleanup the constant's permanent share.
        unsafe {
            Arc::decrement_strong_count(bits as *const String);
        }
    }
}
