//! Vector intrinsics - Element-wise vector operations
//!
//! These functions provide element-wise operations on arrays and series,
//! critical for implementing custom indicators and math in Shape.
//! Optimized using SIMD via the `wide` crate.

use super::{extract_f64_array, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;
use wide::f64x4;

// Threshold for SIMD: arrays smaller than this use scalar fallback
const SIMD_THRESHOLD: usize = 16;

/// Core Vector Absolute Value: |x|
pub fn intrinsic_vec_abs(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "vec_abs requires 1 argument".into(),
            location: None,
        });
    }
    let data = extract_f64_array(&args[0], "Argument")?;
    let mut result = vec![0.0; data.len()];

    if data.len() >= SIMD_THRESHOLD {
        let chunks = data.len() / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f64x4::from(&data[idx..idx + 4]);
            let res = v.abs();
            result[idx..idx + 4].copy_from_slice(&res.to_array());
        }
        for i in (chunks * 4)..data.len() {
            result[i] = data[i].abs();
        }
    } else {
        for i in 0..data.len() {
            result[i] = data[i].abs();
        }
    }

    Ok(f64_vec_to_nb_array(result))
}

/// Core Vector Square Root: sqrt(x)
pub fn intrinsic_vec_sqrt(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "vec_sqrt requires 1 argument".into(),
            location: None,
        });
    }
    let data = extract_f64_array(&args[0], "Argument")?;
    let mut result = vec![0.0; data.len()];

    if data.len() >= SIMD_THRESHOLD {
        let chunks = data.len() / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f64x4::from(&data[idx..idx + 4]);
            let res = v.sqrt();
            result[idx..idx + 4].copy_from_slice(&res.to_array());
        }
        for i in (chunks * 4)..data.len() {
            result[i] = data[i].sqrt();
        }
    } else {
        for i in 0..data.len() {
            result[i] = data[i].sqrt();
        }
    }

    Ok(f64_vec_to_nb_array(result))
}

/// Core Vector Logarithm: ln(x)
pub fn intrinsic_vec_ln(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "vec_ln requires 1 argument".into(),
            location: None,
        });
    }
    let data = extract_f64_array(&args[0], "Argument")?;
    let result: Vec<f64> = data.iter().map(|x| x.ln()).collect();
    Ok(f64_vec_to_nb_array(result))
}

/// Core Vector Exponent: exp(x)
pub fn intrinsic_vec_exp(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "vec_exp requires 1 argument".into(),
            location: None,
        });
    }
    let data = extract_f64_array(&args[0], "Argument")?;
    let result: Vec<f64> = data.iter().map(|x| x.exp()).collect();
    Ok(f64_vec_to_nb_array(result))
}

/// Helper for binary vector ops that also accept a scalar right-hand side
fn binary_vec_op(
    args: &[ValueWord],
    name: &str,
    simd_op: fn(f64x4, f64x4) -> f64x4,
    scalar_op: fn(f64, f64) -> f64,
) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: format!("{} requires 2 arguments", name),
            location: None,
        });
    }

    // Check if right is a scalar number
    if let Some(scalar) = args[1].as_number_coerce() {
        let a = extract_f64_array(&args[0], "Left argument")?;
        let mut result = vec![0.0; a.len()];
        if a.len() >= SIMD_THRESHOLD {
            let s_vec = f64x4::splat(scalar);
            let chunks = a.len() / 4;
            for i in 0..chunks {
                let idx = i * 4;
                let v = f64x4::from(&a[idx..idx + 4]);
                let res = simd_op(v, s_vec);
                result[idx..idx + 4].copy_from_slice(&res.to_array());
            }
            for i in (chunks * 4)..a.len() {
                result[i] = scalar_op(a[i], scalar);
            }
        } else {
            for i in 0..a.len() {
                result[i] = scalar_op(a[i], scalar);
            }
        }
        // If it was a scalar, it might not have an array — skip length check
        if args[0].as_any_array().is_some() {
            return Ok(f64_vec_to_nb_array(result));
        }
    }

    let a = extract_f64_array(&args[0], "Left argument")?;
    let b = extract_f64_array(&args[1], "Right argument")?;

    if a.len() != b.len() {
        return Err(ShapeError::RuntimeError {
            message: format!("Vector length mismatch: {} vs {}", a.len(), b.len()),
            location: None,
        });
    }

    let mut result = vec![0.0; a.len()];
    if a.len() >= SIMD_THRESHOLD {
        let chunks = a.len() / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v1 = f64x4::from(&a[idx..idx + 4]);
            let v2 = f64x4::from(&b[idx..idx + 4]);
            let res = simd_op(v1, v2);
            result[idx..idx + 4].copy_from_slice(&res.to_array());
        }
        for i in (chunks * 4)..a.len() {
            result[i] = scalar_op(a[i], b[i]);
        }
    } else {
        for i in 0..a.len() {
            result[i] = scalar_op(a[i], b[i]);
        }
    }
    Ok(f64_vec_to_nb_array(result))
}

