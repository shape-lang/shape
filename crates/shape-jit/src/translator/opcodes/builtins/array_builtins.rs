//! Series operations and intrinsic aggregation builtin functions for JIT compilation

use cranelift::prelude::*;

use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::BuiltinFunction;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile series builtin functions
    #[inline(always)]
    pub(super) fn compile_series_builtin(&mut self, builtin: &BuiltinFunction) -> bool {
        match builtin {
            // Intrinsic series transformations
            BuiltinFunction::IntrinsicCumsum => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.series_cumsum, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicFillna => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let fill_value = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_fillna, &[series, fill_value]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicShift => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let n = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.series_shift, &[series, n]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }

            // Intrinsic aggregation functions (return scalar from series)
            BuiltinFunction::IntrinsicSum => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.intrinsic_sum, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicMean => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.intrinsic_mean, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicMin => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.intrinsic_min, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicMax => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.intrinsic_max, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicStd => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.intrinsic_std, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicVariance => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_variance, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicMedian => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_median, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicPercentile => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let percentile = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_percentile, &[series, percentile]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicCorrelation => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_correlation, &[a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::IntrinsicCovariance => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_covariance, &[a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let nan_val = box_number(f64::NAN);
                    let val = self.builder.ins().iconst(types::I64, nan_val as i64);
                    self.stack_push(val);
                }
                true
            }

            // Rolling window intrinsics
            BuiltinFunction::IntrinsicRollingSum => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let window = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_rolling_sum, &[series, window]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicRollingMean => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let window = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_rolling_mean, &[series, window]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicRollingStd => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let window = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_rolling_std, &[series, window]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicRollingMin => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let window = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_rolling_min, &[series, window]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicRollingMax => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let window = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_rolling_max, &[series, window]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicEma => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let period = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_ema, &[series, period]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicDiff => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let periods = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_diff, &[series, periods]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else if let Some(series) = self.stack_pop() {
                    let one = self
                        .builder
                        .ins()
                        .iconst(types::I64, box_number(1.0) as i64);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_diff, &[series, one]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicPctChange => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let periods = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_pct_change, &[series, periods]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else if let Some(series) = self.stack_pop() {
                    let one = self
                        .builder
                        .ins()
                        .iconst(types::I64, box_number(1.0) as i64);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_pct_change, &[series, one]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicCumprod => {
                self.stack_pop(); // arg_count
                if let Some(series) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.series_cumprod, &[series]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicClip => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 3 {
                    let max_val = self.stack_pop().unwrap();
                    let min_val = self.stack_pop().unwrap();
                    let series = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.series_clip, &[series, min_val, max_val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            _ => false,
        }
    }
}
