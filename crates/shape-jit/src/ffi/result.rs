// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 5 sites
//     box_ok, box_err, box_some — jit_make_ok, jit_make_err, jit_make_some
//     (these use sub-tag encoding, not jit_box — allocation via Box::into_raw
//      in the Ok/Err/Some wrapper fns in value_ffi.rs)
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Result Type FFI Functions for JIT
//!
//! Functions for creating and manipulating Result types (Ok/Err) in JIT-compiled code.
//!
//! ## Retired Arc-shape producers & remaining legacy consumers
//!
//! W88A retires the JIT-side `jit_v2_make_result_ok` / `_err` /
//! `jit_v2_make_option_some` / `_none` producers as active allocation paths.
//! Normal VM execution now uses schema-backed `__Result` / `__Option`
//! `TypedObjectStorage` via `result_option_carrier`; the JIT has no helper ABI
//! yet that can build those typed objects from a statically stamped schema id.
//! Until that ABI lands, the MIR `EnumStore` consumer deopts before emitting
//! these imports, and these FFI bodies fail closed before allocating
//! `Arc<ResultData>` / `Arc<OptionData>`.
//!
//! The predicate + extraction helpers `jit_arc_result_is_ok` / `_is_err` /
//! `jit_arc_result_payload` / `jit_arc_option_is_some` / `_is_none` /
//! `jit_arc_option_payload` remain legacy consumers for compatibility slots
//! that can still arrive from snapshot/wire policy or old tests. They read from
//! the `*const ResultData` / `*const OptionData` borrow directly — no NaN-box
//! tag decode, no `is_heap_kind` probe (§2.7.7 #4 / #7 forbidden per CLAUDE.md
//! "Forbidden code" — runtime tag_bits dispatch deleted with the W-series).
//!
//! The legacy `jit_make_ok` / `_err` / `_some` + `jit_is_ok` / etc. above are
//! retained as Rust-side compatibility helpers for old boundary conversion/tests.
//! They are NOT registered as Cranelift imports, are absent from `FFIFuncRefs`,
//! and are NOT called from the MIR EnumStore consumer — the producers below are
//! the §2.7.5 stamp-at-compile-time path.

use super::jit_kinds::*;
use super::value_ffi::*;
use shape_value::heap_value::{OptionData, ResultData};
use std::sync::Arc;

// ============================================================================
// Result Type Creation
// ============================================================================

/// Create an Ok result wrapping the inner value
pub extern "C" fn jit_make_ok(inner_bits: u64) -> u64 {
    if tracing::enabled!(target: "shape_jit", tracing::Level::TRACE) {
        let kind = super::value_ffi::heap_kind(inner_bits);
        tracing::trace!(
            target: "shape_jit",
            inner = inner_bits,
            inner_kind = ?kind,
            "make_ok",
        );
    }
    box_ok(inner_bits)
}

/// Create an Err result wrapping the inner value
pub extern "C" fn jit_make_err(inner_bits: u64) -> u64 {
    box_err(inner_bits)
}

// ============================================================================
// Result Type Checking
// ============================================================================

