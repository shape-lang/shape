//! Rolling window intrinsics — partial migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's partial-migration pattern (see
//! `docs/defections.md` 2026-05-07 intrinsics-typed-CC entry's partial-
//! migration subsection), 3 of 6 rolling intrinsics migrate to
//! `register_typed_fn_N` typed entries via [`create_rolling_intrinsics_module`].
//! 3 polymorphic-input intrinsics remain as legacy `IntrinsicFn` bodies
//! pending the **M1-split sub-decision extension** (per-element-type intrinsics
//! for polymorphic-input cases; cross-crate compiler change).
//! `rolling_sum`'s i64 fast path additionally uses a validity-bitmap return
//! (`option_i64_vec_to_nb`); migrating it joins `array_transforms::diff` in
//! the validity-aware-return-variant sub-question.
//!
//! Migrated entries take `Arc<AlignedTypedBuffer>` (f64-aligned SIMD storage)
//! + `i64` (window/period scalar) and return `ConcreteReturn::ArrayF64(Vec<f64>)`
//! per the dispatcher's `TypedReturn → slot push` projection.
//!
//! These functions implement efficient O(n) algorithms for rolling window
//! operations, critical for technical indicators like SMA, Bollinger Bands, etc.
//! Uses SIMD acceleration via the simd_rolling module.

use crate::context::ExecutionContext;
use crate::marshal::register_typed_fn_2;
use crate::module_exports::ModuleExports;
use crate::simd_rolling;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_ast::error::{Result, ShapeError};
use shape_value::{AlignedTypedBuffer, KindedSlot};
use std::sync::Arc;

// ───────────────────── Module factory (3 typed entries) ─────────────────────

/// Create the rolling intrinsics module with 3 typed-marshal entry points
/// (`__intrinsic_rolling_mean`, `__intrinsic_rolling_std`, `__intrinsic_ema`).
/// The 3 polymorphic-input intrinsics (`rolling_sum`, `rolling_min`,
/// `rolling_max`) remain as legacy `IntrinsicFn` bodies in this module until
/// the M1-split sub-decision extension lands.
pub fn create_rolling_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::rolling");
    module.description =
        "Rolling-window intrinsics (typed entries; polymorphic-input intrinsics stay as legacy bodies pending M1-split sub-decision extension)"
            .to_string();

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, i64>(
        &mut module,
        "__intrinsic_rolling_mean",
        "Rolling mean (Simple Moving Average) over a fixed-size window",
        [("series", "Array<number>"), ("window", "int")],
        ConcreteType::ArrayNumber,
        |series, window, _ctx| {
            let data = series.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![])));
            }
            if window <= 0 {
                return Err("Window size must be greater than 0".to_string());
            }
            let result = simd_rolling::rolling_mean(data, window as usize);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, i64>(
        &mut module,
        "__intrinsic_rolling_std",
        "Rolling standard deviation (Welford's algorithm) over a fixed-size window",
        [("series", "Array<number>"), ("window", "int")],
        ConcreteType::ArrayNumber,
        |series, window, _ctx| {
            let data = series.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![])));
            }
            if window <= 0 {
                return Err("Window size must be greater than 0".to_string());
            }
            let window = window as usize;
            if window > data.len() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![
                    f64::NAN;
                    data.len()
                ])));
            }
            let result = simd_rolling::rolling_std_welford(data, window);
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, i64>(
        &mut module,
        "__intrinsic_ema",
        "Exponential Moving Average with smoothing alpha = 2 / (period + 1)",
        [("series", "Array<number>"), ("period", "int")],
        ConcreteType::ArrayNumber,
        |series, period, _ctx| {
            let data = series.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![])));
            }
            if period <= 0 {
                return Err("EMA period must be greater than 0".to_string());
            }
            let alpha = 2.0 / (period as f64 + 1.0);
            let mut result = Vec::with_capacity(data.len());
            let mut ema = data[0];
            result.push(ema);
            for &price in &data[1..] {
                ema = alpha * price + (1.0 - alpha) * ema;
                result.push(ema);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    module
}

// ───────────────────── Legacy bodies (3 polymorphic-input intrinsics) ─────────────────────

/// Intrinsic: Rolling sum over a window.
///
/// **Migration deferred** pending M1-split sub-decision extension
/// (polymorphic input: `Vec<int>` fast path returns `option_i64_vec_to_nb`
/// validity-bitmap; `Vec<number>` fallback returns `Vec<f64>` with NaN
/// sentinels). Joins `array_transforms::diff` in the validity-aware-return
/// sub-question.
pub fn intrinsic_rolling_sum(
    _args: &[KindedSlot],
    _ctx: &mut ExecutionContext,
) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_rolling_sum: pending Phase 2c intrinsic kind threading + M1-split — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

/// Intrinsic: Rolling minimum. See [`intrinsic_rolling_sum`] for the
/// Phase 1.B deferral rationale.
pub fn intrinsic_rolling_min(
    _args: &[KindedSlot],
    _ctx: &mut ExecutionContext,
) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_rolling_min: pending Phase 2c intrinsic kind threading + M1-split — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

/// Intrinsic: Rolling maximum. See [`intrinsic_rolling_sum`] for the
/// Phase 1.B deferral rationale.
pub fn intrinsic_rolling_max(
    _args: &[KindedSlot],
    _ctx: &mut ExecutionContext,
) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_rolling_max: pending Phase 2c intrinsic kind threading + M1-split — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}