/// Core Vector Addition: a + b
pub fn intrinsic_vec_add(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    binary_vec_op(args, "vec_add", |a, b| a + b, |a, b| a + b)
}

/// Core Vector Subtraction: a - b
pub fn intrinsic_vec_sub(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    binary_vec_op(args, "vec_sub", |a, b| a - b, |a, b| a - b)
}

/// Core Vector Multiplication: a * b
pub fn intrinsic_vec_mul(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    binary_vec_op(args, "vec_mul", |a, b| a * b, |a, b| a * b)
}

/// Core Vector Division: a / b
pub fn intrinsic_vec_div(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    binary_vec_op(args, "vec_div", |a, b| a / b, |a, b| a / b)
}

/// Core Vector Max: max(a, b)
pub fn intrinsic_vec_max(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    binary_vec_op(args, "vec_max", |a, b| a.max(b), f64::max)
}

/// Core Vector Min: min(a, b)
pub fn intrinsic_vec_min(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    binary_vec_op(args, "vec_min", |a, b| a.min(b), f64::min)
}

/// Core Vector Select: condition ? true_val : false_val
pub fn intrinsic_vec_select(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 3 {
        return Err(ShapeError::RuntimeError {
            message: "vec_select requires 3 arguments".into(),
            location: None,
        });
    }
    let cond = extract_f64_array(&args[0], "Condition")?;

    let t_vec = if let Some(n) = args[1].as_number_coerce() {
        vec![n; cond.len()]
    } else {
        extract_f64_array(&args[1], "True value")?
    };

    let f_vec = if let Some(n) = args[2].as_number_coerce() {
        vec![n; cond.len()]
    } else {
        extract_f64_array(&args[2], "False value")?
    };

    if t_vec.len() != cond.len() || f_vec.len() != cond.len() {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Vector length mismatch in select: cond={}, true={}, false={}",
                cond.len(),
                t_vec.len(),
                f_vec.len()
            ),
            location: None,
        });
    }

    let mut result = vec![0.0; cond.len()];
    for i in 0..cond.len() {
        result[i] = if cond[i] != 0.0 { t_vec[i] } else { f_vec[i] };
    }

    Ok(f64_vec_to_nb_array(result))
}

// ===== Typed Vec<T> SIMD Kernels =====
//
// These operate directly on raw slices from IntArray/FloatArray HeapValue variants,
// avoiding ValueWord materialization. Used by the executor's arithmetic dispatch.

use shape_value::aligned_vec::AlignedVec;

/// Element-wise addition of two f64 slices using SIMD (f64x4).
/// Returns an AlignedVec<f64>. Panics if lengths differ.
pub fn simd_vec_add_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va + vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] + b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] + b[i]);
        }
    }
    result
}

/// Element-wise subtraction of two f64 slices using SIMD.
pub fn simd_vec_sub_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va - vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] - b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] - b[i]);
        }
    }
    result
}

/// Element-wise multiplication of two f64 slices using SIMD.
pub fn simd_vec_mul_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va * vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] * b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] * b[i]);
        }
    }
    result
}

/// Element-wise division of two f64 slices using SIMD.
pub fn simd_vec_div_f64(a: &[f64], b: &[f64]) -> AlignedVec<f64> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            let res = va / vb;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] / b[i]);
        }
    } else {
        for i in 0..len {
            result.push(a[i] / b[i]);
        }
    }
    result
}

/// Scalar broadcast: multiply each element by a scalar using SIMD.
pub fn simd_vec_scale_f64(a: &[f64], scalar: f64) -> AlignedVec<f64> {
    let len = a.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let s_vec = f64x4::splat(scalar);
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let res = va * s_vec;
            for &v in res.to_array().iter() {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a[i] * scalar);
        }
    } else {
        for i in 0..len {
            result.push(a[i] * scalar);
        }
    }
    result
}

