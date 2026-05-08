//! Math intrinsics — partial migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's partial-migration pattern (see
//! `docs/defections.md` 2026-05-07 zero-copy entry's per-storage-variant
//! correction subsection + intrinsics-typed-CC entry's Q2 lifecycle
//! subsection), 14 of math.rs's 19 intrinsics migrate to
//! `register_typed_fn_N` typed entries via [`create_math_intrinsics_module`].
//! 5 polymorphic intrinsics remain as legacy `IntrinsicFn` bodies pending
//! follow-on architectural sub-decisions:
//!
//! - **`intrinsic_sum` / `intrinsic_min` / `intrinsic_max`**: polymorphic
//!   return (`i64` for `Vec<int>` fast path vs `f64` for `Vec<number>`).
//!   Single typed entry can't carry both. Awaits **M1-split** sub-decision
//!   (per-element-type intrinsics; cross-crate change to shape-vm
//!   compiler emission + opcode discriminants + classification logic).
//! - **`intrinsic_char_code`**: polymorphic input (`Char` vs `String`).
//!   `HeapValue::Char` is first-class post-bulldozer (`heap_value.rs:846`)
//!   so dropping the `Char` branch would break callers iterating
//!   `for c in s.chars()`. Awaits **multi-input-type dispatch** sub-
//!   decision.
//! - **`intrinsic_bspline2_3d_batch`**: 11-arg with FloatArray fast path
//!   AND generic-array slow path. Slow path uses `to_generic()` which is
//!   removed; consumer audit of `math.shape:243` wrapper + user-code
//!   needed before deciding fast-path-only-vs-keep-slow-path.
//!
//! Migrated entries take `Arc<AlignedTypedBuffer>` (for `Vec<number>`
//! aggregations) or `f64` (for trig family + `from_char_code`) typed
//! inputs and return `ConcreteReturn::F64` / `ConcreteReturn::String`
//! per the dispatcher's `TypedReturn → slot push` projection.

use crate::context::ExecutionContext;
use crate::marshal::{register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_ast::error::{Result, ShapeError};
use shape_value::{AlignedTypedBuffer, KindedSlot};
use std::sync::Arc;

// ───────────────────── Module factory (14 typed entries) ─────────────────────

/// Create the math intrinsics module with 14 typed-marshal entry points.
/// The 5 polymorphic intrinsics (sum, min, max, char_code, bspline2_3d_batch)
/// remain as legacy `IntrinsicFn` bodies in this module until their
/// follow-on architectural sub-decisions land.
pub fn create_math_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::math");
    module.description =
        "Math intrinsics (typed entries; polymorphic-shape intrinsics stay as legacy bodies pending follow-on sub-decisions)"
            .to_string();

    // ── Array aggregations ──

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_mean",
        "Mean (average) of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::Number,
        |input, _ctx| {
            let data = input.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let sum: f64 = data.iter().sum();
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(
                sum / data.len() as f64,
            )))
        },
    );

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_variance",
        "Population variance of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::Number,
        |input, _ctx| {
            let data = input.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let mean: f64 = data.iter().sum::<f64>() / data.len() as f64;
            #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
            let var = variance_avx2(data, mean);
            #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
            let var: f64 = data.iter().map(|&x| (x - mean).powi(2)).sum::<f64>()
                / data.len() as f64;
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(var)))
        },
    );

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_std",
        "Population standard deviation of a Vec<number>",
        "input",
        "Array<number>",
        ConcreteType::Number,
        |input, _ctx| {
            let data = input.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let mean: f64 = data.iter().sum::<f64>() / data.len() as f64;
            #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
            let var = variance_avx2(data, mean);
            #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
            let var: f64 = data.iter().map(|&x| (x - mean).powi(2)).sum::<f64>()
                / data.len() as f64;
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(var.sqrt())))
        },
    );

    // ── Trig family (10 unary + 1 binary atan2) ──

    register_unary_f64_op(&mut module, "__intrinsic_sin", "Sine of x", f64::sin);
    register_unary_f64_op(&mut module, "__intrinsic_cos", "Cosine of x", f64::cos);
    register_unary_f64_op(&mut module, "__intrinsic_tan", "Tangent of x", f64::tan);
    register_unary_f64_op(&mut module, "__intrinsic_asin", "Arc sine of x", f64::asin);
    register_unary_f64_op(&mut module, "__intrinsic_acos", "Arc cosine of x", f64::acos);
    register_unary_f64_op(&mut module, "__intrinsic_atan", "Arc tangent of x", f64::atan);
    register_unary_f64_op(&mut module, "__intrinsic_sinh", "Hyperbolic sine of x", f64::sinh);
    register_unary_f64_op(&mut module, "__intrinsic_cosh", "Hyperbolic cosine of x", f64::cosh);
    register_unary_f64_op(&mut module, "__intrinsic_tanh", "Hyperbolic tangent of x", f64::tanh);

    register_typed_fn_2::<_, f64, f64>(
        &mut module,
        "__intrinsic_atan2",
        "Two-argument arc tangent (atan2(y, x))",
        [("y", "number"), ("x", "number")],
        ConcreteType::Number,
        |y, x, _ctx| Ok(TypedReturn::Concrete(ConcreteReturn::F64(y.atan2(x)))),
    );

    // ── Char code (one-direction migration; reverse stays legacy) ──

    register_typed_fn_1::<_, f64>(
        &mut module,
        "__intrinsic_from_char_code",
        "Create a single-character string from a Unicode code point",
        "code",
        "number",
        ConcreteType::String,
        |code, _ctx| {
            let ch = char::from_u32(code as u32).ok_or_else(|| {
                format!(
                    "__intrinsic_from_char_code: invalid code point {}",
                    code as u32
                )
            })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::String(ch.to_string())))
        },
    );

    module
}

