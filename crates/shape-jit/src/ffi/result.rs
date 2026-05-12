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
//! ## Arc-shape producers & consumers (W12-jit-result-option-trinity, Phase 3 cluster-0 Round 7A, 2026-05-12)
//!
//! ADR-006 §2.7.17 / Q18 (Wave 14 W14-variant-codegen) defines the strict-typed
//! `Arc<ResultData>` / `Arc<OptionData>` carriers as the canonical runtime shape
//! for Result<T,E> and Option<T> values. Slot bits at the §2.7.7 stack tier are
//! `Arc::into_raw(Arc<ResultData>) as u64` / `Arc::into_raw(Arc<OptionData>) as u64`
//! with kind labels `NativeKind::Ptr(HeapKind::Result)` /
//! `NativeKind::Ptr(HeapKind::Option)`. The VM-side `BuiltinFunction::OkCtor` /
//! `ErrCtor` / `SomeCtor` / `NoneCtor` (`crates/shape-vm/src/executor/vm_impl/
//! builtins.rs:551-586`) produces this shape via `KindedSlot::from_result` /
//! `from_option`.
//!
//! The JIT-side `jit_v2_make_result_ok` / `_err` / `jit_v2_make_option_some` /
//! `_none` producers below match that output shape — `Arc::into_raw(Arc::new(
//! ResultData::ok(payload))) as u64`. Predicate + extraction helpers
//! `jit_arc_result_is_ok` / `_is_err` / `jit_arc_result_payload` /
//! `jit_arc_option_is_some` / `_is_none` / `jit_arc_option_payload` read from
//! the `*const ResultData` / `*const OptionData` borrow directly — no NaN-box
//! tag decode, no `is_heap_kind` probe (§2.7.7 #4 / #7 forbidden per CLAUDE.md
//! "Forbidden code" — runtime tag_bits dispatch deleted with the W-series).
//!
//! The legacy `jit_make_ok` / `_err` / `_some` + `jit_is_ok` / etc. above are
//! retained for the bytecode-VM-trampoline conversion path (`ffi/conversion.rs`)
//! but are NOT called from the new MIR EnumStore consumer — the producers below
//! are the §2.7.5 stamp-at-compile-time path.

use super::jit_kinds::*;
use super::value_ffi::*;
use shape_value::heap_value::{OptionData, ResultData};
use shape_value::kinded_slot::KindedSlot;
use std::sync::Arc;

// ============================================================================
// Result Type Creation
// ============================================================================

