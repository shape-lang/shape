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
    // Reconstruct a ValueWord from raw bits. This is safe because we're only
    // cloning it (incrementing refcount), then forgetting BOTH the original
    // and the clone — the net effect is +1 refcount.
    let vw: shape_value::ValueWord = unsafe { std::mem::transmute(bits) };
    let _clone = vw.clone(); // +1 refcount
    std::mem::forget(vw); // Don't decrement (we don't own this)
    std::mem::forget(_clone); // Don't decrement (caller owns the new ref)
    bits
}

/// Decrement the reference count of a NaN-boxed heap value and free if last reference.
///
/// For inline values (int, number, bool, none), this is a no-op (Drop is free).
/// For heap values, Drop decrements the Arc refcount and frees if zero.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_release(bits: u64) {
    // Reconstruct a ValueWord and let it drop naturally.
    // This decrements the refcount (and frees if zero).
    let _vw: shape_value::ValueWord = unsafe { std::mem::transmute(bits) };
    // _vw drops here, decrementing refcount
}
