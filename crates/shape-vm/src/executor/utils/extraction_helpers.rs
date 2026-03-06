//! Extraction helpers to reduce duplication across method handlers.
//!
//! Common patterns like "get array from first arg" and "coerce ValueWord to string"
//! are centralized here so every call site is a single function call.

use shape_value::{ArrayView, VMError, ValueWord};

/// Extract a unified array view from the first element of `args`.
/// Handles all array variants: generic Array, IntArray, FloatArray, BoolArray.
#[inline]
pub(crate) fn require_any_array_arg<'a>(args: &'a [ValueWord]) -> Result<ArrayView<'a>, VMError> {
    args.first()
        .ok_or(VMError::StackUnderflow)?
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })
}

/// Coerce a ValueWord value to a String representation.
///
/// Handles: string, f64, i64, bool, none → "null", fallback → Debug format.
/// This replaces the 13-line pattern found in join_str, array_sort, etc.
#[inline]
pub(crate) fn nb_to_string_coerce(nb: &ValueWord) -> String {
    if let Some(s) = nb.as_str() {
        s.to_string()
    } else if let Some(n) = nb.as_f64() {
        n.to_string()
    } else if let Some(i) = nb.as_i64() {
        i.to_string()
    } else if let Some(b) = nb.as_bool() {
        b.to_string()
    } else if nb.is_none() {
        "null".to_string()
    } else {
        format!("{:?}", nb)
    }
}
