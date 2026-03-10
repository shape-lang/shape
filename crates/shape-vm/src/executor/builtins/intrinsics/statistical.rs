//! Statistical intrinsics — correlation, covariance, percentile, median,
//! distributions, stochastic processes, and random number generation.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use shape_value::{VMError, ValueWord};
use std::cell::RefCell;
use std::sync::Arc;

use super::{NbIntrinsicResult, nb_extract_f64_data};

// Thread-local RNG for random intrinsics
thread_local! {
    static RNG: RefCell<ChaCha8Rng> = RefCell::new(ChaCha8Rng::from_entropy());
}

// =============================================================================
// Random Number Generation
// =============================================================================

/// Generate random f64 in [0, 1)
pub fn vm_intrinsic_random(args: &[ValueWord]) -> NbIntrinsicResult {
    if !args.is_empty() {
        return Err(VMError::RuntimeError(
            "random() takes no arguments".to_string(),
        ));
    }

    let value = RNG.with(|rng| rng.borrow_mut().r#gen::<f64>());
    Ok(ValueWord::from_f64(value))
}

/// Generate random integer in [lo, hi] (inclusive)
pub fn vm_intrinsic_random_int(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "random_int() requires 2 arguments (lo, hi)".to_string(),
        ));
    }

    let lo = args[0]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("random_int: lo must be a number".to_string()))?
        as i64;
    let hi = args[1]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("random_int: hi must be a number".to_string()))?
        as i64;

    if lo > hi {
        return Err(VMError::RuntimeError(format!(
            "random_int: lo ({}) must be <= hi ({})",
            lo, hi
        )));
    }

    let value = RNG.with(|rng| rng.borrow_mut().gen_range(lo..=hi));
    Ok(ValueWord::from_f64(value as f64))
}

/// Seed the RNG for reproducibility
pub fn vm_intrinsic_random_seed(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(
            "random_seed() requires 1 argument (seed)".to_string(),
        ));
    }

    let seed = args[0]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("random_seed: seed must be a number".to_string()))?
        as u64;

    RNG.with(|rng| {
        *rng.borrow_mut() = ChaCha8Rng::seed_from_u64(seed);
    });

    Ok(ValueWord::unit())
}

/// Generate random number from normal distribution (Box-Muller transform)
pub fn vm_intrinsic_random_normal(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "random_normal() requires 2 arguments (mean, std)".to_string(),
        ));
    }

    let mean = args[0]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("random_normal: mean must be a number".to_string()))?;
    let std = args[1]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("random_normal: std must be a number".to_string()))?;

    if std < 0.0 {
        return Err(VMError::RuntimeError(
            "random_normal: std must be non-negative".to_string(),
        ));
    }

    // Box-Muller transform
    let value = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let u1: f64 = rng.r#gen();
        let u2: f64 = rng.r#gen();

        let z = (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos();
        mean + std * z
    });

    Ok(ValueWord::from_f64(value))
}

/// Generate array of n random numbers in [0, 1)
pub fn vm_intrinsic_random_array(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(
            "random_array() requires 1 argument (n)".to_string(),
        ));
    }

    let n = args[0]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("random_array: n must be a number".to_string()))?
        as usize;

    let values: Vec<ValueWord> = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        (0..n)
            .map(|_| ValueWord::from_f64(rng.r#gen::<f64>()))
            .collect()
    });

    Ok(ValueWord::from_array(Arc::new(values)))
}

// =============================================================================
// Distribution Intrinsics
// =============================================================================

fn sample_standard_normal() -> f64 {
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let u1: f64 = rng.r#gen();
        let u2: f64 = rng.r#gen();
        (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos()
    })
}

/// Sample from uniform distribution [lo, hi)
pub fn vm_intrinsic_dist_uniform(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "dist_uniform() requires 2 arguments (lo, hi)".to_string(),
        ));
    }

    let lo = args[0]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("dist_uniform: lo must be a number".to_string()))?;
    let hi = args[1]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("dist_uniform: hi must be a number".to_string()))?;

    if lo >= hi {
        return Err(VMError::RuntimeError(
            "dist_uniform: lo must be < hi".to_string(),
        ));
    }

    let value = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let u: f64 = rng.r#gen();
        lo + (hi - lo) * u
    });

    Ok(ValueWord::from_f64(value))
}

/// Sample from lognormal distribution
pub fn vm_intrinsic_dist_lognormal(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "dist_lognormal() requires 2 arguments (mean, std)".to_string(),
        ));
    }

    let mean = args[0].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("dist_lognormal: mean must be a number".to_string())
    })?;
    let std = args[1]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("dist_lognormal: std must be a number".to_string()))?;

    if std < 0.0 {
        return Err(VMError::RuntimeError(
            "dist_lognormal: std must be non-negative".to_string(),
        ));
    }

    let z = sample_standard_normal();
    Ok(ValueWord::from_f64((mean + std * z).exp()))
}

