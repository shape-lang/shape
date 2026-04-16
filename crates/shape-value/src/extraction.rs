//! Centralized extraction helpers for ValueWord values.
//!
//! These `require_*` functions provide a single source of truth for extracting
//! typed values from ValueWord, returning descriptive VMError on type mismatch.
//! They replace scattered ad-hoc patterns like:
//!
//! ```ignore
//! let s = nb.as_str()
//!     .ok_or_else(|| VMError::TypeError { expected: "string", got: "other" })?;
//! ```

use crate::context::VMError;
use crate::datatable::DataTable;
use crate::slot::ValueSlot;
use crate::value::Upvalue;
use crate::value_word::{ArrayView, ValueWord, ValueWordExt};
use std::sync::Arc;

/// Extract a string reference from a ValueWord, or return a type error.
#[inline]
pub fn require_string(nb: &ValueWord) -> Result<&str, VMError> {
    nb.as_str()
        .ok_or_else(|| VMError::type_mismatch("string", nb.type_name()))
}

/// Extract an Arc<String> reference from a ValueWord, or return a type error.
#[inline]
pub fn require_arc_string(nb: &ValueWord) -> Result<&Arc<String>, VMError> {
    nb.as_arc_string()
        .ok_or_else(|| VMError::type_mismatch("string", nb.type_name()))
}

/// Extract an f64 from a ValueWord, or return a type error.
///
/// This only extracts inline f64 values; it does not coerce integers.
#[inline]
pub fn require_f64(nb: &ValueWord) -> Result<f64, VMError> {
    nb.as_f64()
        .ok_or_else(|| VMError::type_mismatch("number", nb.type_name()))
}

/// Extract a number (f64 or i64 coerced to f64) from a ValueWord, or return a type error.
#[inline]
pub fn require_number(nb: &ValueWord) -> Result<f64, VMError> {
    nb.as_number_coerce()
        .ok_or_else(|| VMError::type_mismatch("number", nb.type_name()))
}

/// Extract an i64 from a ValueWord, or return a type error.
///
/// This only extracts inline i48 values; it does not coerce floats.
#[inline]
pub fn require_int(nb: &ValueWord) -> Result<i64, VMError> {
    nb.as_i64()
        .ok_or_else(|| VMError::type_mismatch("int", nb.type_name()))
}

/// Extract a bool from a ValueWord, or return a type error.
#[inline]
pub fn require_bool(nb: &ValueWord) -> Result<bool, VMError> {
    nb.as_bool()
        .ok_or_else(|| VMError::type_mismatch("bool", nb.type_name()))
}

/// Extract an array view from a ValueWord, or return a type error.
///
/// Handles all array variants: Generic, Int, Float, and Bool.
#[inline]
pub fn require_array(nb: &ValueWord) -> Result<ArrayView<'_>, VMError> {
    nb.as_any_array()
        .ok_or_else(|| VMError::type_mismatch("array", nb.type_name()))
}

/// Extract a DataTable reference from a ValueWord, or return a type error.
#[inline]
pub fn require_datatable(nb: &ValueWord) -> Result<&Arc<DataTable>, VMError> {
    nb.as_datatable()
        .ok_or_else(|| VMError::type_mismatch("datatable", nb.type_name()))
}

/// Extract TypedObject fields (schema_id, slots, heap_mask) from a ValueWord, or return a type error.
#[inline]
pub fn require_typed_object(nb: &ValueWord) -> Result<(u64, &[ValueSlot], u64), VMError> {
    nb.as_typed_object()
        .ok_or_else(|| VMError::type_mismatch("object", nb.type_name()))
}

/// Extract Closure fields (function_id, upvalues) from a ValueWord, or return a type error.
#[inline]
pub fn require_closure(nb: &ValueWord) -> Result<(u16, &[Upvalue]), VMError> {
    nb.as_closure()
        .ok_or_else(|| VMError::type_mismatch("closure", nb.type_name()))
}

/// Convert a ValueWord to a display string.
///
/// Uses the Display impl which dispatches through HeapValue.
#[inline]
pub fn nb_to_display_string(nb: &ValueWord) -> String {
    format!("{}", crate::value_word::ValueWordDisplay(*nb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_require_string_ok() {
        let nb = ValueWord::from_string(Arc::new("hello".to_string()));
        assert_eq!(require_string(&nb).unwrap(), "hello");
    }

    #[test]
    fn test_require_string_err() {
        let nb = ValueWord::from_f64(42.0);
        let err = require_string(&nb).unwrap_err();
        match err {
            VMError::TypeError { expected, got } => {
                assert_eq!(expected, "string");
                assert_eq!(got, "number");
            }
            other => panic!("expected TypeError, got {:?}", other),
        }
    }

    #[test]
    fn test_require_number_coerce() {
        // f64 works
        let nb = ValueWord::from_f64(3.14);
        assert_eq!(require_number(&nb).unwrap(), 3.14);

        // i64 is coerced to f64
        let nb = ValueWord::from_i64(42);
        assert_eq!(require_number(&nb).unwrap(), 42.0);

        // string fails
        let nb = ValueWord::from_string(Arc::new("x".to_string()));
        assert!(require_number(&nb).is_err());
    }

    #[test]
    fn test_require_int() {
        let nb = ValueWord::from_i64(99);
        assert_eq!(require_int(&nb).unwrap(), 99);

        let nb = ValueWord::from_f64(1.5);
        assert!(require_int(&nb).is_err());
    }

    #[test]
    fn test_require_bool() {
        let nb = ValueWord::from_bool(true);
        assert_eq!(require_bool(&nb).unwrap(), true);

        let nb = ValueWord::from_i64(1);
        assert!(require_bool(&nb).is_err());
    }

    #[test]
    fn test_require_array() {
        let nb = ValueWord::from_array(Arc::new(vec![ValueWord::from_i64(1)]));
        assert_eq!(require_array(&nb).unwrap().len(), 1);

        let nb = ValueWord::from_f64(1.0);
        assert!(require_array(&nb).is_err());
    }

    #[test]
    fn test_require_typed_object() {
        let nb = ValueWord::from_f64(1.0);
        let err = require_typed_object(&nb).unwrap_err();
        match err {
            VMError::TypeError { expected, got } => {
                assert_eq!(expected, "object");
                assert_eq!(got, "number");
            }
            other => panic!("expected TypeError, got {:?}", other),
        }
    }

    #[test]
    fn test_nb_to_display_string() {
        assert_eq!(nb_to_display_string(&ValueWord::from_f64(3.14)), "3.14");
        assert_eq!(nb_to_display_string(&ValueWord::from_i64(42)), "42");
        assert_eq!(nb_to_display_string(&ValueWord::from_bool(true)), "true");
        assert_eq!(nb_to_display_string(&ValueWord::none()), "none");
        let s = ValueWord::from_string(Arc::new("hello".to_string()));
        assert_eq!(nb_to_display_string(&s), "hello");
    }
}
