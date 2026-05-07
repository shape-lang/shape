//! Statistical distribution intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, all 5 distribution
//! intrinsics (`dist_uniform`, `dist_lognormal`, `dist_exponential`,
//! `dist_poisson`, `dist_sample_n`) migrate to `register_typed_fn_N` typed
//! entries via [`create_distributions_intrinsics_module`].
//!
//! `dist_sample_n` previously called sibling `intrinsic_dist_*` legacy bodies
//! via `&[ValueWord]`; bodies are now refactored to delegate to pure-helper
//! functions (`sample_uniform`, `sample_lognormal`, etc.) so both the typed
//! entries and `dist_sample_n` share the same sampling math without going
//! through the marshal layer twice.
//!
//! Sampling uses the thread-local RNG from the `random` module via
//! `random::with_rng`.

use super::random;
use crate::marshal::{register_typed_fn_1, register_typed_fn_2, register_typed_fn_3};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use shape_value::AlignedTypedBuffer;
use std::sync::Arc;

// ───────────────────── Module factory (5 typed entries) ─────────────────────

/// Create the distributions intrinsics module with all 5 typed-marshal entry points.
pub fn create_distributions_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::distributions");
    module.description =
        "Statistical distribution sampling intrinsics (uniform, lognormal, exponential, poisson, sample_n)"
            .to_string();

    register_typed_fn_2::<_, f64, f64>(
        &mut module,
        "__intrinsic_dist_uniform",
        "Sample from a uniform distribution [lo, hi)",
        [("lo", "number"), ("hi", "number")],
        ConcreteType::Number,
        |lo, hi, _ctx| {
            if lo >= hi {
                return Err(format!(
                    "__intrinsic_dist_uniform: lo ({}) must be < hi ({})",
                    lo, hi
                ));
            }
            let value = random::with_rng(|rng| sample_uniform(rng, lo, hi));
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value)))
        },
    );

    register_typed_fn_2::<_, f64, f64>(
        &mut module,
        "__intrinsic_dist_lognormal",
        "Sample from a lognormal distribution",
        [("mean", "number"), ("std", "number")],
        ConcreteType::Number,
        |mean, std, _ctx| {
            if std < 0.0 {
                return Err("__intrinsic_dist_lognormal: std must be non-negative".to_string());
            }
            let value = random::with_rng(|rng| sample_lognormal(rng, mean, std));
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value)))
        },
    );

    register_typed_fn_1::<_, f64>(
        &mut module,
        "__intrinsic_dist_exponential",
        "Sample from an exponential distribution",
        "lambda",
        "number",
        ConcreteType::Number,
        |lambda, _ctx| {
            if lambda <= 0.0 {
                return Err("__intrinsic_dist_exponential: lambda must be positive".to_string());
            }
            let value = random::with_rng(|rng| sample_exponential(rng, lambda));
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value)))
        },
    );

    register_typed_fn_1::<_, f64>(
        &mut module,
        "__intrinsic_dist_poisson",
        "Sample from a Poisson distribution",
        "lambda",
        "number",
        ConcreteType::Number,
        |lambda, _ctx| {
            if lambda < 0.0 {
                return Err("__intrinsic_dist_poisson: lambda must be non-negative".to_string());
            }
            let value = random::with_rng(|rng| sample_poisson(rng, lambda));
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value)))
        },
    );

    register_typed_fn_3::<_, Arc<String>, Arc<AlignedTypedBuffer>, i64>(
        &mut module,
        "__intrinsic_dist_sample_n",
        "Sample n values from a named distribution (uniform / lognormal / exponential / poisson)",
        [
            ("dist_name", "string"),
            ("params", "Array<number>"),
            ("n", "int"),
        ],
        ConcreteType::ArrayNumber,
        |dist_name, params, n, _ctx| {
            if n < 0 {
                return Err("__intrinsic_dist_sample_n: n must be non-negative".to_string());
            }
            let n = n as usize;
            let p = params.as_slice();
            let dist = dist_name.as_str();

            // Validate parameter shape for the named distribution.
            match dist {
                "uniform" | "lognormal" => {
                    if p.len() != 2 {
                        return Err(format!(
                            "__intrinsic_dist_sample_n: '{}' requires 2 params, got {}",
                            dist,
                            p.len()
                        ));
                    }
                }
                "exponential" | "poisson" => {
                    if p.len() != 1 {
                        return Err(format!(
                            "__intrinsic_dist_sample_n: '{}' requires 1 param, got {}",
                            dist,
                            p.len()
                        ));
                    }
                }
                _ => return Err(format!("Unknown distribution: {}", dist)),
            }

            // Per-distribution validity preconditions (mirror per-call typed entries).
            match dist {
                "uniform" if p[0] >= p[1] => {
                    return Err(format!(
                        "__intrinsic_dist_sample_n: uniform lo ({}) must be < hi ({})",
                        p[0], p[1]
                    ));
                }
                "lognormal" if p[1] < 0.0 => {
                    return Err(
                        "__intrinsic_dist_sample_n: lognormal std must be non-negative".to_string(),
                    );
                }
                "exponential" if p[0] <= 0.0 => {
                    return Err(
                        "__intrinsic_dist_sample_n: exponential lambda must be positive"
                            .to_string(),
                    );
                }
                "poisson" if p[0] < 0.0 => {
                    return Err(
                        "__intrinsic_dist_sample_n: poisson lambda must be non-negative"
                            .to_string(),
                    );
                }
                _ => {}
            }

            let samples: Vec<f64> = random::with_rng(|rng| {
                (0..n)
                    .map(|_| match dist {
                        "uniform" => sample_uniform(rng, p[0], p[1]),
                        "lognormal" => sample_lognormal(rng, p[0], p[1]),
                        "exponential" => sample_exponential(rng, p[0]),
                        "poisson" => sample_poisson(rng, p[0]),
                        _ => unreachable!(),
                    })
                    .collect()
            });

            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(samples)))
        },
    );

    module
}

