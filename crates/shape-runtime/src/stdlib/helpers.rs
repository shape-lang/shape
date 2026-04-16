//! Common argument extraction helpers for stdlib module functions.
//!
//! These reduce boilerplate in module implementations by centralising
//! argument-count validation and typed argument extraction with uniform
//! error messages.

use shape_value::{ValueWord, ValueWordExt};

/// Validate that `args` has exactly `expected` elements.
///
/// Returns `Ok(())` on success, or an error string naming `fn_name` and the
/// mismatch.
pub fn check_arg_count(args: &[ValueWord], expected: usize, fn_name: &str) -> Result<(), String> {
    if args.len() != expected {
        Err(format!(
            "{}() expected {} argument{}, got {}",
            fn_name,
            expected,
            if expected == 1 { "" } else { "s" },
            args.len()
        ))
    } else {
        Ok(())
    }
}

/// Extract a string argument at `index` from `args`.
///
/// Returns the borrowed `&str` on success, or an error string naming
/// `fn_name` and the position.
pub fn extract_string_arg<'a>(
    args: &'a [ValueWord],
    index: usize,
    fn_name: &str,
) -> Result<&'a str, String> {
    args.get(index)
        .and_then(|a| a.as_str())
        .ok_or_else(|| {
            format!(
                "{}() requires a string argument at position {}",
                fn_name, index
            )
        })
}

/// Extract a numeric (i64) argument at `index` from `args`.
///
/// Returns the `i64` value on success, or an error string naming
/// `fn_name` and the position.
pub fn extract_number_arg(
    args: &[ValueWord],
    index: usize,
    fn_name: &str,
) -> Result<i64, String> {
    args.get(index)
        .and_then(|a| a.as_i64())
        .ok_or_else(|| {
            format!(
                "{}() requires a numeric argument at position {}",
                fn_name, index
            )
        })
}

/// Extract an f64 argument at `index` from `args`.
///
/// Returns the `f64` value on success, or an error string naming
/// `fn_name` and the position. Accepts both f64 and i64 values (the
/// latter is widened to f64).
pub fn extract_float_arg(
    args: &[ValueWord],
    index: usize,
    fn_name: &str,
) -> Result<f64, String> {
    args.get(index)
        .and_then(|a| a.as_f64().or_else(|| a.as_i64().map(|i| i as f64)))
        .ok_or_else(|| {
            format!(
                "{}() requires a numeric argument at position {}",
                fn_name, index
            )
        })
}

/// Extract a bool argument at `index` from `args`.
///
/// Returns the `bool` value on success, or an error string naming
/// `fn_name` and the position.
pub fn extract_bool_arg(
    args: &[ValueWord],
    index: usize,
    fn_name: &str,
) -> Result<bool, String> {
    args.get(index)
        .and_then(|a| a.as_bool())
        .ok_or_else(|| {
            format!(
                "{}() requires a bool argument at position {}",
                fn_name, index
            )
        })
}

// ─── String-error context extension ─────────────────────────────────

/// Extension trait that adds `.with_context()` to `Result<T, String>`.
///
/// Many stdlib module functions return `Result<T, String>`. This trait
/// lets callers wrap a bare string error with function-name context:
///
/// ```ignore
/// serde_json::from_str(data)
///     .map_err(|e| e.to_string())
///     .with_context("json.parse")?;
/// // error becomes: "json.parse(): <original message>"
/// ```
pub trait StringResultExt<T> {
    /// Wrap the error string with `"context(): original_error"` on failure.
    fn with_context(self, context: &str) -> Result<T, String>;
}

impl<T> StringResultExt<T> for Result<T, String> {
    #[inline]
    fn with_context(self, context: &str) -> Result<T, String> {
        self.map_err(|e| format!("{}(): {}", context, e))
    }
}