/// Check if a value is Ok (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_ok(bits: u64) -> u64 {
    if is_ok_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Check if a value is Err (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_err(bits: u64) -> u64 {
    if is_err_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Check if a value is any Result type (Ok or Err)
pub extern "C" fn jit_is_result(bits: u64) -> u64 {
    if is_result_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

// ============================================================================
// Result Type Unwrapping
// ============================================================================

/// Unwrap an Ok value, returning the inner value.
/// Consumes the Ok wrapper (decrements refcount, frees if last reference).
/// If not Ok, returns TAG_NULL.
pub extern "C" fn jit_unwrap_ok(bits: u64) -> u64 {
    if is_ok_tag(bits) {
        // Read the inner u64 payload from the `UnifiedValue<u64>` wrapper
        // before freeing the wrapper. The wrapper carries a single inner
        // u64 (per `box_ok` in `value_ffi.rs`); freeing the `UnifiedValue<u64>`
        // does not touch the inner payload — the caller owns it on return.
        let inner = unsafe { unbox_result_inner(bits) };
        let ptr = unbox_heap_pointer(bits);
        if !ptr.is_null() {
            unsafe {
                UnifiedValue::<u64>::heap_drop(ptr as u64);
            }
        }
        inner
    } else {
        TAG_NULL
    }
}

/// Unwrap an Err value, returning the inner value.
/// Consumes the Err wrapper (frees the wrapper, caller owns inner).
/// If not Err, returns TAG_NULL.
pub extern "C" fn jit_unwrap_err(bits: u64) -> u64 {
    if is_err_tag(bits) {
        let inner = unsafe { unbox_result_inner(bits) };
        let ptr = unbox_heap_pointer(bits);
        if !ptr.is_null() {
            unsafe {
                UnifiedValue::<u64>::heap_drop(ptr as u64);
            }
        }
        inner
    } else {
        TAG_NULL
    }
}

/// Unwrap Ok or return default value
/// If Ok, returns the inner value; otherwise returns the default
pub extern "C" fn jit_unwrap_or(bits: u64, default_bits: u64) -> u64 {
    if is_ok_tag(bits) {
        unsafe { unbox_result_inner(bits) }
    } else {
        default_bits
    }
}

// ============================================================================
// Result Type Transformation
// ============================================================================

/// Map over Ok value - if Ok, applies function and returns new Ok
/// This is a simplified version that just returns the inner value for now
/// (full map support would require function call machinery)
pub extern "C" fn jit_result_inner(bits: u64) -> u64 {
    if is_ok_tag(bits) || is_err_tag(bits) {
        unsafe { unbox_result_inner(bits) }
    } else {
        bits
    }
}

// ============================================================================
// Option Type Functions
// ============================================================================

/// Create a Some value wrapping the inner value
pub extern "C" fn jit_make_some(inner_bits: u64) -> u64 {
    box_some(inner_bits)
}

/// Check if a value is Some (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_some(bits: u64) -> u64 {
    if is_some_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Check if a value is None (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
pub extern "C" fn jit_is_none(bits: u64) -> u64 {
    if is_none_tag(bits) {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Unwrap a Some value, returning the inner value.
/// Consumes the Some wrapper (frees the wrapper, caller owns inner).
/// If not Some, returns TAG_NULL.
pub extern "C" fn jit_unwrap_some(bits: u64) -> u64 {
    if is_some_tag(bits) {
        let inner = unsafe { unbox_some_inner(bits) };
        let ptr = unbox_heap_pointer(bits);
        if !ptr.is_null() {
            unsafe {
                UnifiedValue::<u64>::heap_drop(ptr as u64);
            }
        }
        inner
    } else {
        TAG_NULL
    }
}

// ============================================================================
// Legacy Result/Option accessors plus retired producer ABI backstops.
// ADR-006 §2.7.17 / Q18; W88A old-producer containment.
// ============================================================================
//
// The four producer symbols remain registered so stale `FFIFuncRefs` resolve to
// this explicit surface instead of a missing-symbol crash. Normal JIT lowering
// must not reach them; `mir_compiler/statements.rs` deopts Result/Option
// `EnumStore` before emitting the call. The replacement ABI must build
// schema-backed `__Result` / `__Option` typed objects with a statically known
// schema id; this FFI signature lacks that context.

#[cold]
#[track_caller]
fn retired_result_option_producer_surface(func_name: &str) -> ! {
    panic!(
        "SURFACE: {func_name} is retired by W88A. JIT Result/Option construction \
         must deopt before allocation or use a future schema-backed \
         __Result/__Option TypedObject ABI with statically known schema ids; \
         refusing to allocate old Arc<ResultData>/Arc<OptionData> carriers."
    );
}

/// Retired producer backstop. Reaching this function means the MIR `EnumStore`
/// deopt gate failed or a stale direct FFI reference was invoked.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_result_ok(_payload_bits: u64, _payload_kind_code: u8) -> u64 {
    retired_result_option_producer_surface("jit_v2_make_result_ok")
}

/// Retired producer backstop. See `jit_v2_make_result_ok`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_result_err(_payload_bits: u64, _payload_kind_code: u8) -> u64 {
    retired_result_option_producer_surface("jit_v2_make_result_err")
}

/// Retired producer backstop. See `jit_v2_make_result_ok`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_option_some(_payload_bits: u64, _payload_kind_code: u8) -> u64 {
    retired_result_option_producer_surface("jit_v2_make_option_some")
}

/// Retired producer backstop. See `jit_v2_make_result_ok`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_option_none() -> u64 {
    retired_result_option_producer_surface("jit_v2_make_option_none")
}

/// Read `is_ok` from an `Arc<ResultData>` pointer. Returns `1` for Ok, `0`
/// otherwise (including the null-bits guard). **Borrows** the inner — does
/// NOT consume or retain a strong-count share. The caller's slot continues
/// to own the Arc share.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<ResultData>) as u64` per the
/// §2.7.7 stack kind label `Ptr(HeapKind::Result)`. After W88A this is a
/// legacy compatibility consumer only; active JIT producers are fail-closed.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_result_is_ok(bits: u64) -> u8 {
    if bits == 0 {
        return 0;
    }
    let r: &ResultData = unsafe { &*(bits as *const ResultData) };
    if r.is_ok { 1 } else { 0 }
}

/// Read `is_err` from an `Arc<ResultData>` pointer (negation of `is_ok`).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_result_is_err(bits: u64) -> u8 {
    if bits == 0 {
        return 0;
    }
    let r: &ResultData = unsafe { &*(bits as *const ResultData) };
    if r.is_ok { 0 } else { 1 }
}