/// Sample from exponential distribution
pub fn vm_intrinsic_dist_exponential(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(
            "dist_exponential() requires 1 argument (lambda)".to_string(),
        ));
    }

    let lambda = args[0].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("dist_exponential: lambda must be a number".to_string())
    })?;

    if lambda <= 0.0 {
        return Err(VMError::RuntimeError(
            "dist_exponential: lambda must be positive".to_string(),
        ));
    }

    let value = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let u: f64 = rng.r#gen();
        -u.ln() / lambda
    });

    Ok(ValueWord::from_f64(value))
}

/// Sample from Poisson distribution
pub fn vm_intrinsic_dist_poisson(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(
            "dist_poisson() requires 1 argument (lambda)".to_string(),
        ));
    }

    let lambda = args[0].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("dist_poisson: lambda must be a number".to_string())
    })?;

    if lambda < 0.0 {
        return Err(VMError::RuntimeError(
            "dist_poisson: lambda must be non-negative".to_string(),
        ));
    }

    let value = if lambda < 30.0 {
        // Knuth's algorithm
        RNG.with(|rng| {
            let mut rng = rng.borrow_mut();
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
        // Normal approximation
        let z = sample_standard_normal();
        let value = lambda + lambda.sqrt() * z;
        value.max(0.0).round()
    };

    Ok(ValueWord::from_f64(value))
}

/// Sample n values from a named distribution
pub fn vm_intrinsic_dist_sample_n(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "dist_sample_n() requires 3 arguments (dist_name, params, n)".to_string(),
        ));
    }

    let dist_name = args[0].as_str().ok_or_else(|| {
        VMError::RuntimeError("dist_sample_n: dist_name must be a string".to_string())
    })?;

    let params_view = args[1].as_any_array().ok_or_else(|| {
        VMError::RuntimeError("dist_sample_n: params must be an array".to_string())
    })?;
    let params: Vec<ValueWord> = params_view.to_generic().iter().cloned().collect();

    let n = args[2].as_number_coerce().ok_or_else(|| {
        VMError::RuntimeError("dist_sample_n: n must be a non-negative number".to_string())
    })?;
    if n < 0.0 {
        return Err(VMError::RuntimeError(
            "dist_sample_n: n must be a non-negative number".to_string(),
        ));
    }
    let n = n as usize;

    let mut samples: Vec<ValueWord> = Vec::with_capacity(n);
    for _ in 0..n {
        let sample = match dist_name {
            "uniform" => vm_intrinsic_dist_uniform(&params)?,
            "lognormal" => vm_intrinsic_dist_lognormal(&params)?,
            "exponential" => vm_intrinsic_dist_exponential(&params)?,
            "poisson" => vm_intrinsic_dist_poisson(&params)?,
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Unknown distribution: {}",
                    dist_name
                )));
            }
        };
        samples.push(sample);
    }

    Ok(ValueWord::from_array(Arc::new(samples)))
}

// =============================================================================
// Stochastic Process Intrinsics
// =============================================================================

/// Helper to extract a required numeric parameter from ValueWord with validation
fn nb_require_number(
    args: &[ValueWord],
    idx: usize,
    name: &str,
    param: &str,
) -> Result<f64, VMError> {
    args[idx]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError(format!("{}: {} must be a number", name, param)))
}

pub fn vm_intrinsic_brownian_motion(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "brownian_motion() requires 3 arguments (n, dt, sigma)".to_string(),
        ));
    }

    let n_f = nb_require_number(args, 0, "brownian_motion", "n")?;
    if n_f < 0.0 {
        return Err(VMError::RuntimeError(
            "brownian_motion: n must be non-negative".to_string(),
        ));
    }
    let n = n_f as usize;
    let dt = nb_require_number(args, 1, "brownian_motion", "dt")?;
    if dt <= 0.0 {
        return Err(VMError::RuntimeError(
            "brownian_motion: dt must be positive".to_string(),
        ));
    }
    let sigma = nb_require_number(args, 2, "brownian_motion", "sigma")?;
    if sigma < 0.0 {
        return Err(VMError::RuntimeError(
            "brownian_motion: sigma must be non-negative".to_string(),
        ));
    }

    let mut path: Vec<ValueWord> = Vec::with_capacity(n);
    let mut x = 0.0;
    let scale = sigma * dt.sqrt();

    for i in 0..n {
        if i > 0 {
            x += scale * sample_standard_normal();
        }
        path.push(ValueWord::from_f64(x));
    }

    Ok(ValueWord::from_array(Arc::new(path)))
}

