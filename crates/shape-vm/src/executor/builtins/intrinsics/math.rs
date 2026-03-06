//! Math intrinsics — sum, mean, min, max, variance, std

use shape_value::{VMError, ValueWord};
use std::sync::Arc;

use super::{NbIntrinsicResult, nb_extract_f64_data};

/// Sum of all values in a series or array
pub fn vm_intrinsic_sum(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "sum() requires 1 argument".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(0.0));
    }

    let sum: f64 = data.iter().sum();
    Ok(ValueWord::from_f64(sum))
}

/// Mean (average) of all values
pub fn vm_intrinsic_mean(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "mean() requires 1 argument".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let sum: f64 = data.iter().sum();
    let mean = sum / data.len() as f64;
    Ok(ValueWord::from_f64(mean))
}

/// Minimum value
pub fn vm_intrinsic_min(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "min() requires at least 1 argument".to_string(),
        ));
    }

    // Handle multi-argument min(a, b, c, ...)
    if args.len() >= 2 {
        let mut all_numbers = true;
        let mut min_val = f64::INFINITY;

        for arg in args {
            if let Some(n) = arg.as_number_coerce() {
                min_val = min_val.min(n);
            } else {
                all_numbers = false;
                break;
            }
        }

        if all_numbers {
            return Ok(ValueWord::from_f64(min_val));
        }
    }

    // Single argument: series or array
    let data = nb_extract_f64_data(&args[0])?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    Ok(ValueWord::from_f64(min))
}

/// Maximum value
pub fn vm_intrinsic_max(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "max() requires at least 1 argument".to_string(),
        ));
    }

    // Handle multi-argument max(a, b, c, ...)
    if args.len() >= 2 {
        let mut all_numbers = true;
        let mut max_val = f64::NEG_INFINITY;

        for arg in args {
            if let Some(n) = arg.as_number_coerce() {
                max_val = max_val.max(n);
            } else {
                all_numbers = false;
                break;
            }
        }

        if all_numbers {
            return Ok(ValueWord::from_f64(max_val));
        }
    }

    // Single argument: series or array
    let data = nb_extract_f64_data(&args[0])?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok(ValueWord::from_f64(max))
}

/// Variance (population variance)
pub fn vm_intrinsic_variance(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "variance() requires 1 argument".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let n = data.len() as f64;
    let mean: f64 = data.iter().sum::<f64>() / n;
    let variance: f64 = data.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / n;

    Ok(ValueWord::from_f64(variance))
}

/// Standard deviation
pub fn vm_intrinsic_std(args: &[ValueWord]) -> NbIntrinsicResult {
    let variance_nb = vm_intrinsic_variance(args)?;
    let var = variance_nb.as_number_coerce().unwrap_or(f64::NAN);
    Ok(ValueWord::from_f64(var.sqrt()))
}

// ===== Character Code Intrinsics =====

/// Get the Unicode code point of the first character in a string
pub fn vm_intrinsic_char_code(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "__intrinsic_char_code requires 1 argument".to_string(),
        ));
    }
    let s = args[0]
        .as_str()
        .ok_or_else(|| VMError::RuntimeError("char_code: argument must be a string".to_string()))?;
    let ch = s
        .chars()
        .next()
        .ok_or_else(|| VMError::RuntimeError("char_code: empty string".to_string()))?;
    Ok(ValueWord::from_f64(ch as u32 as f64))
}

/// Create a single-character string from a Unicode code point
pub fn vm_intrinsic_from_char_code(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "__intrinsic_from_char_code requires 1 argument".to_string(),
        ));
    }
    let code = args[0].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("from_char_code: argument must be a number".to_string())
    })?;
    let ch = char::from_u32(code as u32).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "from_char_code: invalid code point {}",
            code as u32
        ))
    })?;
    Ok(ValueWord::from_string(Arc::new(ch.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
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