/// Element-wise addition of two i64 slices with checked overflow.
/// Returns Err if any element pair overflows.
pub fn simd_vec_add_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        match a[i].checked_add(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Element-wise subtraction of two i64 slices with checked overflow.
pub fn simd_vec_sub_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        match a[i].checked_sub(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Element-wise multiplication of two i64 slices with checked overflow.
pub fn simd_vec_mul_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        match a[i].checked_mul(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Element-wise division of two i64 slices with checked overflow.
pub fn simd_vec_div_i64(a: &[i64], b: &[i64]) -> std::result::Result<Vec<i64>, ()> {
    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        if b[i] == 0 {
            return Err(());
        }
        match a[i].checked_div(b[i]) {
            Some(v) => result.push(v),
            None => return Err(()),
        }
    }
    Ok(result)
}

/// Coerce an i64 slice to f64 for mixed-type arithmetic.
pub fn i64_slice_to_f64(data: &[i64]) -> AlignedVec<f64> {
    let mut result = AlignedVec::with_capacity(data.len());
    for &v in data {
        result.push(v as f64);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionContext;
    use shape_value::ValueWord;

    fn make_array(vals: &[f64]) -> ValueWord {
        f64_vec_to_nb_array(vals.to_vec())
    }

    fn unwrap_array(nb: ValueWord) -> Vec<f64> {
        let arr = nb.as_any_array().expect("Expected array").to_generic();
        arr.iter()
            .map(|v| v.as_number_coerce().expect("Expected number"))
            .collect()
    }

    #[test]
    fn test_vec_abs() {
        let input = make_array(&[-1.0, 2.0, -3.5]);
        let mut ctx = ExecutionContext::new_empty();
        let result = intrinsic_vec_abs(&[input], &mut ctx).unwrap();
        let arr = unwrap_array(result);
        assert_eq!(arr, vec![1.0, 2.0, 3.5]);
    }

    #[test]
    fn test_vec_add_vector() {
        let a = make_array(&[1.0, 2.0]);
        let b = make_array(&[3.0, 4.0]);
        let mut ctx = ExecutionContext::new_empty();
        let result = intrinsic_vec_add(&[a, b], &mut ctx).unwrap();
        let arr = unwrap_array(result);
        assert_eq!(arr, vec![4.0, 6.0]);
    }

    #[test]
    fn test_vec_add_scalar() {
        let a = make_array(&[1.0, 2.0]);
        let b = ValueWord::from_f64(5.0);
        let mut ctx = ExecutionContext::new_empty();
        let result = intrinsic_vec_add(&[a, b], &mut ctx).unwrap();
        let arr = unwrap_array(result);
        assert_eq!(arr, vec![6.0, 7.0]);
    }

    #[test]
    fn test_vec_sqrt_simd() {
        // Large enough to trigger SIMD (threshold 16)
        let data: Vec<f64> = (0..20).map(|i| (i * i) as f64).collect();
        let input = f64_vec_to_nb_array(data);
        let mut ctx = ExecutionContext::new_empty();
        let result = intrinsic_vec_sqrt(&[input], &mut ctx).unwrap();
        let arr = unwrap_array(result);
        assert_eq!(arr.len(), 20);
        for i in 0..20 {
            assert_eq!(arr[i], i as f64);
        }
    }

    // ===== SIMD kernel tests =====

    #[test]
    fn test_simd_vec_add_f64_small() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        let result = super::simd_vec_add_f64(&a, &b);
        assert_eq!(&*result, &[5.0, 7.0, 9.0]);
    }

    #[test]
    fn test_simd_vec_add_f64_large() {
        let a: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..20).map(|i| (i * 2) as f64).collect();
        let result = super::simd_vec_add_f64(&a, &b);
        for i in 0..20 {
            assert_eq!(result[i], (i * 3) as f64);
        }
    }

    #[test]
    fn test_simd_vec_sub_f64() {
        let a = [10.0, 20.0, 30.0];
        let b = [3.0, 5.0, 7.0];
        let result = super::simd_vec_sub_f64(&a, &b);
        assert_eq!(&*result, &[7.0, 15.0, 23.0]);
    }

    #[test]
    fn test_simd_vec_mul_f64() {
        let a = [2.0, 3.0, 4.0];
        let b = [5.0, 6.0, 7.0];
        let result = super::simd_vec_mul_f64(&a, &b);
        assert_eq!(&*result, &[10.0, 18.0, 28.0]);
    }

    #[test]
    fn test_simd_vec_div_f64() {
        let a = [10.0, 20.0, 30.0];
        let b = [2.0, 5.0, 6.0];
        let result = super::simd_vec_div_f64(&a, &b);
        assert_eq!(&*result, &[5.0, 4.0, 5.0]);
    }

    #[test]
    fn test_simd_vec_scale_f64() {
        let a = [1.0, 2.0, 3.0];
        let result = super::simd_vec_scale_f64(&a, 10.0);
        assert_eq!(&*result, &[10.0, 20.0, 30.0]);
    }

    #[test]
    fn test_simd_vec_add_i64_ok() {
        let a = [1i64, 2, 3];
        let b = [4i64, 5, 6];
        let result = super::simd_vec_add_i64(&a, &b).unwrap();
        assert_eq!(result, vec![5, 7, 9]);
    }

    #[test]
    fn test_simd_vec_add_i64_overflow() {
        let a = [i64::MAX];
        let b = [1i64];
        assert!(super::simd_vec_add_i64(&a, &b).is_err());
    }

    #[test]
    fn test_simd_vec_div_i64_zero() {
        let a = [10i64];
        let b = [0i64];
        assert!(super::simd_vec_div_i64(&a, &b).is_err());
    }

    #[test]
    fn test_i64_slice_to_f64() {
        let data = [1i64, -2, 100];
        let result = super::i64_slice_to_f64(&data);
        assert_eq!(&*result, &[1.0, -2.0, 100.0]);
    }
}
