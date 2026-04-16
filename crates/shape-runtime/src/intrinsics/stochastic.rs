//! Stochastic process intrinsics
//!
//! Provides basic stochastic process generators (Brownian motion, GBM, OU, random walk).
//! Uses the shared thread-local RNG from random.rs.

use super::random;
use super::{extract_f64, extract_usize, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use rand::Rng;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;

fn sample_standard_normal() -> f64 {
    random::with_rng(|rng| {
        let u1: f64 = rng.r#gen();
        let u2: f64 = rng.r#gen();
        (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos()
    })
}

/// Intrinsic: Brownian motion path
pub fn intrinsic_brownian_motion(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 3 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_brownian_motion requires 3 arguments (n, dt, sigma)".to_string(),
            location: None,
        });
    }

    let n = extract_usize(&args[0], "n")?;
    let dt = extract_f64(&args[1], "dt")?;
    let sigma = extract_f64(&args[2], "sigma")?;

    if dt <= 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_brownian_motion: dt must be positive".to_string(),
            location: None,
        });
    }
    if sigma < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_brownian_motion: sigma must be non-negative".to_string(),
            location: None,
        });
    }

    let mut path = Vec::with_capacity(n);
    let mut x = 0.0;
    let scale = sigma * dt.sqrt();

    for i in 0..n {
        if i > 0 {
            x += scale * sample_standard_normal();
        }
        path.push(x);
    }

    Ok(f64_vec_to_nb_array(path))
}

/// Intrinsic: Geometric Brownian Motion path
pub fn intrinsic_gbm(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 5 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_gbm requires 5 arguments (n, dt, mu, sigma, s0)".to_string(),
            location: None,
        });
    }

    let n = extract_usize(&args[0], "n")?;
    let dt = extract_f64(&args[1], "dt")?;
    let mu = extract_f64(&args[2], "mu")?;
    let sigma = extract_f64(&args[3], "sigma")?;
    let s0 = extract_f64(&args[4], "s0")?;

    if dt <= 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_gbm: dt must be positive".to_string(),
            location: None,
        });
    }
    if sigma < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_gbm: sigma must be non-negative".to_string(),
            location: None,
        });
    }
    if s0 <= 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_gbm: s0 must be positive".to_string(),
            location: None,
        });
    }

    let mut path = Vec::with_capacity(n);
    let mut s = s0;
    let drift = (mu - 0.5 * sigma * sigma) * dt;
    let diffusion_scale = sigma * dt.sqrt();

    for i in 0..n {
        if i > 0 {
            let z = sample_standard_normal();
            s *= (drift + diffusion_scale * z).exp();
        }
        path.push(s);
    }

    Ok(f64_vec_to_nb_array(path))
}

/// Intrinsic: Ornstein-Uhlenbeck process
pub fn intrinsic_ou_process(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 6 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_ou_process requires 6 arguments (n, dt, theta, mu, sigma, x0)"
                .to_string(),
            location: None,
        });
    }

    let n = extract_usize(&args[0], "n")?;
    let dt = extract_f64(&args[1], "dt")?;
    let theta = extract_f64(&args[2], "theta")?;
    let mu = extract_f64(&args[3], "mu")?;
    let sigma = extract_f64(&args[4], "sigma")?;
    let x0 = extract_f64(&args[5], "x0")?;

    if dt <= 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_ou_process: dt must be positive".to_string(),
            location: None,
        });
    }
    if theta < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_ou_process: theta must be non-negative".to_string(),
            location: None,
        });
    }
    if sigma < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_ou_process: sigma must be non-negative".to_string(),
            location: None,
        });
    }

    let mut path = Vec::with_capacity(n);
    let mut x = x0;
    let diffusion_scale = sigma * dt.sqrt();

    for i in 0..n {
        if i > 0 {
            let z = sample_standard_normal();
            x += theta * (mu - x) * dt + diffusion_scale * z;
        }
        path.push(x);
    }

    Ok(f64_vec_to_nb_array(path))
}

/// Intrinsic: Random walk
pub fn intrinsic_random_walk(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_walk requires 2 arguments (n, step_size)".to_string(),
            location: None,
        });
    }

    let n = extract_usize(&args[0], "n")?;
    let step_size = extract_f64(&args[1], "step_size")?;

    if step_size <= 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_walk: step_size must be positive".to_string(),
            location: None,
        });
    }

    let mut path = Vec::with_capacity(n);
    let mut x = 0.0;

    for i in 0..n {
        if i > 0 {
            let step = random::with_rng(|rng| {
                if rng.r#gen::<f64>() < 0.5 {
                    -step_size
                } else {
                    step_size
                }
            });
            x += step;
        }
        path.push(x);
    }

    Ok(f64_vec_to_nb_array(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intrinsics::random as random_intrinsics;
    use shape_value::ValueWordExt;

    fn mean_variance(samples: &[f64]) -> (f64, f64) {
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        (mean, var)
    }

    #[test]
    fn test_brownian_motion_stats() {
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(999.0)], &mut ctx);

        let result = intrinsic_brownian_motion(
            &[
                ValueWord::from_f64(10001.0),
                ValueWord::from_f64(1.0),
                ValueWord::from_f64(1.0),
            ],
            &mut ctx,
        )
        .unwrap();

        let arr = result.as_any_array().expect("Expected array").to_generic();
        let path: Vec<f64> = arr
            .iter()
            .map(|v| v.as_number_coerce().unwrap_or(0.0))
            .collect();

        let mut increments = Vec::with_capacity(path.len() - 1);
        for i in 1..path.len() {
            increments.push(path[i] - path[i - 1]);
        }

        let (mean, var) = mean_variance(&increments);
        assert!(mean.abs() < 0.05);
        assert!((var - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_gbm_positive() {
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(42.0)], &mut ctx);

        let result = intrinsic_gbm(
            &[
                ValueWord::from_f64(100.0),
                ValueWord::from_f64(1.0 / 252.0),
                ValueWord::from_f64(0.1),
                ValueWord::from_f64(0.2),
                ValueWord::from_f64(100.0),
            ],
            &mut ctx,
        )
        .unwrap();

        let arr = result.as_any_array().expect("Expected array").to_generic();
        for v in arr.iter() {
            if let Some(n) = v.as_number_coerce() {
                assert!(n > 0.0);
            }
        }
    }

    #[test]
    fn test_random_walk_steps() {
        let mut ctx = ExecutionContext::new_empty();
        let _ = random_intrinsics::intrinsic_random_seed(&[ValueWord::from_f64(11.0)], &mut ctx);

        let step = 2.0;
        let result = intrinsic_random_walk(
            &[ValueWord::from_f64(101.0), ValueWord::from_f64(step)],
            &mut ctx,
        )
        .unwrap();

        let arr = result.as_any_array().expect("Expected array").to_generic();
        let path: Vec<f64> = arr
            .iter()
            .map(|v| v.as_number_coerce().unwrap_or(0.0))
            .collect();

        for i in 1..path.len() {
            let diff = (path[i] - path[i - 1]).abs();
            assert!((diff - step).abs() < 1e-9);
        }
    }
}
