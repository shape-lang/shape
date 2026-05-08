//! Stochastic process intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, all 4 stochastic
//! process intrinsics (`brownian_motion`, `gbm`, `ou_process`, `random_walk`)
//! migrate to `register_typed_fn_N` typed entries via
//! [`create_stochastic_intrinsics_module`]. Inputs are scalar `i64`/`f64`;
//! outputs project through `ConcreteReturn::ArrayF64`.
//!
//! `gbm` (arity 5) and `ou_process` (arity 6) use the N2 marshal-API
//! arity extension (`register_typed_fn_5` / `register_typed_fn_6`,
//! committed in `5dcb1ce` per supervisor sign-off on N2 sync-only at
//! first landing). `brownian_motion` (arity 3) and `random_walk` (arity 2)
//! use the pre-existing arity-3 / arity-2 helpers.
//!
//! Sampling uses the thread-local RNG from the `random` module via
//! `random::with_rng`.

use super::random;
use crate::marshal::{
    register_typed_fn_2, register_typed_fn_3, register_typed_fn_5, register_typed_fn_6,
};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use rand::Rng;

// ───────────────────── Module factory (4 typed entries) ─────────────────────

/// Create the stochastic intrinsics module with all 4 typed-marshal entry points.
pub fn create_stochastic_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::stochastic");
    module.description = "Stochastic process intrinsics (Brownian motion, GBM, OU, random walk)"
        .to_string();

    register_typed_fn_3::<_, i64, f64, f64>(
        &mut module,
        "__intrinsic_brownian_motion",
        "Brownian motion path of length n with timestep dt and volatility sigma",
        [("n", "int"), ("dt", "number"), ("sigma", "number")],
        ConcreteType::ArrayNumber,
        |n, dt, sigma, _ctx| {
            if n < 0 {
                return Err("__intrinsic_brownian_motion: n must be non-negative".to_string());
            }
            if dt <= 0.0 {
                return Err("__intrinsic_brownian_motion: dt must be positive".to_string());
            }
            if sigma < 0.0 {
                return Err(
                    "__intrinsic_brownian_motion: sigma must be non-negative".to_string()
                );
            }
            let n = n as usize;
            let scale = sigma * dt.sqrt();
            let path = random::with_rng(|rng| {
                let mut path = Vec::with_capacity(n);
                let mut x = 0.0;
                for i in 0..n {
                    if i > 0 {
                        x += scale * standard_normal(rng);
                    }
                    path.push(x);
                }
                path
            });
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(path)))
        },
    );

    register_typed_fn_5::<_, i64, f64, f64, f64, f64>(
        &mut module,
        "__intrinsic_gbm",
        "Geometric Brownian Motion path: s0 * exp((mu - 0.5*sigma^2)*dt + sigma*sqrt(dt)*Z)",
        [
            ("n", "int"),
            ("dt", "number"),
            ("mu", "number"),
            ("sigma", "number"),
            ("s0", "number"),
        ],
        ConcreteType::ArrayNumber,
        |n, dt, mu, sigma, s0, _ctx| {
            if n < 0 {
                return Err("__intrinsic_gbm: n must be non-negative".to_string());
            }
            if dt <= 0.0 {
                return Err("__intrinsic_gbm: dt must be positive".to_string());
            }
            if sigma < 0.0 {
                return Err("__intrinsic_gbm: sigma must be non-negative".to_string());
            }
            if s0 <= 0.0 {
                return Err("__intrinsic_gbm: s0 must be positive".to_string());
            }
            let n = n as usize;
            let drift = (mu - 0.5 * sigma * sigma) * dt;
            let diffusion_scale = sigma * dt.sqrt();
            let path = random::with_rng(|rng| {
                let mut path = Vec::with_capacity(n);
                let mut s = s0;
                for i in 0..n {
                    if i > 0 {
                        let z = standard_normal(rng);
                        s *= (drift + diffusion_scale * z).exp();
                    }
                    path.push(s);
                }
                path
            });
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(path)))
        },
    );

    register_typed_fn_6::<_, i64, f64, f64, f64, f64, f64>(
        &mut module,
        "__intrinsic_ou_process",
        "Ornstein-Uhlenbeck mean-reverting process: dx = theta*(mu - x)*dt + sigma*sqrt(dt)*Z",
        [
            ("n", "int"),
            ("dt", "number"),
            ("theta", "number"),
            ("mu", "number"),
            ("sigma", "number"),
            ("x0", "number"),
        ],
        ConcreteType::ArrayNumber,
        |n, dt, theta, mu, sigma, x0, _ctx| {
            if n < 0 {
                return Err("__intrinsic_ou_process: n must be non-negative".to_string());
            }
            if dt <= 0.0 {
                return Err("__intrinsic_ou_process: dt must be positive".to_string());
            }
            if theta < 0.0 {
                return Err("__intrinsic_ou_process: theta must be non-negative".to_string());
            }
            if sigma < 0.0 {
                return Err("__intrinsic_ou_process: sigma must be non-negative".to_string());
            }
            let n = n as usize;
            let diffusion_scale = sigma * dt.sqrt();
            let path = random::with_rng(|rng| {
                let mut path = Vec::with_capacity(n);
                let mut x = x0;
                for i in 0..n {
                    if i > 0 {
                        let z = standard_normal(rng);
                        x += theta * (mu - x) * dt + diffusion_scale * z;
                    }
                    path.push(x);
                }
                path
            });
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(path)))
        },
    );

    register_typed_fn_2::<_, i64, f64>(
        &mut module,
        "__intrinsic_random_walk",
        "Discrete +/- step_size random walk of length n",
        [("n", "int"), ("step_size", "number")],
        ConcreteType::ArrayNumber,
        |n, step_size, _ctx| {
            if n < 0 {
                return Err("__intrinsic_random_walk: n must be non-negative".to_string());
            }
            if step_size <= 0.0 {
                return Err("__intrinsic_random_walk: step_size must be positive".to_string());
            }
            let n = n as usize;
            let path = random::with_rng(|rng| {
                let mut path = Vec::with_capacity(n);
                let mut x = 0.0;
                for i in 0..n {
                    if i > 0 {
                        let step = if rng.r#gen::<f64>() < 0.5 {
                            -step_size
                        } else {
                            step_size
                        };
                        x += step;
                    }
                    path.push(x);
                }
                path
            });
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(path)))
        },
    );

    module
}