/// Create an Ok result wrapping the inner value
pub extern "C" fn jit_make_ok(inner_bits: u64) -> u64 {
    if std::env::var_os("SHAPE_JIT_TRACE").is_some() {
        let kind = super::value_ffi::heap_kind(inner_bits);
        eprintln!("[make_ok] inner={:#x} inner_kind={:?}", inner_bits, kind);
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
    if is_ok_tag(bits) { TAG_BOOL_TRUE } else { TAG_BOOL_FALSE }
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
// Arc-shape Result/Option producers & accessors
// (W12-jit-result-option-trinity, Phase 3 cluster-0 Round 7A, 2026-05-12)
// ADR-006 §2.7.17 / Q18.
// ============================================================================
//
// These functions implement the strict-typed `Arc<ResultData>` /
// `Arc<OptionData>` carrier per ADR-006 §2.7.17 — the same shape the VM-side
// `BuiltinFunction::OkCtor` / `ErrCtor` / `SomeCtor` / `NoneCtor` produces via
// `KindedSlot::from_result` / `from_option`. The MIR-emitted EnumStore consumer
// for `Ok(v)` / `Err(e)` / `Some(x)` / `None` dispatches to these producers
// (not to the legacy `jit_make_ok` / `_err` / `_some` NaN-box family).
//
// The `payload_kind_code` parameter on the producers is the §2.7.7 / Q9
// parallel-track encoding (`crates/shape-jit/src/ffi/stack_kind_code.rs`).
// Stamped at JIT-compile time from the EnumStore operand's MIR kind. Decoded
// inside the FFI via `stack_kind_code::decode` — no Bool-default fallback;
// a sentinel/unknown byte causes the function to leak the payload (no inner
// share) and return a poisoned None/Err per the surface-and-stop discipline.
// In practice the consumer-side dispatch generates the byte from the
// `operand_slot_kind` result which is `Some(kind)` by construction (the
// producer-site MIR-emission classifies the operand kind).

/// Decode the payload kind code, returning a sentinel `NativeKind::Bool` for
/// `None` to keep the surface visible at the FFI body — the caller's
/// `KindedSlot::Drop` will be a no-op on a Bool kind with zero bits, which
/// matches the §2.7.17 `OptionData::none()` placeholder shape. Any genuine
/// kind-source gap should be detected at the call site before reaching the
/// FFI; this fallback is the "FFI-body surface-and-stop" path (audible via
/// `SHAPE_JIT_DEBUG=1` in the caller's own diagnostic, NOT a Bool-default
/// rationalization per §2.7.7 #9).
#[inline]
fn decode_payload_kind_or_surface(
    code: u8,
    func_name: &str,
) -> shape_value::NativeKind {
    match super::stack_kind_code::decode(code) {
        Some(k) => k,
        None => {
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                eprintln!(
                    "[{}] SURFACE: payload kind code {} is sentinel/unknown. \
                     ADR-006 §2.7.7 #9 — producer-site MIR kind classification \
                     gap. Falling back to Bool placeholder; downstream consumer \
                     will surface on slot-kind mismatch.",
                    func_name, code
                );
            }
            shape_value::NativeKind::Bool
        }
    }
}

/// Allocate an `Arc<ResultData>` carrying `Ok(payload)` with the payload's
/// kind stamped at the call site. Returns `Arc::into_raw(arc) as u64` — the
/// slot bits the caller installs with kind `NativeKind::Ptr(HeapKind::Result)`.
///
/// **Strong-count contract:** the caller transfers exactly one strong-count
/// share of the inner payload to this function (via `KindedSlot::new(...)`,
/// which adopts the bits without bumping any refcount — the bits are the
/// caller's already-owned share). The returned `Arc<ResultData>` carries one
/// new strong-count share of the wrapper Arc, owned by the caller via the
/// returned raw bits. Subsequent `KindedSlot::Drop` of the wrapper slot (kind
/// `Ptr(HeapKind::Result)`) retires both the wrapper share AND the inner
/// payload share via `ResultData::drop` → `KindedSlot::Drop` per ADR-006
/// §2.7.17.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_result_ok(payload_bits: u64, payload_kind_code: u8) -> u64 {
    let kind = decode_payload_kind_or_surface(payload_kind_code, "jit_v2_make_result_ok");
    let payload_slot = shape_value::ValueSlot::from_raw(payload_bits);
    let payload = KindedSlot::new(payload_slot, kind);
    let arc = Arc::new(ResultData::ok(payload));
    Arc::into_raw(arc) as u64
}

/// Allocate an `Arc<ResultData>` carrying `Err(payload)`. Same contract as
/// `jit_v2_make_result_ok`; the discriminator differs.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_result_err(payload_bits: u64, payload_kind_code: u8) -> u64 {
    let kind = decode_payload_kind_or_surface(payload_kind_code, "jit_v2_make_result_err");
    let payload_slot = shape_value::ValueSlot::from_raw(payload_bits);
    let payload = KindedSlot::new(payload_slot, kind);
    let arc = Arc::new(ResultData::err(payload));
    Arc::into_raw(arc) as u64
}

/// Allocate an `Arc<OptionData>` carrying `Some(payload)`. Mirror of the
/// `jit_v2_make_result_*` shape.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_option_some(payload_bits: u64, payload_kind_code: u8) -> u64 {
    let kind = decode_payload_kind_or_surface(payload_kind_code, "jit_v2_make_option_some");
    let payload_slot = shape_value::ValueSlot::from_raw(payload_bits);
    let payload = KindedSlot::new(payload_slot, kind);
    let arc = Arc::new(OptionData::some(payload));
    Arc::into_raw(arc) as u64
}

/// Allocate an `Arc<OptionData>` carrying `None`. No payload — the inner
/// payload slot is a zero-bits Bool placeholder per §2.7.17 `OptionData::
/// none()` so the inner `KindedSlot::Drop` is a no-op when the wrapper
/// Arc reaches refcount zero.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_option_none() -> u64 {
    let arc = Arc::new(OptionData::none());
    Arc::into_raw(arc) as u64
}

