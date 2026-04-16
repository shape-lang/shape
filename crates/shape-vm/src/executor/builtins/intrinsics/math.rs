//! Math intrinsics — delegates to shape_runtime canonical implementations
//!
//! Each function is a thin wrapper that calls the runtime intrinsic with a
//! temporary ExecutionContext and converts ShapeError to VMError.

use shape_value::{VMError, ValueWord};

use super::NbIntrinsicResult;

/// Helper: call a runtime intrinsic that takes (&[ValueWord], &mut ExecutionContext)
/// and convert the error to VMError.
fn delegate(
    args: &[ValueWord],
    func: fn(
        &[ValueWord],
        &mut shape_runtime::context::ExecutionContext,
    ) -> shape_ast::error::Result<ValueWord>,
) -> NbIntrinsicResult {
    let mut ctx = shape_runtime::context::ExecutionContext::new_empty();
    func(args, &mut ctx).map_err(|e| VMError::RuntimeError(format!("{}", e)))
}

/// Sum of all values in a series or array
pub fn vm_intrinsic_sum(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_sum)
}

/// Mean (average) of all values
pub fn vm_intrinsic_mean(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_mean)
}

/// Minimum value
pub fn vm_intrinsic_min(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_min)
}

/// Maximum value
pub fn vm_intrinsic_max(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_max)
}

/// Variance (population variance)
pub fn vm_intrinsic_variance(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_variance)
}

/// Standard deviation
pub fn vm_intrinsic_std(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_std)
}

// ===== Trigonometric Intrinsics =====

/// Two-argument arc tangent
pub fn vm_intrinsic_atan2(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_atan2)
}

/// Hyperbolic sine
pub fn vm_intrinsic_sinh(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_sinh)
}

/// Hyperbolic cosine
pub fn vm_intrinsic_cosh(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_cosh)
}

/// Hyperbolic tangent
pub fn vm_intrinsic_tanh(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_tanh)
}

// ===== Character Code Intrinsics =====

/// Get the Unicode code point of the first character in a string
pub fn vm_intrinsic_char_code(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::math::intrinsic_char_code)
}

/// Create a single-character string from a Unicode code point
pub fn vm_intrinsic_from_char_code(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::math::intrinsic_from_char_code,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::ValueWordExt;
    use std::sync::Arc;

    fn make_nb_array(values: Vec<f64>) -> ValueWord {
        ValueWord::from_array(Arc::new(
            values.into_iter().map(ValueWord::from_f64).collect(),
        ))
    }

    fn assert_nb_number_eq(value: ValueWord, expected: f64) {
        let n = value.as_number_coerce().expect("Expected number");
        assert!(
            (n - expected).abs() < 0.0001,
            "Expected {}, got {}",
            expected,
            n
        );
    }

    #[test]
    fn test_sum_array() {
        let arr = make_nb_array(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let result = vm_intrinsic_sum(&[arr]).unwrap();
        assert_nb_number_eq(result, 15.0);
    }

    #[test]
    fn test_mean_array() {
        let arr = make_nb_array(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let result = vm_intrinsic_mean(&[arr]).unwrap();
        assert_nb_number_eq(result, 3.0);
    }

    #[test]
    fn test_min_max_array() {
        let arr = make_nb_array(vec![3.0, 1.0, 4.0, 1.0, 5.0]);

        let min_result = vm_intrinsic_min(&[arr.clone()]).unwrap();
        assert_nb_number_eq(min_result, 1.0);

        let max_result = vm_intrinsic_max(&[arr]).unwrap();
        assert_nb_number_eq(max_result, 5.0);
    }

    #[test]
    fn test_min_max_multiple_args() {
        let a = ValueWord::from_f64(3.0);
        let b = ValueWord::from_f64(1.0);
        let c = ValueWord::from_f64(4.0);

        let min_result = vm_intrinsic_min(&[a.clone(), b.clone(), c.clone()]).unwrap();
        assert_nb_number_eq(min_result, 1.0);

        let max_result = vm_intrinsic_max(&[a, b, c]).unwrap();
        assert_nb_number_eq(max_result, 4.0);
    }

    #[test]
    fn test_variance_std() {
        let arr = make_nb_array(vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);

        let var_result = vm_intrinsic_variance(&[arr.clone()]).unwrap();
        let var = var_result.as_number_coerce().expect("Expected number");
        assert!((var - 4.0).abs() < 0.001);

        let std_result = vm_intrinsic_std(&[arr]).unwrap();
        let std = std_result.as_number_coerce().expect("Expected number");
        assert!((std - 2.0).abs() < 0.001);
    }
}