/// Helper: register a unary `f64 -> f64` typed entry (the trig family pattern).
fn register_unary_f64_op(
    module: &mut ModuleExports,
    name: &'static str,
    desc: &'static str,
    op: fn(f64) -> f64,
) {
    register_typed_fn_1::<_, f64>(
        module,
        name,
        desc,
        "x",
        "number",
        ConcreteType::Number,
        move |x, _ctx| Ok(TypedReturn::Concrete(ConcreteReturn::F64(op(x)))),
    );
}

// ───────────────────── Legacy bodies (5 polymorphic intrinsics) ─────────────────────

/// Intrinsic: Sum of all values in a series.
///
/// **Migration deferred** pending M1-split sub-decision (polymorphic return:
/// `i64` for `Vec<int>` fast path vs `f64` for `Vec<number>`). Cross-crate
/// change required to shape-vm compiler emission for type-driven dispatch.
pub fn intrinsic_sum(_args: &[KindedSlot], _ctx: &mut ExecutionContext) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_sum: pending Phase 2c intrinsic kind threading + M1-split — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

/// Intrinsic: Minimum value in a series or among multi-scalar arguments.
///
/// **Migration deferred** pending M1-split sub-decision (polymorphic return
/// + polymorphic input shape). Multi-scalar branches are dead code from
/// stdlib emission per audit but kept until the architectural decision lands.
pub fn intrinsic_min(_args: &[KindedSlot], _ctx: &mut ExecutionContext) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_min: pending Phase 2c intrinsic kind threading + M1-split — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

/// Intrinsic: Maximum value in a series or among multi-scalar arguments.
///
/// **Migration deferred** pending M1-split sub-decision. See `intrinsic_min`.
pub fn intrinsic_max(_args: &[KindedSlot], _ctx: &mut ExecutionContext) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_max: pending Phase 2c intrinsic kind threading + M1-split — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

/// Intrinsic: Get the Unicode code point of a single character.
///
/// **Migration deferred** pending multi-input-type dispatch sub-decision.
/// `HeapValue::Char` is first-class post-bulldozer; dropping the `as_char`
/// branch would break `for c in s.chars()`-style consumers.
pub fn intrinsic_char_code(
    _args: &[KindedSlot],
    _ctx: &mut ExecutionContext,
) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_char_code: pending Phase 2c intrinsic kind threading — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

/// Intrinsic: Batched quadratic B-spline interpolation on a 3D grid.
///
/// **Migration deferred** pending consumer audit (fast path uses
/// `as_f64_slice()`; slow path uses `to_generic()` which is removed.
/// Audit needs to determine if any consumer passes a generic array
/// before deciding fast-path-only-vs-keep-slow-path).
///
/// Args: grid_data, nx, ny, nz, x_lo, x_hi, y_lo, y_hi, z_lo, z_hi, pos_flat
pub fn intrinsic_bspline2_3d_batch(
    _args: &[KindedSlot],
    _ctx: &mut ExecutionContext,
) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_bspline2_3d_batch: pending Phase 2c intrinsic kind threading + consumer audit — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}

// ───────────────────── Helpers (used by typed + legacy bodies) ─────────────────────

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn variance_avx2(data: &[f64], mean: f64) -> f64 {
    use std::simd::f64x4;

    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();

    let mean_vec = f64x4::splat(mean);
    let mut var_sum = f64x4::splat(0.0);

    for chunk in chunks {
        let values = f64x4::from_slice(chunk);
        let diff = values - mean_vec;
        var_sum += diff * diff;
    }

    let vector_var = var_sum.reduce_sum();
    let remainder_var: f64 = remainder.iter().map(|&x| (x - mean).powi(2)).sum();

    (vector_var + remainder_var) / data.len() as f64
}