/// Extract the inner payload bits from an `Arc<ResultData>` and bump its
/// strong-count share so the returned bits are an OWNED slot the caller can
/// install at its destination. The wrapper Arc continues to own its own
/// inner share via `r.payload.clone()` — when the wrapper Drops later, the
/// wrapper-owned inner share will be retired too. The returned share is
/// independent (the §2.7.17 receiver-recovery soundness rule: clone the
/// inner share, transfer it via `mem::forget`).
///
/// Caller's slot must carry the payload's kind label from the legacy carrier's
/// embedded `KindedSlot`; new JIT construction must deopt until the
/// schema-backed typed-object ABI can provide this without old carriers.
///
/// SAFETY: same construction-side contract as `jit_arc_result_is_ok`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_result_payload(bits: u64) -> u64 {
    if bits == 0 {
        return 0;
    }
    let r: &ResultData = unsafe { &*(bits as *const ResultData) };
    // Clone the payload share. KindedSlot::Clone is kind-aware (per
    // ADR-006 §2.7.6) and bumps the inner refcount when the payload is a
    // heap kind (`String` / `Ptr(HeapKind::*)`); scalar kinds are a copy.
    let payload_clone = r.payload.clone();
    let raw = payload_clone.slot.raw();
    // Transfer the share to the caller: forget the local so its Drop
    // doesn't retire the share we just minted.
    std::mem::forget(payload_clone);
    raw
}

/// Read `is_some` from an `Arc<OptionData>` pointer. Mirror of
/// `jit_arc_result_is_ok` for the Option carrier.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_option_is_some(bits: u64) -> u8 {
    if bits == 0 {
        return 0;
    }
    let o: &OptionData = unsafe { &*(bits as *const OptionData) };
    if o.is_some { 1 } else { 0 }
}

/// Read `is_none` from an `Arc<OptionData>` pointer (negation of `is_some`).
/// Treats null bits as "not a valid Option pointer" → returns `0`
/// (so a downstream caller doesn't enter the None arm on garbage bits).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_option_is_none(bits: u64) -> u8 {
    if bits == 0 {
        return 0;
    }
    let o: &OptionData = unsafe { &*(bits as *const OptionData) };
    if o.is_some { 0 } else { 1 }
}

/// Extract the inner payload bits from an `Arc<OptionData>`. Same shape /
/// contract as `jit_arc_result_payload`. Callers must have proven
/// `is_some == true` via `jit_arc_option_is_some` before calling (the
/// EnumTest → EnumPayload control-flow pair guarantees this); calling on
/// a None carrier returns the inner zero-bits Bool placeholder — harmless
/// but not meaningful.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_option_payload(bits: u64) -> u64 {
    if bits == 0 {
        return 0;
    }
    let o: &OptionData = unsafe { &*(bits as *const OptionData) };
    let payload_clone = o.payload.clone();
    let raw = payload_clone.slot.raw();
    std::mem::forget(payload_clone);
    raw
}

