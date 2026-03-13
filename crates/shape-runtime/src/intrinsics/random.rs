//! Random number generation intrinsics
//!
//! Provides high-quality PRNG using ChaCha8 for reproducibility and performance.
//! Thread-local state ensures zero contention in parallel contexts.

use super::{extract_f64, extract_usize, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;
use std::cell::RefCell;

thread_local! {
    static RNG: RefCell<ChaCha8Rng> = RefCell::new(ChaCha8Rng::from_entropy());
}

/// Access the shared thread-local RNG.
///
/// Public so that shape-vm can share the same RNG state when delegating
/// random/distribution/stochastic intrinsics to the runtime.
pub fn with_rng<F, R>(f: F) -> R
where
    F: FnOnce(&mut ChaCha8Rng) -> R,
{
    RNG.with(|rng| f(&mut *rng.borrow_mut()))
}

/// Intrinsic: Generate random f64 in [0, 1)
pub fn intrinsic_random(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if !args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random takes no arguments".to_string(),
            location: None,
        });
    }

    let value = RNG.with(|rng| rng.borrow_mut().r#gen::<f64>());
    Ok(ValueWord::from_f64(value))
}

/// Intrinsic: Generate random integer in [lo, hi] (inclusive)
pub fn intrinsic_random_int(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_int requires 2 arguments (lo, hi)".to_string(),
            location: None,
        });
    }

    let lo = extract_f64(&args[0], "__intrinsic_random_int: lo")? as i64;
    let hi = extract_f64(&args[1], "__intrinsic_random_int: hi")? as i64;

    if lo > hi {
        return Err(ShapeError::RuntimeError {
            message: format!("__intrinsic_random_int: lo ({}) must be <= hi ({})", lo, hi),
            location: None,
        });
    }

    let value = RNG.with(|rng| rng.borrow_mut().gen_range(lo..=hi));
    Ok(ValueWord::from_f64(value as f64))
}

/// Intrinsic: Seed the RNG for reproducibility
pub fn intrinsic_random_seed(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_seed requires 1 argument (seed)".to_string(),
            location: None,
        });
    }

    let seed = extract_f64(&args[0], "__intrinsic_random_seed: seed")? as u64;

    RNG.with(|rng| {
        *rng.borrow_mut() = ChaCha8Rng::seed_from_u64(seed);
    });

    Ok(ValueWord::unit())
}

/// Intrinsic: Generate random number from normal distribution
pub fn intrinsic_random_normal(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_normal requires 2 arguments (mean, std)".to_string(),
            location: None,
        });
    }

    let mean = extract_f64(&args[0], "__intrinsic_random_normal: mean")?;
    let std = extract_f64(&args[1], "__intrinsic_random_normal: std")?;

    if std < 0.0 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_normal: std must be non-negative".to_string(),
            location: None,
        });
    }

    let value = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let u1: f64 = rng.r#gen();
        let u2: f64 = rng.r#gen();
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        mean + std * z
    });

    Ok(ValueWord::from_f64(value))
}

/// Intrinsic: Generate array of n random numbers in [0, 1)
pub fn intrinsic_random_array(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_random_array requires 1 argument (n)".to_string(),
            location: None,
        });
    }

    let n = extract_usize(&args[0], "__intrinsic_random_array: n")?;

    let values: Vec<f64> = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        (0..n).map(|_| rng.r#gen::<f64>()).collect()
    });

    Ok(f64_vec_to_nb_array(values))
}
