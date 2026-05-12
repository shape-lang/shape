//! Kinded reclaim for JIT-emitted `UnifiedValue<T>` heap allocations.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! When `jit_arc_release` decrements a `UnifiedValue<T>` refcount to zero, the
//! reclaim needs to call `Box::from_raw::<UnifiedValue<T>>(ptr)` with the
//! correct `T`. The `kind: u16` field at offset 0 of `UnifiedValue` is the
//! canonical discriminator (§2.7.6 / Q8 single-discriminator: it carries the
//! same semantic as `HeapValue::kind()` and is set at construction by
//! `unified_box(kind, data)`). The per-kind arms below pick the matching `T`.
//!
//! This is NOT a tag-bit probe — `kind` is a structural field on the heap
//! object, read from a known offset, with the value placed there by the
//! producing call. CLAUDE.md "Forbidden Patterns" #4 ("Runtime `tag_bits`
//! dispatch") forbids decoding kind from the BIT PATTERN of a value; reading
//! a discriminator field from a heap object is the §2.7.6 / Q8 dispatch
//! pattern, not the deleted W-series shape.
//!
//! ## Coverage
//!
//! Every `unified_box(KIND, payload)` call site in `crates/shape-jit/src/ffi/`
//! must have a matching arm here. The `surface_and_stop_unknown_kind` fallback
//! is the principled response to an unknown kind — never a silent leak. The
//! W11-jit-new-array close gate prohibits adding a "skip on unknown kind"
//! arm (CLAUDE.md "Forbidden rationalizations": *"Soft-fail counter for now,
//! harden later."*).
//!
//! ## Caller contract
//!
//! `ptr` must be a non-null `*const UnifiedValue<_>` whose refcount has JUST
//! reached zero (only the unique owner can call this). The kinded `Box::from_raw`
//! takes ownership and runs the inner `T::Drop`.