/// Retain (clone) an `Arc<ResultData>` strong-count share. Bumps the
/// standard Rust Arc refcount at offset -16 of the `Arc::into_raw` pointer
/// via `Arc::increment_strong_count::<ResultData>` — NOT the W-series
/// `UnifiedValue<T>` refcount at offset 4 (`jit_arc_retain`'s shape).
///
/// W12-jit-result-option-trinity (Phase 3 cluster-0 Round 7A, 2026-05-12),
/// retained as a legacy consumer after W88A. The legacy `jit_arc_retain`
/// would write a U32 fetch_add at the wrong
/// offset of `Arc<ResultData>` — corrupting `payload.slot.0`'s high 32
/// bits with the spurious "refcount". The kinded retain operates on the
/// correct refcount location via `Arc::increment_strong_count::<T>` per
/// the Rust standard library Arc contract.
///
/// SAFETY: `bits` must be legacy `Arc::into_raw(Arc<ResultData>) as u64`.
/// Null is silently no-op'd.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_result_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const ResultData);
    }
}

/// Release an `Arc<ResultData>` strong-count share. Mirrors
/// `jit_arc_result_retain`'s decrement — uses
/// `Arc::decrement_strong_count::<ResultData>` per Rust Arc contract.
/// Reaching refcount zero runs `ResultData::Drop` which retires the
/// inner `KindedSlot::Drop` (kind-aware per §2.7.6 / Q8).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_result_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const ResultData);
    }
}

/// Retain (clone) an `Arc<OptionData>` strong-count share. Mirror of
/// `jit_arc_result_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_option_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const OptionData);
    }
}

