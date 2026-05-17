//! Random number generation intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, all 5 random
//! intrinsics (`random`, `random_int`, `random_seed`, `random_normal`,
//! `random_array`) migrate to `register_typed_fn_N` typed entries via
//! [`create_random_intrinsics_module`].
//!
//! Thread-local `ChaCha8Rng` state is observably-stateful FFI behavior at
//! the runtime layer; the marshal-API surface treats each intrinsic as
//! pure-from-thread-local (each call mutates RNG, returns f64/Unit/array).
//! `with_rng` stays `pub` for shape-vm to share the same RNG state when
//! delegating random/distribution/stochastic intrinsics to the runtime.
//!
//! Provides high-quality PRNG using ChaCha8 for reproducibility and performance.
//! Thread-local state ensures zero contention in parallel contexts.

use crate::marshal::{register_typed_fn_0, register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
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

// ───────────────────── Module factory (5 typed entries) ─────────────────────

/// Create the random intrinsics module with all 5 typed-marshal entry points.
pub fn create_random_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::random");
    module.description =
        "Random number generation intrinsics (ChaCha8 thread-local PRNG)".to_string();

    register_typed_fn_0::<_>(
        &mut module,
        "__intrinsic_random",
        "Generate random f64 in [0, 1)",
        ConcreteType::Number,
        |_ctx| {
            let value = with_rng(|rng| rng.r#gen::<f64>());
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value)))
        },
    );

    register_typed_fn_2::<_, f64, f64>(
        &mut module,
        "__intrinsic_random_int",
        "Generate random integer in [lo, hi] (inclusive); returns as number",
        [("lo", "number"), ("hi", "number")],
        ConcreteType::Number,
        |lo, hi, _ctx| {
            let lo = lo as i64;
            let hi = hi as i64;
            if lo > hi {
                return Err(format!(
                    "__intrinsic_random_int: lo ({}) must be <= hi ({})",
                    lo, hi
                ));
            }
            let value = with_rng(|rng| rng.gen_range(lo..=hi));
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value as f64)))
        },
    );

    register_typed_fn_1::<_, f64>(
        &mut module,
        "__intrinsic_random_seed",
        "Seed the thread-local RNG for reproducibility",
        "seed",
        "number",
        ConcreteType::Unit,
        |seed, _ctx| {
            let seed = seed as u64;
            with_rng(|rng| {
                *rng = ChaCha8Rng::seed_from_u64(seed);
            });
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    register_typed_fn_2::<_, f64, f64>(
        &mut module,
        "__intrinsic_random_normal",
        "Generate random number from a normal distribution (Box-Muller)",
        [("mean", "number"), ("std", "number")],
        ConcreteType::Number,
        |mean, std, _ctx| {
            if std < 0.0 {
                return Err("__intrinsic_random_normal: std must be non-negative".to_string());
            }
            let value = with_rng(|rng| {
                let u1: f64 = rng.r#gen();
                let u2: f64 = rng.r#gen();
                let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                mean + std * z
            });
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(value)))
        },
    );

    register_typed_fn_1::<_, i64>(
        &mut module,
        "__intrinsic_random_array",
        "Generate array of n random numbers in [0, 1)",
        "n",
        "int",
        ConcreteType::ArrayNumber,
        |n, _ctx| {
            if n < 0 {
                return Err("__intrinsic_random_array: n must be non-negative".to_string());
            }
            let n = n as usize;
            let values: Vec<f64> = with_rng(|rng| (0..n).map(|_| rng.r#gen::<f64>()).collect());
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(values)))
        },
    );

    module
}