pub fn vm_intrinsic_gbm(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "gbm() requires 5 arguments (n, dt, mu, sigma, s0)".to_string(),
        ));
    }

    let n_f = nb_require_number(args, 0, "gbm", "n")?;
    if n_f < 0.0 {
        return Err(VMError::RuntimeError(
            "gbm: n must be non-negative".to_string(),
        ));
    }
    let n = n_f as usize;
    let dt = nb_require_number(args, 1, "gbm", "dt")?;
    if dt <= 0.0 {
        return Err(VMError::RuntimeError(
            "gbm: dt must be positive".to_string(),
        ));
    }
    let mu = nb_require_number(args, 2, "gbm", "mu")?;
    let sigma = nb_require_number(args, 3, "gbm", "sigma")?;
    if sigma < 0.0 {
        return Err(VMError::RuntimeError(
            "gbm: sigma must be non-negative".to_string(),
        ));
    }
    let s0 = nb_require_number(args, 4, "gbm", "s0")?;
    if s0 <= 0.0 {
        return Err(VMError::RuntimeError(
            "gbm: s0 must be positive".to_string(),
        ));
    }

    let mut path: Vec<ValueWord> = Vec::with_capacity(n);
    let mut s = s0;
    let drift = (mu - 0.5 * sigma * sigma) * dt;
    let diffusion_scale = sigma * dt.sqrt();

    for i in 0..n {
        if i > 0 {
            let z = sample_standard_normal();
            s *= (drift + diffusion_scale * z).exp();
        }
        path.push(ValueWord::from_f64(s));
    }

    Ok(ValueWord::from_array(Arc::new(path)))
}

pub fn vm_intrinsic_ou_process(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 6 {
        return Err(VMError::RuntimeError(
            "ou_process() requires 6 arguments (n, dt, theta, mu, sigma, x0)".to_string(),
        ));
    }

    let n_f = nb_require_number(args, 0, "ou_process", "n")?;
    if n_f < 0.0 {
        return Err(VMError::RuntimeError(
            "ou_process: n must be non-negative".to_string(),
        ));
    }
    let n = n_f as usize;
    let dt = nb_require_number(args, 1, "ou_process", "dt")?;
    if dt <= 0.0 {
        return Err(VMError::RuntimeError(
            "ou_process: dt must be positive".to_string(),
        ));
    }
    let theta = nb_require_number(args, 2, "ou_process", "theta")?;
    if theta < 0.0 {
        return Err(VMError::RuntimeError(
            "ou_process: theta must be non-negative".to_string(),
        ));
    }
    let mu = nb_require_number(args, 3, "ou_process", "mu")?;
    let sigma = nb_require_number(args, 4, "ou_process", "sigma")?;
    if sigma < 0.0 {
        return Err(VMError::RuntimeError(
            "ou_process: sigma must be non-negative".to_string(),
        ));
    }
    let x0 = nb_require_number(args, 5, "ou_process", "x0")?;

    let mut path: Vec<ValueWord> = Vec::with_capacity(n);
    let mut x = x0;
    let diffusion_scale = sigma * dt.sqrt();

    for i in 0..n {
        if i > 0 {
            let z = sample_standard_normal();
            x += theta * (mu - x) * dt + diffusion_scale * z;
        }
        path.push(ValueWord::from_f64(x));
    }

    Ok(ValueWord::from_array(Arc::new(path)))
}

pub fn vm_intrinsic_random_walk(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "random_walk() requires 2 arguments (n, step_size)".to_string(),
        ));
    }

    let n_f = nb_require_number(args, 0, "random_walk", "n")?;
    if n_f < 0.0 {
        return Err(VMError::RuntimeError(
            "random_walk: n must be non-negative".to_string(),
        ));
    }
    let n = n_f as usize;
    let step_size = nb_require_number(args, 1, "random_walk", "step_size")?;
    if step_size <= 0.0 {
        return Err(VMError::RuntimeError(
            "random_walk: step_size must be positive".to_string(),
        ));
    }

    let mut path: Vec<ValueWord> = Vec::with_capacity(n);
    let mut x = 0.0;

    for i in 0..n {
        if i > 0 {
            let step = RNG.with(|rng| {
                if rng.borrow_mut().r#gen::<f64>() < 0.5 {
                    -step_size
                } else {
                    step_size
                }
            });
            x += step;
        }
        path.push(ValueWord::from_f64(x));
    }

    Ok(ValueWord::from_array(Arc::new(path)))
}

// =============================================================================
// Statistical Intrinsics
// =============================================================================

