//! Error handling macros for reducing boilerplate in function implementations
//!
//! These macros provide consistent error messages and reduce code duplication
//! across the runtime evaluation functions.

/// Validates that exactly N arguments were passed to a function.
///
/// # Example
/// ```ignore
/// require_args!(args, 2, "add");
/// // Equivalent to:
/// // if args.len() != 2 {
/// //     return Err(ShapeError::RuntimeError {
/// //         message: "add() requires exactly 2 argument(s)".to_string(),
/// //         location: None,
/// //     });
/// // }
/// ```
#[macro_export]
macro_rules! require_args {
    ($args:expr, $count:expr, $func:expr) => {
        if $args.len() != $count {
            return Err($crate::error::ShapeError::RuntimeError {
                message: format!(
                    "{}() requires exactly {} argument{}",
                    $func,
                    $count,
                    if $count == 1 { "" } else { "s" }
                ),
                location: None,
            });
        }
    };
}

/// Validates that at least N arguments were passed to a function.
///
/// # Example
/// ```ignore
/// require_min_args!(args, 1, "sum");
/// // Equivalent to:
/// // if args.len() < 1 {
/// //     return Err(ShapeError::RuntimeError {
/// //         message: "sum() requires at least 1 argument(s)".to_string(),
/// //         location: None,
/// //     });
/// // }
/// ```
#[macro_export]
macro_rules! require_min_args {
    ($args:expr, $min:expr, $func:expr) => {
        if $args.len() < $min {
            return Err($crate::error::ShapeError::RuntimeError {
                message: format!(
                    "{}() requires at least {} argument{}",
                    $func,
                    $min,
                    if $min == 1 { "" } else { "s" }
                ),
                location: None,
            });
        }
    };
}

/// Validates that arguments count is within a range.
///
/// # Example
/// ```ignore
/// require_args_range!(args, 1, 3, "range");
/// ```
#[macro_export]
macro_rules! require_args_range {
    ($args:expr, $min:expr, $max:expr, $func:expr) => {
        if $args.len() < $min || $args.len() > $max {
            return Err($crate::error::ShapeError::RuntimeError {
                message: format!(
                    "{}() requires between {} and {} arguments, got {}",
                    $func,
                    $min,
                    $max,
                    $args.len()
                ),
                location: None,
            });
        }
    };
}
