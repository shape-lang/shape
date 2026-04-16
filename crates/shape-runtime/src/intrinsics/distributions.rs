//! Statistical distribution intrinsics
//!
//! Sampling from common probability distributions.
//! Uses the thread-local RNG from the random module.

use super::random;
use super::{extract_f64, extract_str, extract_usize};
use crate::context::ExecutionContext;
use rand::Rng;
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};

/// Intrinsic: Sample from uniform distribution [lo, hi)
pub fn intrinsic_dist_uniform(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_uniform requires 2 arguments (lo, hi)".to_string(),
            location: None,
        });
    }

    let lo = extract_f64(&args[0], "__intrinsic_dist_uniform: lo")?;
    let hi = extract_f64(&args[1], "__intrinsic_dist_uniform: hi")?;

    if lo >= hi {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "__intrinsic_dist_uniform: lo ({}) must be < hi ({})",
                lo, hi
            ),
            location: None,
        });
    }

    let value = random::with_rng(|rng| {
        let u: f64 = rng.r#gen();
        lo + (hi - lo) * u
    });

    Ok(ValueWord::from_f64(value))
}

/// Intrinsic: Sample from lognormal distribution
pub fn intrinsic_dist_lognormal(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_lognormal requires 2 arguments (mean, std)".to_string(),
            location: None,
        });
    }

    let mean = extract_f64(&args[0], "__intrinsic_dist_lognormal: mean")?;
    let std = extract_f64(&args[1], "__intrinsic_dist_lognormal: std")?;

    if std < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_lognormal: std must be non-negative".to_string(),
            location: None,
        });
    }

    let normal_sample = random::with_rng(|rng| {
        let u1: f64 = rng.r#gen();
        let u2: f64 = rng.r#gen();
        let z = (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos();
        mean + std * z
    });

    Ok(ValueWord::from_f64(normal_sample.exp()))
}

/// Intrinsic: Sample from exponential distribution
pub fn intrinsic_dist_exponential(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_exponential requires 1 argument (lambda)".to_string(),
            location: None,
        });
    }

    let lambda = extract_f64(&args[0], "__intrinsic_dist_exponential: lambda")?;

    if lambda <= 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_exponential: lambda must be positive".to_string(),
            location: None,
        });
    }

    let value = random::with_rng(|rng| {
        let u: f64 = rng.r#gen();
        -u.ln() / lambda
    });

    Ok(ValueWord::from_f64(value))
}

/// Intrinsic: Sample from Poisson distribution
pub fn intrinsic_dist_poisson(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_poisson requires 1 argument (lambda)".to_string(),
            location: None,
        });
    }

    let lambda = extract_f64(&args[0], "__intrinsic_dist_poisson: lambda")?;

    if lambda < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_poisson: lambda must be non-negative".to_string(),
            location: None,
        });
    }

    let value = if lambda < 30.0 {
        random::with_rng(|rng| {
            let l = (-lambda).exp();
            let mut k = 0;
            let mut p = 1.0;
            loop {
                k += 1;
                let u: f64 = rng.r#gen();
                p *= u;
                if p <= l {
                    break;
                }
            }
            (k - 1) as f64
        })
    } else {
        random::with_rng(|rng| {
            let u1: f64 = rng.r#gen();
            let u2: f64 = rng.r#gen();
            let z = (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos();
            let value = lambda + lambda.sqrt() * z;
            value.max(0.0).round()
        })
    };

    Ok(ValueWord::from_f64(value))
}

/// Intrinsic: Sample n values from a named distribution
pub fn intrinsic_dist_sample_n(
    args: &[ValueWord],
    ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 3 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_dist_sample_n requires 3 arguments (dist_name, params, n)"
                .to_string(),
            location: None,
        });
    }

    let dist_name = extract_str(&args[0], "__intrinsic_dist_sample_n: dist_name")?;
    let params = args[1]
        .as_any_array()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "__intrinsic_dist_sample_n: params must be an array".to_string(),
            location: None,
        })?
        .to_generic();
    let n = extract_usize(&args[2], "__intrinsic_dist_sample_n: n")?;

    let mut samples = Vec::with_capacity(n);

    for _ in 0..n {
        // params is already a &[ValueWord] (via Arc<Vec<ValueWord>>)
        let sample = match dist_name {
            "uniform" => intrinsic_dist_uniform(&params, ctx)?,
            "lognormal" => intrinsic_dist_lognormal(&params, ctx)?,
            "exponential" => intrinsic_dist_exponential(&params, ctx)?,
            "poisson" => intrinsic_dist_poisson(&params, ctx)?,
            _ => {
                return Err(ShapeError::RuntimeError {
                    message: format!("Unknown distribution: {}", dist_name),
                    location: None,
                });
            }
        };
        samples.push(sample);
    }

    Ok(ValueWord::from_array(std::sync::Arc::new(samples)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intrinsics::random as random_intrinsics;

    fn mean_variance(samples: &[f64]) -> (f64, f64) {
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        (mean, var)
    }

    #[test]
    fn test_uniform_mean_variance() {
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(42.0)], &mut ctx);

        let mut samples = Vec::with_capacity(20000);
        for _ in 0..20000 {
            let v = intrinsic_dist_uniform(
                &[ValueWord::from_f64(0.0), ValueWord::from_f64(2.0)],
                &mut ctx,
            )
            .unwrap();
            if let Some(n) = v.as_number_coerce() {
                samples.push(n);
            }
        }

        let (mean, var) = mean_variance(&samples);
        assert!((mean - 1.0).abs() < 0.05);
        assert!((var - 1.0 / 3.0).abs() < 0.05);
    }

    #[test]
    fn test_exponential_mean_variance() {
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(7.0)], &mut ctx);

        let lambda = 2.0;
        let mut samples = Vec::with_capacity(20000);
        for _ in 0..20000 {
            let v = intrinsic_dist_exponential(&[ValueWord::from_f64(lambda)], &mut ctx).unwrap();
            if let Some(n) = v.as_number_coerce() {
                samples.push(n);
            }
        }
        let (mean, var) = mean_variance(&samples);
        assert!((mean - 1.0 / lambda).abs() < 0.05);
        assert!((var - 1.0 / (lambda * lambda)).abs() < 0.1);
    }

    #[test]
    fn test_poisson_mean_variance() {
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(123.0)], &mut ctx);

        let lambda = 12.0;
        let mut samples = Vec::with_capacity(20000);
        for _ in 0..20000 {
            let v = intrinsic_dist_poisson(&[ValueWord::from_f64(lambda)], &mut ctx).unwrap();
            if let Some(n) = v.as_number_coerce() {
                samples.push(n);
            }
        }
        let (mean, var) = mean_variance(&samples);
        assert!((mean - lambda).abs() < 0.3);
        assert!((var - lambda).abs() < 0.5);
    }

    #[test]
    fn test_sample_n_length() {
        use std::sync::Arc;
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(5.0)], &mut ctx);

        let result = intrinsic_dist_sample_n(
            &[
                ValueWord::from_string(Arc::new("uniform".to_string())),
                ValueWord::from_array(Arc::new(vec![
                    ValueWord::from_f64(0.0),
                    ValueWord::from_f64(1.0),
                ])),
                ValueWord::from_f64(10.0),
            ],
            &mut ctx,
        )
        .unwrap();

        let arr = result.as_any_array().expect("Expected array");
        assert_eq!(arr.len(), 10);
    }
}