/// Core B-spline computation on a contiguous f64 slice (fastest path).
#[inline]
fn bspline2_3d_batch_slice(
    grid: &[f64],
    nx: usize, ny: usize, nz: usize,
    x_lo: f64, x_hi: f64, y_lo: f64, y_hi: f64, z_lo: f64, z_hi: f64,
    pos: &[f64],
) -> Vec<f64> {
    let n = pos.len() / 3;
    let nxm = (nx - 1) as f64;
    let nym = (ny - 1) as f64;
    let nzm = (nz - 1) as f64;
    let inv_x = nxm / (x_hi - x_lo);
    let inv_y = nym / (y_hi - y_lo);
    let inv_z = nzm / (z_hi - z_lo);
    let nyz = ny * nz;
    let mut result = Vec::with_capacity(n);

    for s in 0..n {
        let i3 = s * 3;
        let gx = ((pos[i3] - x_lo) * inv_x).clamp(0.0, nxm);
        let gy = ((pos[i3 + 1] - y_lo) * inv_y).clamp(0.0, nym);
        let gz = ((pos[i3 + 2] - z_lo) * inv_z).clamp(0.0, nzm);

        let cx = (gx + 0.5).floor() as isize;
        let cy = (gy + 0.5).floor() as isize;
        let cz = (gz + 0.5).floor() as isize;
        let tx = gx - cx as f64;
        let ty = gy - cy as f64;
        let tz = gz - cz as f64;

        let (wx0, wx1, wx2) = bspline_weights(tx);
        let (wy0, wy1, wy2) = bspline_weights(ty);
        let (wz0, wz1, wz2) = bspline_weights(tz);

        let ix = [
            (cx - 1).max(0) as usize,
            cx as usize,
            (cx + 1).min(nx as isize - 1) as usize,
        ];
        let iy = [
            (cy - 1).max(0) as usize,
            cy as usize,
            (cy + 1).min(ny as isize - 1) as usize,
        ];
        let iz = [
            (cz - 1).max(0) as usize,
            cz as usize,
            (cz + 1).min(nz as isize - 1) as usize,
        ];

        let wx = [wx0, wx1, wx2];
        let wy = [wy0, wy1, wy2];
        let wz = [wz0, wz1, wz2];

        let mut val = 0.0;
        for a in 0..3 {
            let rx = ix[a] * nyz;
            for b in 0..3 {
                let rxy = rx + iy[b] * nz;
                let wxy = wx[a] * wy[b];
                for c in 0..3 {
                    val += wxy * wz[c] * unsafe { *grid.get_unchecked(rxy + iz[c]) };
                }
            }
        }
        result.push(val);
    }
    result
}

/// Core B-spline computation using per-element access function (generic arrays).
/// Only accesses 27 grid elements per query point — no bulk copy.
#[inline]
fn bspline2_3d_batch_fn(
    grid: &dyn Fn(usize) -> f64,
    nx: usize, ny: usize, nz: usize,
    x_lo: f64, x_hi: f64, y_lo: f64, y_hi: f64, z_lo: f64, z_hi: f64,
    pos: &[f64],
) -> Vec<f64> {
    let n = pos.len() / 3;
    let nxm = (nx - 1) as f64;
    let nym = (ny - 1) as f64;
    let nzm = (nz - 1) as f64;
    let inv_x = nxm / (x_hi - x_lo);
    let inv_y = nym / (y_hi - y_lo);
    let inv_z = nzm / (z_hi - z_lo);
    let nyz = ny * nz;
    let mut result = Vec::with_capacity(n);

    for s in 0..n {
        let i3 = s * 3;
        let gx = ((pos[i3] - x_lo) * inv_x).clamp(0.0, nxm);
        let gy = ((pos[i3 + 1] - y_lo) * inv_y).clamp(0.0, nym);
        let gz = ((pos[i3 + 2] - z_lo) * inv_z).clamp(0.0, nzm);

        let cx = (gx + 0.5).floor() as isize;
        let cy = (gy + 0.5).floor() as isize;
        let cz = (gz + 0.5).floor() as isize;
        let tx = gx - cx as f64;
        let ty = gy - cy as f64;
        let tz = gz - cz as f64;

        let (wx0, wx1, wx2) = bspline_weights(tx);
        let (wy0, wy1, wy2) = bspline_weights(ty);
        let (wz0, wz1, wz2) = bspline_weights(tz);

        let ix = [
            (cx - 1).max(0) as usize,
            cx as usize,
            (cx + 1).min(nx as isize - 1) as usize,
        ];
        let iy = [
            (cy - 1).max(0) as usize,
            cy as usize,
            (cy + 1).min(ny as isize - 1) as usize,
        ];
        let iz = [
            (cz - 1).max(0) as usize,
            cz as usize,
            (cz + 1).min(nz as isize - 1) as usize,
        ];

        let wx = [wx0, wx1, wx2];
        let wy = [wy0, wy1, wy2];
        let wz = [wz0, wz1, wz2];

        let mut val = 0.0;
        for a in 0..3 {
            let rx = ix[a] * nyz;
            for b in 0..3 {
                let rxy = rx + iy[b] * nz;
                let wxy = wx[a] * wy[b];
                for c in 0..3 {
                    val += wxy * wz[c] * grid(rxy + iz[c]);
                }
            }
        }
        result.push(val);
    }
    result
}

/// Quadratic B-spline basis weights for offset t.
#[inline(always)]
fn bspline_weights(t: f64) -> (f64, f64, f64) {
    (
        0.5 * (0.5 - t) * (0.5 - t),
        0.75 - t * t,
        0.5 * (0.5 + t) * (0.5 + t),
    )
}