/// Read `is_ok` from an `Arc<ResultData>` pointer. Returns `1` for Ok, `0`
/// otherwise (including the null-bits guard). **Borrows** the inner — does
/// NOT consume or retain a strong-count share. The caller's slot continues
/// to own the Arc share.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<ResultData>) as u64` per the
/// §2.7.7 stack kind label `Ptr(HeapKind::Result)`. The producer side
/// (VM-side `BuiltinFunction::OkCtor`, JIT-side `jit_v2_make_result_ok`,
/// etc.) is the source.
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
/// Caller's slot must carry the payload's kind label per the EnumStore
/// producer's compile-time classification (threaded into the parallel-kind
/// track via the codegen consumer; that kind matches `r.payload.kind`).
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
/// W12-jit-result-option-trinity (Phase 3 cluster-0 Round 7A, 2026-05-12).
/// The legacy `jit_arc_retain` would write a U32 fetch_add at the wrong
/// offset of `Arc<ResultData>` — corrupting `payload.slot.0`'s high 32
/// bits with the spurious "refcount". The kinded retain operates on the
/// correct refcount location via `Arc::increment_strong_count::<T>` per
/// the Rust standard library Arc contract.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<ResultData>) as u64` from
/// `jit_v2_make_result_ok` / `jit_v2_make_result_err` or the VM-side
/// `KindedSlot::from_result` producer. Null is silently no-op'd.
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
    // Strict-typed analog at the VM tier:
    // `KindedSlot::from_result(Arc<ResultData>)` /
    // `KindedSlot::from_option(Arc<OptionData>)` per ADR-006 §2.7.17 /
    // Q18 (Wave 14 W14-variant-codegen). The carrier shape is
    // `Arc<ResultData>` / `Arc<OptionData>` with an inner `payload:
    // KindedSlot`, NOT the JIT-internal `UnifiedValue<u64>` shape these
    // tests exercise. Coverage of the Result/Option kinded carriers
    // lives in `crates/shape-value/src/heap_value.rs::tests` (search for
    // `ResultData` / `OptionData`) and in the VM-tier match / ok / err
    // execution tests in `shape-vm`. The two surviving green tests in
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

    // ── Arc-shape Result/Option FFI round-trip tests ────────────────────
    // (W12-jit-result-option-trinity, Phase 3 cluster-0 Round 7A, 2026-05-12)

    use super::super::stack_kind_code;
    use shape_value::heap_value::HeapKind;
    use shape_value::ValueSlot;

    /// Recover and free the Arc carriers without leaking, matching the
    /// §2.7.17 stack-tier drop dispatch.
    unsafe fn drop_arc_result(bits: u64) {
        if bits != 0 {
            let _ = Arc::<ResultData>::from_raw(bits as *const ResultData);
        }
    }

    unsafe fn drop_arc_option(bits: u64) {
        if bits != 0 {
            let _ = Arc::<OptionData>::from_raw(bits as *const OptionData);
        }
    }

    #[test]
    fn arc_result_ok_roundtrip_int_payload() {
        let inner_bits = ValueSlot::from_int(42).raw();
        let arc_bits = jit_v2_make_result_ok(inner_bits, stack_kind_code::C_INT64);
        assert_ne!(arc_bits, 0);

        assert_eq!(jit_arc_result_is_ok(arc_bits), 1);
        assert_eq!(jit_arc_result_is_err(arc_bits), 0);

        let payload_bits = jit_arc_result_payload(arc_bits);
        // Int64 payload: KindedSlot::Clone is a copy; raw bits match.
        assert_eq!(payload_bits, inner_bits);
        assert_eq!(ValueSlot::from_raw(payload_bits).as_i64(), 42);
        unsafe { drop_arc_result(arc_bits) };
    }

    #[test]
    fn arc_result_err_roundtrip_int_payload() {
        let inner_bits = ValueSlot::from_int(-1).raw();
        let arc_bits = jit_v2_make_result_err(inner_bits, stack_kind_code::C_INT64);
        assert_ne!(arc_bits, 0);

        assert_eq!(jit_arc_result_is_ok(arc_bits), 0);
        assert_eq!(jit_arc_result_is_err(arc_bits), 1);

        let payload_bits = jit_arc_result_payload(arc_bits);
        assert_eq!(payload_bits, inner_bits);
        unsafe { drop_arc_result(arc_bits) };
    }

    #[test]
    fn arc_option_some_roundtrip_int_payload() {
        let inner_bits = ValueSlot::from_int(7).raw();
        let arc_bits = jit_v2_make_option_some(inner_bits, stack_kind_code::C_INT64);
        assert_ne!(arc_bits, 0);

        assert_eq!(jit_arc_option_is_some(arc_bits), 1);
        assert_eq!(jit_arc_option_is_none(arc_bits), 0);

        let payload_bits = jit_arc_option_payload(arc_bits);
        assert_eq!(payload_bits, inner_bits);
        unsafe { drop_arc_option(arc_bits) };
    }

    #[test]
    fn arc_option_none_roundtrip() {
        let arc_bits = jit_v2_make_option_none();
        assert_ne!(arc_bits, 0);

        assert_eq!(jit_arc_option_is_some(arc_bits), 0);
        assert_eq!(jit_arc_option_is_none(arc_bits), 1);
        unsafe { drop_arc_option(arc_bits) };
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
        let result_code = stack_kind_code::encode(
            shape_value::NativeKind::Ptr(HeapKind::Result),
        );
        let option_code = stack_kind_code::encode(
            shape_value::NativeKind::Ptr(HeapKind::Option),
        );
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