// ───────────────────── Sampling helpers (used by typed entries) ─────────────────────

/// Sample from a uniform distribution [lo, hi).
fn sample_uniform(rng: &mut ChaCha8Rng, lo: f64, hi: f64) -> f64 {
    let u: f64 = rng.r#gen();
    lo + (hi - lo) * u
}

/// Sample from a lognormal distribution via Box-Muller normal sampling.
fn sample_lognormal(rng: &mut ChaCha8Rng, mean: f64, std: f64) -> f64 {
    let u1: f64 = rng.r#gen();
    let u2: f64 = rng.r#gen();
    let z = (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos();
    (mean + std * z).exp()
}

/// Sample from an exponential distribution (inverse-CDF).
fn sample_exponential(rng: &mut ChaCha8Rng, lambda: f64) -> f64 {
    let u: f64 = rng.r#gen();
    -u.ln() / lambda
}

/// Sample from a Poisson distribution.
///
/// For lambda < 30 uses Knuth's multiplicative method; for lambda >= 30 uses
/// a normal approximation rounded to the nearest non-negative integer.
fn sample_poisson(rng: &mut ChaCha8Rng, lambda: f64) -> f64 {
    if lambda < 30.0 {
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
    } else {
        let u1: f64 = rng.r#gen();
        let u2: f64 = rng.r#gen();
        let z = (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos();
        let value = lambda + lambda.sqrt() * z;
        value.max(0.0).round()
    }
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
        random_intrinsics::with_rng(|rng| {
            *rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        });

        let samples: Vec<f64> =
            random_intrinsics::with_rng(|rng| (0..20000).map(|_| sample_uniform(rng, 0.0, 2.0)).collect());

        let (mean, var) = mean_variance(&samples);
        assert!((mean - 1.0).abs() < 0.05);
        assert!((var - 1.0 / 3.0).abs() < 0.05);
    }

    #[test]
    fn test_exponential_mean_variance() {
        random_intrinsics::with_rng(|rng| {
            *rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        });

        let lambda = 2.0;
        let samples: Vec<f64> = random_intrinsics::with_rng(|rng| {
            (0..20000).map(|_| sample_exponential(rng, lambda)).collect()
        });
        let (mean, var) = mean_variance(&samples);
        assert!((mean - 1.0 / lambda).abs() < 0.05);
        assert!((var - 1.0 / (lambda * lambda)).abs() < 0.1);
    }

    #[test]
    fn test_poisson_mean_variance() {
        random_intrinsics::with_rng(|rng| {
            *rng = rand_chacha::ChaCha8Rng::seed_from_u64(123);
        });

        let lambda = 12.0;
        let samples: Vec<f64> = random_intrinsics::with_rng(|rng| {
            (0..20000).map(|_| sample_poisson(rng, lambda)).collect()
        });
        let (mean, var) = mean_variance(&samples);
        assert!((mean - lambda).abs() < 0.3);
        assert!((var - lambda).abs() < 0.5);
    }
}
