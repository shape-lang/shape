//! ARC reference counting FFI for JIT-compiled code.
//!
//! When JIT code operates on heap-allocated values (String, Array, HashMap, etc.),
//! it needs to increment/decrement reference counts at ownership boundaries.
//! These functions provide the FFI entry points for that.
//!
//! The implementation uses ValueWord's Clone/Drop to manage refcounts correctly.

/// Increment the reference count of a NaN-boxed heap value.
///
/// For inline values (int, number, bool, none), this is a no-op (Clone is free).
/// For heap values, Clone increments the Arc refcount.
///
/// Returns the same bits (pass-through for convenience in JIT call sequences).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_retain(bits: u64) -> u64 {
    // FR.6: `ValueWord = u64` is Copy — `vw.clone()` is a bit copy, not
    // a refcount bump, and `std::mem::forget` on a Copy u64 is a no-op.
    // Call the real retain helper which bumps the Arc/unified refcount
    // for heap-tagged bits (no-op for scalars). Returns the same bits
    // for non-owned heap values; returns a new bit pattern for owned
    // Box-backed values (deep-cloned). JIT callers must use the
    // returned value — historically always a pass-through, still
    // correct for the Arc-backed hot path.
    shape_value::value_word_drop::vw_clone(bits)
}

/// Decrement the reference count of a NaN-boxed heap value and free if last reference.
///
/// For inline values (int, number, bool, none), this is a no-op (Drop is free).
/// For heap values, Drop decrements the Arc refcount and frees if zero.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_release(bits: u64) {
    // FR.6: `ValueWord = u64` is Copy — the prior `let _vw = transmute(...)`
    // Drop was a silent no-op (refcount never decremented). Call the
    // real release helper, which decrements Arc/unified refcount for
    // heap-tagged bits and is a no-op for scalars.
    shape_value::value_word_drop::vw_drop(bits);
}