// ───────────────────── Helpers (used by typed bodies) ─────────────────────

/// Sample a standard normal (Box-Muller).
#[inline]
fn standard_normal(rng: &mut rand_chacha::ChaCha8Rng) -> f64 {
    let u1: f64 = rng.r#gen();
    let u2: f64 = rng.r#gen();
    (-2.0_f64 * u1.ln()).sqrt() * (2.0_f64 * std::f64::consts::PI * u2).cos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intrinsics::random as random_intrinsics;
    use rand::SeedableRng;

    fn mean_variance(samples: &[f64]) -> (f64, f64) {
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        (mean, var)
    }

    #[test]
    fn test_brownian_motion_increments_unit_variance() {
        random_intrinsics::with_rng(|rng| {
            *rng = rand_chacha::ChaCha8Rng::seed_from_u64(999);
        });

        let path = random_intrinsics::with_rng(|rng| {
            let n = 10001usize;
            let scale = 1.0_f64 * 1.0_f64.sqrt();
            let mut path = Vec::with_capacity(n);
            let mut x = 0.0;
            for i in 0..n {
                if i > 0 {
                    x += scale * standard_normal(rng);
                }
                path.push(x);
            }
            path
        });

        let increments: Vec<f64> = (1..path.len()).map(|i| path[i] - path[i - 1]).collect();
        let (mean, var) = mean_variance(&increments);
        assert!(mean.abs() < 0.05);
        assert!((var - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_gbm_positive_path() {
        random_intrinsics::with_rng(|rng| {
            *rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        });

        let n = 100usize;
        let dt: f64 = 1.0 / 252.0;
        let mu = 0.1;
        let sigma = 0.2;
        let s0 = 100.0;
        let drift = (mu - 0.5 * sigma * sigma) * dt;
        let diffusion_scale = sigma * dt.sqrt();
        let path = random_intrinsics::with_rng(|rng| {
            let mut path = Vec::with_capacity(n);
            let mut s = s0;
            for i in 0..n {
                if i > 0 {
                    let z = standard_normal(rng);
                    s *= (drift + diffusion_scale * z).exp();
                }
                path.push(s);
            }
            path
        });
        for &v in &path {
            assert!(v > 0.0);
        }
    }

    #[test]
    fn test_random_walk_step_size_invariant() {
        random_intrinsics::with_rng(|rng| {
            *rng = rand_chacha::ChaCha8Rng::seed_from_u64(11);
        });

        let n = 101usize;
        let step = 2.0;
        let path: Vec<f64> = random_intrinsics::with_rng(|rng| {
            let mut path = Vec::with_capacity(n);
            let mut x = 0.0;
            for i in 0..n {
                if i > 0 {
                    let s = if rng.r#gen::<f64>() < 0.5 {
                        -step
                    } else {
                        step
                    };
                    x += s;
                }
                path.push(x);
            }
            path
        });

        for i in 1..path.len() {
            let diff = (path[i] - path[i - 1]).abs();
            assert!((diff - step).abs() < 1e-9);
        }
    }
}
