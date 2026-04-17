//! Scan intrinsics - Generic prefix operations
//!
//! Provides prefix scan operations beyond cumsum/cumprod:
//! - Running max/min (useful for drawdown, high-water marks)
//! - Logical OR/AND (useful for alert accumulation)
//! - Custom binary operations

use super::{extract_f64_array, extract_str, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};

/// Intrinsic: Generic Prefix Scan
pub fn intrinsic_scan(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_scan requires 2-3 arguments (series, operation, [initial])"
                .to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Input array")?;
    let op = extract_str(&args[1], "Operation")?;

    let initial = if args.len() == 3 {
        if let Some(n) = args[2].as_number_coerce() {
            Some(n)
        } else if let Some(b) = args[2].as_bool() {
            Some(if b { 1.0 } else { 0.0 })
        } else if args[2].is_none() {
            None
        } else {
            return Err(ShapeError::RuntimeError {
                message: "Initial value must be a number or boolean".to_string(),
                location: None,
            });
        }
    } else {
        None
    };

    let result = match op {
        "sum" | "add" => scan_sum(&data, initial),
        "prod" | "mul" | "product" => scan_prod(&data, initial),
        "max" | "maximum" => scan_max(&data, initial),
        "min" | "minimum" => scan_min(&data, initial),
        "or" | "any" => scan_or(&data, initial),
        "and" | "all" => scan_and(&data, initial),
        _ => {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Unknown scan operation: {}. Use 'sum', 'prod', 'max', 'min', 'or', 'and'",
                    op
                ),
                location: None,
            });
        }
    };

    Ok(f64_vec_to_nb_array(result))
}

fn scan_sum(data: &[f64], initial: Option<f64>) -> Vec<f64> {
    if data.is_empty() {
        return vec![];
    }
    let mut result = Vec::with_capacity(data.len());
    let mut acc = initial.unwrap_or(0.0);
    for &val in data {
        acc += val;
        result.push(acc);
    }
    result
}

fn scan_prod(data: &[f64], initial: Option<f64>) -> Vec<f64> {
    if data.is_empty() {
        return vec![];
    }
    let mut result = Vec::with_capacity(data.len());
    let mut acc = initial.unwrap_or(1.0);
    for &val in data {
        acc *= val;
        result.push(acc);
    }
    result
}

fn scan_max(data: &[f64], initial: Option<f64>) -> Vec<f64> {
    if data.is_empty() {
        return vec![];
    }
    let mut result = Vec::with_capacity(data.len());
    let mut acc = initial.unwrap_or(f64::NEG_INFINITY);
    for &val in data {
        acc = acc.max(val);
        result.push(acc);
    }
    result
}

fn scan_min(data: &[f64], initial: Option<f64>) -> Vec<f64> {
    if data.is_empty() {
        return vec![];
    }
    let mut result = Vec::with_capacity(data.len());
    let mut acc = initial.unwrap_or(f64::INFINITY);
    for &val in data {
        acc = acc.min(val);
        result.push(acc);
    }
    result
}

fn scan_or(data: &[f64], initial: Option<f64>) -> Vec<f64> {
    if data.is_empty() {
        return vec![];
    }
    let mut result = Vec::with_capacity(data.len());
    let mut acc = initial.map(|v| v != 0.0).unwrap_or(false);
    for &val in data {
        acc = acc || (val != 0.0);
        result.push(if acc { 1.0 } else { 0.0 });
    }
    result
}

fn scan_and(data: &[f64], initial: Option<f64>) -> Vec<f64> {
    if data.is_empty() {
        return vec![];
    }
    let mut result = Vec::with_capacity(data.len());
    let mut acc = initial.map(|v| v != 0.0).unwrap_or(true);
    for &val in data {
        acc = acc && (val != 0.0);
        result.push(if acc { 1.0 } else { 0.0 });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_scan_sum() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = scan_sum(&data, None);
        assert_eq!(result, vec![1.0, 3.0, 6.0, 10.0, 15.0]);
    }

    #[test]
    fn test_scan_sum_with_initial() {
        let data = vec![1.0, 2.0, 3.0];
        let result = scan_sum(&data, Some(10.0));
        assert_eq!(result, vec![11.0, 13.0, 16.0]);
    }

    #[test]
    fn test_scan_max() {
        let data = vec![3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0];
        let result = scan_max(&data, None);
        assert_eq!(result, vec![3.0, 3.0, 4.0, 4.0, 5.0, 9.0, 9.0]);
    }

    #[test]
    fn test_scan_min() {
        let data = vec![5.0, 3.0, 7.0, 2.0, 8.0];
        let result = scan_min(&data, None);
        assert_eq!(result, vec![5.0, 3.0, 3.0, 2.0, 2.0]);
    }

    #[test]
    fn test_scan_or() {
        let data = vec![0.0, 0.0, 1.0, 0.0, 0.0];
        let result = scan_or(&data, None);
        assert_eq!(result, vec![0.0, 0.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_scan_and() {
        let data = vec![1.0, 1.0, 0.0, 1.0, 1.0];
        let result = scan_and(&data, None);
        assert_eq!(result, vec![1.0, 1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_intrinsic_scan_max() {
        let mut ctx = ExecutionContext::new_empty();

        let series = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_f64(3.0),
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(4.0),
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(5.0),
        ]));

        let result = intrinsic_scan(
            &[series, ValueWord::from_string(Arc::new("max".to_string()))],
            &mut ctx,
        )
        .unwrap();

        let arr = result.as_any_array().expect("Expected array").to_generic();
        let data: Vec<f64> = arr.iter().map(|v| v.as_number_coerce().unwrap()).collect();
        assert_eq!(data, &[3.0, 3.0, 4.0, 4.0, 5.0]);
    }

    #[test]
    fn test_intrinsic_scan_or_alerts() {
        let mut ctx = ExecutionContext::new_empty();

        let alerts = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_f64(0.0),
            ValueWord::from_f64(0.0),
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(0.0),
            ValueWord::from_f64(0.0),
        ]));

        let result = intrinsic_scan(
            &[
                alerts,
                ValueWord::from_string(Arc::new("or".to_string())),
                ValueWord::from_bool(false),
            ],
            &mut ctx,
        )
        .unwrap();

        let arr = result.as_any_array().expect("Expected array").to_generic();
        let data: Vec<f64> = arr.iter().map(|v| v.as_number_coerce().unwrap()).collect();
        assert_eq!(data, &[0.0, 0.0, 1.0, 1.0, 1.0]);
    }
}
