//! Extraction helpers for method handlers.
//!
//! Wave-β R-misc migration (per ADR-006 §2.7 + playbook §10
//! `D-array-detect` / `D-v2-array-detect` template): the legacy v1
//! ValueWord-shaped helpers (`require_any_array_arg`,
//! `nb_to_string_coerce`, `check_arg_count`) depended on a stack of
//! deleted carriers — `shape_value::ValueWord` / `ValueWordExt` (the v1
//! dynamic-tag word, deleted), `shape_value::ArrayView` (the v1
//! polymorphic array view, deleted alongside `as_any_array`), and the
//! `as_str` / `as_f64` / `as_i64` / `as_bool` / `is_none` accessors on
//! `&ValueWord` (all deleted by §2.7.6 — heap dispatch goes through
//! `KindedSlot::slot.as_heap_value()` + `HeapValue` match per Q8).
//!
//! - `require_any_array_arg(&[ValueWord]) -> Result<ArrayView, _>` —
//!   deleted (no caller is reachable until D-array-detect /
//!   D-v2-array-detect rebuild lands the kinded array dispatch surface).
//!   Callers in `array_sort.rs` / `array_transform.rs` are pre-existing
//!   Wave-α-broken on `nb_to_string_coerce`; the array-view accessor
//!   itself has zero live callers.
//! - `nb_to_string_coerce(&ValueWord) -> String` — deleted. Callers in
//!   `array_sort.rs` / `array_transform.rs` reach this via `array.iter()`
//!   on the deleted v1 array view; both consumer files are pre-existing
//!   Wave-α-broken and migrate together with the kinded array surface.
//! - `check_arg_count(&[ValueWord], usize, &str, &str) -> Result<(), _>`
//!   — deleted. Zero live callers (verified via grep across `crates/`).
//!
//! `type_mismatch_error(method_name: &str, expected_type: &str) -> VMError`
//! is preserved verbatim — its signature carries no `ValueWord`
//! dependency and it is the only helper in this module with live
//! callers (~80 sites across `priority_queue_methods.rs`,
//! `deque_methods.rs`, and other `objects/*_methods.rs` files).

use shape_value::VMError;

/// Produce a `VMError::RuntimeError` of the form
/// `"<method> called on non-<expected_type> value"`.
///
/// This consolidates the ~77 occurrences of that pattern across the
/// collection method handlers. Used by every `*_methods.rs` consumer
/// in `executor/objects/`.
#[inline]
pub(crate) fn type_mismatch_error(method_name: &str, expected_type: &str) -> VMError {
    VMError::RuntimeError(format!(
        "{} called on non-{} value",
        method_name, expected_type
    ))
}

// `check_arg_count`, `require_any_array_arg`, `nb_to_string_coerce` were
// deleted as part of the Wave-β R-misc strict-typing migration. Their
// signatures took `&[ValueWord]` / `&ValueWord` / returned `ArrayView`,
// all of which were deleted with the v1 dynamic-tag carrier. The
// post-§2.7.6 replacement dispatches on `KindedSlot.kind()` and uses
// the per-variant `KindedSlot::as_*` accessors plus
// `slot.as_heap_value()` for heap-arm dispatch. No live caller depends
// on the deleted helpers; rebuild as needed once the kinded array
// dispatch surface lands (D-array-detect / D-v2-array-detect).