/// Release an `Arc<OptionData>` strong-count share. Mirror of
/// `jit_arc_result_release`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_option_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const OptionData);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 5 Result/Option round-trip tests DELETED (W12-deleted-valuewordshape-
    // tests-rewrite, 2026-05-12): `test_result_ok_roundtrip`,
    // `test_result_err_roundtrip`, `test_unwrap_or_with_ok`,
    // `test_option_some_roundtrip`, `test_result_inner`.
    //
    // All five asserted that JIT-internal Result/Option helpers
    // (`jit_make_ok` / `jit_is_ok` / `jit_unwrap_ok` and siblings)
    // round-trip an inner value. Under ADR-006 §2.7.5 the producers
    // `box_ok` / `box_err` / `box_some` return raw `Box::into_raw(
    // UnifiedValue<u64>) as u64` (no NaN-box tag bits). The consumers
    // `is_ok_tag` / `is_err_tag` / `is_some_tag` call `is_heap_kind(bits,
    // HK_OK)` etc., which gates on `is_heap(bits) -> is_tagged(bits)` —
    // returns false for raw pointers. Every `jit_is_*` returns
    // `TAG_BOOL_FALSE` and every `jit_unwrap_*` returns `TAG_NULL` on
    // the producers' output.
    //
    // Same production-code consumer migration gap as
    // `test_jit_typed_object_ffi`: the JIT-internal Result/Option carrier
    // helpers are in the deleted-tag-bit-dispatch family. The consumers
    // must migrate to read the `HK_OK`/`HK_ERR`/`HK_SOME` prefix at
    // offset 0 of the allocation via `read_heap_kind` (per §2.7.5 "*not*
    // tag-bit dispatch — it reads a field from a heap-resident struct that
    // the producing call placed there"). NOT a deleted ValueWord-shape
    // assertion the test got wrong.
    //
    // Legacy typed-Arc analog kept only for compatibility surfaces:
    // `KindedSlot::from_result(Arc<ResultData>)` /
    // `KindedSlot::from_option(Arc<OptionData>)` per ADR-006 §2.7.17 /
    // Q18 (Wave 14 W14-variant-codegen). The carrier shape is
    // `Arc<ResultData>` / `Arc<OptionData>` with an inner `payload:
    // KindedSlot`, NOT the JIT-internal `UnifiedValue<u64>` shape these
    // tests exercise. Canonical VM execution now uses schema-backed
    // `__Result` / `__Option` via `result_option_carrier`; old carrier
    // coverage lives in compatibility tests. The two surviving green tests in
    // this module (`test_unwrap_or_with_err`, `test_option_none`,
    // `test_non_result_values`) cover the early-return branches that
    // don't require producer→consumer round-trip.
    //
    // The JIT-internal Result/Option helpers will be re-tested once a
    // future sub-cluster migrates the consumers to use `read_heap_kind`
    // — or, more likely, once the JIT codegen migrates to emit
    // `HeapKind::Result` / `HeapKind::Option` Arc handles directly per
    // §2.7.5 (eliminating the `UnifiedValue<u64>`-wrapped intermediate
    // shape entirely).

    #[test]
    fn test_unwrap_or_with_err() {
        let err_result = jit_make_err(box_number(-1.0));
        let default = box_number(999.0);

        let result = jit_unwrap_or(err_result, default);
        assert_eq!(unbox_number(result), 999.0);
    }

    #[test]
    fn test_option_none() {
        // TAG_NULL represents None
        assert_eq!(jit_is_none(TAG_NULL), TAG_BOOL_TRUE);
        assert_eq!(jit_is_some(TAG_NULL), TAG_BOOL_FALSE);
    }

    #[test]
    fn test_non_result_values() {
        // Regular numbers should not be results
        let num = box_number(42.0);
        assert_eq!(jit_is_result(num), TAG_BOOL_FALSE);
        assert_eq!(jit_is_ok(num), TAG_BOOL_FALSE);
        assert_eq!(jit_is_err(num), TAG_BOOL_FALSE);
    }

    // `test_result_inner` was here — DELETED per the block above (same
    // production-code consumer migration gap: `jit_result_inner` gates on
    // `is_ok_tag(bits) || is_err_tag(bits)` which fails for raw producer
    // pointers, returning the bits unchanged instead of the unwrapped
    // inner).

    // ── Legacy Result/Option carrier consumer tests ─────────────────────
    //
    // W88A retires the four `jit_v2_make_result_*` / `jit_v2_make_option_*`
    // producers. Do not add producer round-trip tests here; they would
    // reintroduce the old `Arc<ResultData>` / `Arc<OptionData>` allocation
    // path this patch deliberately fails closed.

    use super::super::stack_kind_code;
    use shape_value::heap_value::HeapKind;

    #[test]
    #[should_panic(expected = "jit_v2_make_result_ok is retired by W88A")]
    fn retired_result_option_producer_surface_is_a_hard_stop() {
        // Call the private helper, not the extern "C" wrapper: unwinding out
        // of an extern "C" function aborts the test process. The wrapper's
        // body is a direct tail call to this helper.
        retired_result_option_producer_surface("jit_v2_make_result_ok");
    }

    #[test]
    fn arc_result_null_bits_safe() {
        // The null-bits guard prevents segfaults on garbage producer output.
        // Returns 0 for both predicates — caller's match dispatch picks the
        // implicit "neither arm matched" path.
        assert_eq!(jit_arc_result_is_ok(0), 0);
        assert_eq!(jit_arc_result_is_err(0), 0);
        assert_eq!(jit_arc_result_payload(0), 0);
        assert_eq!(jit_arc_option_is_some(0), 0);
        assert_eq!(jit_arc_option_is_none(0), 0);
        assert_eq!(jit_arc_option_payload(0), 0);
    }

    #[test]
    fn arc_carrier_kind_label_matches_producer() {
        // The producer's kind label matches Wave 14 W14-variant-codegen.
        // Ord lookup ensures the stack_kind_code table stays in lockstep
        // with the HeapKind ordinal table per CLAUDE.md "Renames to refuse
        // on sight" — the kind-blind producer that doesn't stamp kind is
        // the W-series defection-attractor shape.
        let result_code = stack_kind_code::encode(shape_value::NativeKind::Ptr(HeapKind::Result));
        let option_code = stack_kind_code::encode(shape_value::NativeKind::Ptr(HeapKind::Option));
        assert_eq!(
            stack_kind_code::decode(result_code),
            Some(shape_value::NativeKind::Ptr(HeapKind::Result))
        );
        assert_eq!(
            stack_kind_code::decode(option_code),
            Some(shape_value::NativeKind::Ptr(HeapKind::Option))
        );
    }
}