/// Pearson correlation coefficient
pub fn vm_intrinsic_correlation(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "correlation() requires 2 arguments (series_a, series_b)".to_string(),
        ));
    }

    let data_a = nb_extract_f64_data(&args[0])?;
    let data_b = nb_extract_f64_data(&args[1])?;

    if data_a.len() != data_b.len() {
        return Err(VMError::RuntimeError(format!(
            "Array lengths must match: {} != {}",
            data_a.len(),
            data_b.len()
        )));
    }

    if data_a.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let result = shape_runtime::simd_statistics::correlation(&data_a, &data_b);
    Ok(ValueWord::from_f64(result))
}

/// Covariance
pub fn vm_intrinsic_covariance(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "covariance() requires 2 arguments (series_a, series_b)".to_string(),
        ));
    }

    let data_a = nb_extract_f64_data(&args[0])?;
    let data_b = nb_extract_f64_data(&args[1])?;

    if data_a.len() != data_b.len() {
        return Err(VMError::RuntimeError(
            "Array lengths must match".to_string(),
        ));
    }

    if data_a.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let result = shape_runtime::simd_statistics::covariance(&data_a, &data_b);
    Ok(ValueWord::from_f64(result))
}

/// Percentile calculation using quickselect
pub fn vm_intrinsic_percentile(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "percentile() requires 2 arguments (series, percentile)".to_string(),
        ));
    }

    let mut data = nb_extract_f64_data(&args[0])?;

    let percentile = args[1]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("Percentile must be a number".to_string()))?;
    if percentile < 0.0 || percentile > 100.0 {
        return Err(VMError::RuntimeError(
            "Percentile must be between 0 and 100".to_string(),
        ));
    }

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let n = data.len();
    let k = ((percentile / 100.0) * (n - 1) as f64).round() as usize;
    let k = k.min(n - 1);

    let result = quickselect(&mut data, k);
    Ok(ValueWord::from_f64(result))
}

/// Median (50th percentile)
pub fn vm_intrinsic_median(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "median() requires 1 argument (series)".to_string(),
        ));
    }

    let mut data = nb_extract_f64_data(&args[0])?;
    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = data.len();
    let result = if n % 2 == 0 {
        (data[n / 2 - 1] + data[n / 2]) / 2.0
    } else {
        data[n / 2]
    };
    Ok(ValueWord::from_f64(result))
}

// =============================================================================
// Quickselect Algorithm
// =============================================================================

/// Quickselect for O(n) average case percentile calculation
fn quickselect(arr: &mut [f64], k: usize) -> f64 {
    if arr.len() == 1 {
        return arr[0];
    }

    let k = k.min(arr.len() - 1);
    let mut left = 0;
    let mut right = arr.len() - 1;

    loop {
        if left == right {
            return arr[left];
        }

        // Choose pivot as median of three
        let mid = left + (right - left) / 2;
        let pivot_idx = median_of_three(arr, left, mid, right);

        // Partition
        let pivot_idx = partition(arr, left, right, pivot_idx);

        if k == pivot_idx {
            return arr[k];
        } else if k < pivot_idx {
            right = pivot_idx.saturating_sub(1);
        } else {
            left = pivot_idx + 1;
        }
    }
}

fn median_of_three(arr: &[f64], a: usize, b: usize, c: usize) -> usize {
    if (arr[a] <= arr[b] && arr[b] <= arr[c]) || (arr[c] <= arr[b] && arr[b] <= arr[a]) {
        b
    } else if (arr[b] <= arr[a] && arr[a] <= arr[c]) || (arr[c] <= arr[a] && arr[a] <= arr[b]) {
        a
    } else {
        c
    }
}

fn partition(arr: &mut [f64], left: usize, right: usize, pivot_idx: usize) -> usize {
    let pivot_value = arr[pivot_idx];
    arr.swap(pivot_idx, right);

    let mut store_idx = left;
    for i in left..right {
        if arr[i] < pivot_value {
            arr.swap(i, store_idx);
            store_idx += 1;
        }
    }

    arr.swap(store_idx, right);
    store_idx
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

    #[test]
    fn test_percentile() {
        let arr = make_nb_array(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        let p50 = ValueWord::from_f64(50.0);

        let result = vm_intrinsic_percentile(&[arr, p50]).unwrap();

        let n = result.as_number_coerce().expect("Expected number");
        assert!((n - 5.0).abs() < 1.0 || (n - 6.0).abs() < 1.0);
    }

    #[test]
    fn test_median() {
        let arr = make_nb_array(vec![1.0, 3.0, 5.0, 7.0, 9.0]);

        let result = vm_intrinsic_median(&[arr]).unwrap();

        let n = result.as_number_coerce().expect("Expected number");
        assert_eq!(n, 5.0);
    }
}