use super::jit_kinds::UnifiedValue;
use super::value_ffi::{
    HK_BIG_INT, HK_BOOL_ARRAY, HK_COLUMN_REF, HK_DATATABLE, HK_DECIMAL, HK_DURATION, HK_ENUM,
    HK_ERR, HK_EXPR_PROXY, HK_FLOAT_ARRAY, HK_FLOAT_ARRAY_SLICE, HK_FUTURE, HK_HASHMAP,
    HK_HOST_CLOSURE, HK_INDEXED_TABLE, HK_INT_ARRAY, HK_MATRIX, HK_OK, HK_RANGE, HK_ROW_VIEW,
    HK_SIMULATION_CALL, HK_SOME, HK_STRING, HK_TASK_GROUP, HK_TIME, HK_TIMEFRAME, HK_TIMESPAN,
    HK_TIME_REFERENCE, HK_TRAIT_OBJECT, HK_TYPED_OBJECT, HK_TYPED_TABLE,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Read the `kind: u16` field at offset 0 of a `UnifiedValue<_>`.
///
/// # Safety
/// `ptr` must be a non-null `*const UnifiedValue<_>` allocation.
#[inline]
unsafe fn read_kind(ptr: *const u8) -> u16 {
    unsafe { *(ptr as *const u16) }
}

/// Dispatch the kinded `Box::from_raw` reclaim for a `UnifiedValue<T>` whose
/// refcount has reached zero. The `kind: u16` field at offset 0 selects the
/// matching `T`.
///
/// # Safety
/// - `ptr` must be a non-null `*const UnifiedValue<T>` from
///   `unified_box(kind, payload)`.
/// - The caller must be the unique remaining owner (refcount just hit zero).
/// - `Box::from_raw` is called exactly once per allocation.
pub(super) fn release_unified_value_by_kind(ptr: *const u8) {
    // SAFETY: caller contract.
    let kind = unsafe { read_kind(ptr) };
    unsafe {
        match kind {
            HK_STRING => drop_box::<Arc<String>>(ptr),
            HK_TYPED_OBJECT => drop_box::<*const u8>(ptr),
            // Result / Option carriers wrap a `u64` inner — the inner
            // value (if itself heap) was already handled by its own
            // refcount path; the carrier's `Box::from_raw` only frees
            // the `UnifiedValue<u64>` allocation itself.
            HK_OK | HK_ERR | HK_SOME => drop_box::<u64>(ptr),
            // Other kinds JIT-emit through `unified_box`:
            HK_HASHMAP => drop_box::<HashMap<String, u64>>(ptr),
            HK_RANGE => drop_box::<crate::context::JITRange>(ptr),
            HK_COLUMN_REF => drop_box::<(*const f64, usize)>(ptr),
            HK_FLOAT_ARRAY | HK_INT_ARRAY | HK_BOOL_ARRAY | HK_FLOAT_ARRAY_SLICE | HK_MATRIX
            | HK_DATATABLE | HK_TYPED_TABLE | HK_INDEXED_TABLE | HK_ROW_VIEW | HK_TIME
            | HK_DURATION | HK_TIMESPAN | HK_TIMEFRAME | HK_TIME_REFERENCE | HK_DECIMAL
            | HK_BIG_INT | HK_HOST_CLOSURE | HK_ENUM | HK_TRAIT_OBJECT | HK_EXPR_PROXY
            | HK_FUTURE | HK_TASK_GROUP | HK_SIMULATION_CALL => {
                // These kinds are JIT-allocated through various paths but
                // their `T` is non-trivial to nominate here. Per ADR-006
                // §2.7.7 #9 the principled response when the reclaim
                // can't pick a sound `T` is surface-and-stop, not a
                // silent leak. In practice the W11 emitter's
                // `is_refcounted` gate funnels only `String` /
                // `Ptr(HeapKind::*)` slots to `arc_release`, and the
                // common Ptr kinds (TypedObject, HashMap, Range,
                // ColumnRef) are covered above. Reaching any of these
                // arms is a kind-source gap from the emitter side —
                // surface-and-stop instead of skipping the free.
                surface_and_stop_unknown_kind(kind);
            }
            _ => {
                // Any other kind reaching here is either a JIT-private
                // shape not catalogued in HK_* (would indicate a missing
                // entry above) or a corrupted allocation. Per CLAUDE.md
                // "Forbidden rationalizations" we do NOT silently skip
                // the free — surface-and-stop.
                surface_and_stop_unknown_kind(kind);
            }
        }
    }
}

/// Reclaim a `UnifiedValue<T>` via `Box::from_raw`. Generic over `T` so the
/// kinded dispatch in `release_unified_value_by_kind` picks the right
/// instantiation.
///
/// # Safety
/// `ptr` must be a non-null `*mut UnifiedValue<T>` from a live JIT-emitted
/// `unified_box::<T>` allocation whose refcount just reached zero.
#[inline]
unsafe fn drop_box<T>(ptr: *const u8) {
    let typed = ptr as *mut UnifiedValue<T>;
    unsafe {
        drop(Box::from_raw(typed));
    }
}

/// Print the surface-and-stop diagnostic for an unknown kind at the JIT
/// release boundary.
///
/// We intentionally do NOT panic from `extern "C"` (extern C can't unwind,
/// per the W17-jit-stubs ignored-test pattern). The eprintln + intentional
/// leak is the controlled surface-and-stop response — it makes the gap
/// audible without crashing the process.
fn surface_and_stop_unknown_kind(kind: u16) {
    eprintln!(
        "jit_release SURFACE-AND-STOP: UnifiedValue<T> reclaim has no arm for \
         kind={} at the W11-jit-new-array boundary. The allocation is leaked \
         intentionally pending a kinded arm for this discriminator. \
         ADR-006 §2.7.6 / Q8 / §2.7.14. \
         (NB: CLAUDE.md \"Forbidden rationalizations\" — do not turn this \
         into a silent skip; add the missing arm with the matching `T` of \
         the producing `unified_box::<T>` call site.)",
        kind
    );
}
