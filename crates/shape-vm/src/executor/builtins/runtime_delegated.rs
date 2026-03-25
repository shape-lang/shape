//! Native intrinsic dispatch
//!
//! Handles SIMD-accelerated intrinsic functions (sum, mean, rolling, etc.)
//!
//! All intrinsics use ValueWord-native signatures (no ValueWord conversion).

use super::intrinsics;
use crate::{bytecode::BuiltinFunction, executor::VirtualMachine};
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};

impl VirtualMachine {
    /// Handle Intrinsic function builtins (37+ variants)
    pub(in crate::executor) fn handle_intrinsic_builtin(
        &mut self,
        builtin: BuiltinFunction,
        ctx: Option<&mut ExecutionContext>,
    ) -> Result<(), VMError> {
        let nb_args = self.pop_builtin_args()?;

        // Intrinsics that delegate directly to runtime (need ExecutionContext)
        match builtin {
            BuiltinFunction::IntrinsicSeries
            | BuiltinFunction::IntrinsicLinearRecurrence
            | BuiltinFunction::IntrinsicBspline2_3dBatch => {
                return self.handle_delegated_intrinsic(builtin, &nb_args, ctx);
            }
            _ => {}
        }

        // All intrinsics use ValueWord-native signatures
        let result = match builtin {
            // Math
            BuiltinFunction::IntrinsicSum => intrinsics::vm_intrinsic_sum(&nb_args),
            BuiltinFunction::IntrinsicMean => intrinsics::vm_intrinsic_mean(&nb_args),
            BuiltinFunction::IntrinsicMin => intrinsics::vm_intrinsic_min(&nb_args),
            BuiltinFunction::IntrinsicMax => intrinsics::vm_intrinsic_max(&nb_args),
            BuiltinFunction::IntrinsicStd => intrinsics::vm_intrinsic_std(&nb_args),
            BuiltinFunction::IntrinsicVariance => intrinsics::vm_intrinsic_variance(&nb_args),
            // Random
            BuiltinFunction::IntrinsicRandom => intrinsics::vm_intrinsic_random(&nb_args),
            BuiltinFunction::IntrinsicRandomInt => intrinsics::vm_intrinsic_random_int(&nb_args),
            BuiltinFunction::IntrinsicRandomSeed => intrinsics::vm_intrinsic_random_seed(&nb_args),
            BuiltinFunction::IntrinsicRandomNormal => {
                intrinsics::vm_intrinsic_random_normal(&nb_args)
            }
            BuiltinFunction::IntrinsicRandomArray => {
                intrinsics::vm_intrinsic_random_array(&nb_args)
            }
            // Series transforms
            BuiltinFunction::IntrinsicDiff => intrinsics::vm_intrinsic_diff(&nb_args),
            BuiltinFunction::IntrinsicPctChange => intrinsics::vm_intrinsic_pct_change(&nb_args),
            BuiltinFunction::IntrinsicFillna => intrinsics::vm_intrinsic_fillna(&nb_args),
            BuiltinFunction::IntrinsicCumsum => intrinsics::vm_intrinsic_cumsum(&nb_args),
            BuiltinFunction::IntrinsicCumprod => intrinsics::vm_intrinsic_cumprod(&nb_args),
            BuiltinFunction::IntrinsicClip => intrinsics::vm_intrinsic_clip(&nb_args),
            BuiltinFunction::IntrinsicShift => intrinsics::vm_intrinsic_shift(&nb_args),
            // Distribution intrinsics
            BuiltinFunction::IntrinsicDistUniform => {
                intrinsics::vm_intrinsic_dist_uniform(&nb_args)
            }
            BuiltinFunction::IntrinsicDistLognormal => {
                intrinsics::vm_intrinsic_dist_lognormal(&nb_args)
            }
            BuiltinFunction::IntrinsicDistExponential => {
                intrinsics::vm_intrinsic_dist_exponential(&nb_args)
            }
            BuiltinFunction::IntrinsicDistPoisson => {
                intrinsics::vm_intrinsic_dist_poisson(&nb_args)
            }
            BuiltinFunction::IntrinsicDistSampleN => {
                intrinsics::vm_intrinsic_dist_sample_n(&nb_args)
            }
            // Stochastic process intrinsics
            BuiltinFunction::IntrinsicBrownianMotion => {
                intrinsics::vm_intrinsic_brownian_motion(&nb_args)
            }
            BuiltinFunction::IntrinsicGbm => intrinsics::vm_intrinsic_gbm(&nb_args),
            BuiltinFunction::IntrinsicOuProcess => intrinsics::vm_intrinsic_ou_process(&nb_args),
            BuiltinFunction::IntrinsicRandomWalk => intrinsics::vm_intrinsic_random_walk(&nb_args),
            // Rolling intrinsics (SIMD)
            BuiltinFunction::IntrinsicRollingSum => intrinsics::vm_intrinsic_rolling_sum(&nb_args),
            BuiltinFunction::IntrinsicRollingMean => {
                intrinsics::vm_intrinsic_rolling_mean(&nb_args)
            }
            BuiltinFunction::IntrinsicRollingStd => intrinsics::vm_intrinsic_rolling_std(&nb_args),
            BuiltinFunction::IntrinsicRollingMin => intrinsics::vm_intrinsic_rolling_min(&nb_args),
            BuiltinFunction::IntrinsicRollingMax => intrinsics::vm_intrinsic_rolling_max(&nb_args),
            BuiltinFunction::IntrinsicEma => intrinsics::vm_intrinsic_ema(&nb_args),
            // Statistical intrinsics (SIMD)
            BuiltinFunction::IntrinsicCorrelation => intrinsics::vm_intrinsic_correlation(&nb_args),
            BuiltinFunction::IntrinsicCovariance => intrinsics::vm_intrinsic_covariance(&nb_args),
            BuiltinFunction::IntrinsicPercentile => intrinsics::vm_intrinsic_percentile(&nb_args),
            BuiltinFunction::IntrinsicMedian => intrinsics::vm_intrinsic_median(&nb_args),
            // Trigonometric intrinsics
            BuiltinFunction::IntrinsicAtan2 => intrinsics::vm_intrinsic_atan2(&nb_args),
            BuiltinFunction::IntrinsicSinh => intrinsics::vm_intrinsic_sinh(&nb_args),
            BuiltinFunction::IntrinsicCosh => intrinsics::vm_intrinsic_cosh(&nb_args),
            BuiltinFunction::IntrinsicTanh => intrinsics::vm_intrinsic_tanh(&nb_args),
            // Character code intrinsics
            BuiltinFunction::IntrinsicCharCode => intrinsics::vm_intrinsic_char_code(&nb_args),
            BuiltinFunction::IntrinsicFromCharCode => {
                intrinsics::vm_intrinsic_from_char_code(&nb_args)
            }

            _ => {
                return Err(VMError::RuntimeError(format!(
                    "Not an intrinsic: {:?}",
                    builtin
                )));
            }
        };

        self.push_vw(result?)?;
        Ok(())
    }

    /// Handle intrinsics that delegate directly to runtime
    fn handle_delegated_intrinsic(
        &mut self,
        builtin: BuiltinFunction,
        nb_args: &[ValueWord],
        ctx: Option<&mut ExecutionContext>,
    ) -> Result<(), VMError> {
        use shape_runtime::intrinsics;

        let ctx = ctx.ok_or_else(|| {
            VMError::RuntimeError("Intrinsic requires ExecutionContext".to_string())
        })?;

        let result = match builtin {
            BuiltinFunction::IntrinsicSeries => {
                return Err(VMError::NotImplemented(
                    "IntrinsicSeries (VM-only mode)".to_string(),
                ));
            }
            BuiltinFunction::IntrinsicLinearRecurrence => {
                intrinsics::recurrence::intrinsic_linear_recurrence(nb_args, ctx)
            }
            BuiltinFunction::IntrinsicBspline2_3dBatch => {
                intrinsics::math::intrinsic_bspline2_3d_batch(nb_args, ctx)
            }
            _ => unreachable!(),
        }
        .map_err(|e| VMError::RuntimeError(format!("Intrinsic failed: {}", e)))?;

        self.push_vw(result)?;
        Ok(())
    }
}