/// Format a contextualized error string.
///
/// Convenience function for call sites that have a non-`String` error and
/// want to produce `Err(String)` with function-name context in one step:
///
/// ```ignore
/// serde_json::from_str(data)
///     .map_err(|e| contextualize("json.parse", &e))?;
/// ```
#[inline]
pub fn contextualize(context: &str, err: &dyn std::fmt::Display) -> String {
    format!("{}(): {}", context, err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn check_arg_count_exact() {
        let args = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
        assert!(check_arg_count(&args, 2, "test_fn").is_ok());
    }

    #[test]
    fn check_arg_count_mismatch() {
        let args = vec![ValueWord::from_i64(1)];
        let err = check_arg_count(&args, 2, "test_fn").unwrap_err();
        assert!(err.contains("test_fn()"));
        assert!(err.contains("expected 2 arguments"));
        assert!(err.contains("got 1"));
    }

    #[test]
    fn check_arg_count_singular() {
        let args = vec![];
        let err = check_arg_count(&args, 1, "foo").unwrap_err();
        assert!(err.contains("expected 1 argument,"));
    }

    #[test]
    fn extract_string_arg_success() {
        let args = vec![ValueWord::from_string(Arc::new("hello".to_string()))];
        assert_eq!(extract_string_arg(&args, 0, "fn").unwrap(), "hello");
    }

    #[test]
    fn extract_string_arg_wrong_type() {
        let args = vec![ValueWord::from_i64(42)];
        let err = extract_string_arg(&args, 0, "fn").unwrap_err();
        assert!(err.contains("string argument at position 0"));
    }

    #[test]
    fn extract_string_arg_out_of_bounds() {
        let args: Vec<ValueWord> = vec![];
        assert!(extract_string_arg(&args, 0, "fn").is_err());
    }

    #[test]
    fn extract_number_arg_success() {
        let args = vec![ValueWord::from_i64(99)];
        assert_eq!(extract_number_arg(&args, 0, "fn").unwrap(), 99);
    }

    #[test]
    fn extract_number_arg_wrong_type() {
        let args = vec![ValueWord::from_string(Arc::new("nope".to_string()))];
        let err = extract_number_arg(&args, 0, "fn").unwrap_err();
        assert!(err.contains("numeric argument at position 0"));
    }

    #[test]
    fn extract_number_arg_out_of_bounds() {
        let args: Vec<ValueWord> = vec![];
        assert!(extract_number_arg(&args, 0, "fn").is_err());
    }

    #[test]
    fn extract_float_arg_from_f64() {
        let args = vec![ValueWord::from_f64(3.14)];
        let val = extract_float_arg(&args, 0, "test").unwrap();
        assert!((val - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_float_arg_from_i64() {
        let args = vec![ValueWord::from_i64(42)];
        let val = extract_float_arg(&args, 0, "test").unwrap();
        assert!((val - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_float_arg_wrong_type() {
        let args = vec![ValueWord::from_string(Arc::new("nope".to_string()))];
        let err = extract_float_arg(&args, 0, "fn").unwrap_err();
        assert!(err.contains("numeric argument at position 0"));
    }

    #[test]
    fn extract_bool_arg_success() {
        let args = vec![ValueWord::from_bool(true)];
        assert!(extract_bool_arg(&args, 0, "fn").unwrap());
    }

    #[test]
    fn extract_bool_arg_wrong_type() {
        let args = vec![ValueWord::from_i64(1)];
        let err = extract_bool_arg(&args, 0, "fn").unwrap_err();
        assert!(err.contains("bool argument at position 0"));
    }

    #[test]
    fn string_result_with_context() {
        let result: Result<i32, String> = Err("file not found".to_string());
        let err = result.with_context("file.read").unwrap_err();
        assert_eq!(err, "file.read(): file not found");
    }

    #[test]
    fn string_result_with_context_ok() {
        let result: Result<i32, String> = Ok(42);
        assert_eq!(result.with_context("file.read").unwrap(), 42);
    }

    #[test]
    fn contextualize_formats_correctly() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let msg = contextualize("file.read", &io_err);
        assert!(msg.starts_with("file.read(): "));
        assert!(msg.contains("gone"));
    }
}
