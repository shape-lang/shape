//! Native math and stats builtin implementations
//!
//! Direct builtin methods — no string-based dispatch.

use crate::executor::VirtualMachine;
use shape_value::{ArgVec, VMError, ValueWord, ValueWordExt};

/// Extract a number (f64) from a ValueWord
fn nb_to_f64(nb: &ValueWord) -> Result<f64, VMError> {
    if let Some(f) = nb.as_number_coerce() {
        Ok(f)
    } else {
        Err(VMError::RuntimeError(format!(
            "expected a number, got {}",
            nb.type_name()
        )))
    }
}

// ── PC.3: broadcasted-math helpers for `arr.sqrt()` / `.ln()` / `.exp()` ────
//
// Because `sqrt`/`ln`/`exp` are NOT listed in `is_known_builtin_method`, the
// compiler lowers `arr.sqrt()` to a `BuiltinCall(Sqrt)` instead of a typed
// `CallMethod`. Without the broadcasting path below that would turn the
// array into `nb_to_f64` → "expected a number, got ptr". To keep the
// single-element `(2.5).sqrt()` path bit-exact while enabling SIMD when
// the receiver is an array, each builtin first checks for a v2 F64 typed
// array (fast path) or a legacy FloatArray (fallback), and only then
// coerces to `f64` for the scalar case.

/// Try to apply a SIMD element-wise `f64` transform if `arg` is either a v2
/// typed F64 array or a legacy FloatArray. Returns `Some(result)` on a hit,
/// `None` when the argument isn't an F64 array — the caller then takes the
/// scalar f64 path.
#[inline]
fn try_broadcast_f64_unary(
    arg: &ValueWord,
    simd_op: fn(wide::f64x4) -> wide::f64x4,
    scalar_op: fn(f64) -> f64,
) -> Option<ValueWord> {
    use crate::executor::v2_handlers::v2_array_detect as v2;

    // Fast path: v2 typed array pointer.
    if let Some(view) = v2::as_v2_typed_array(arg) {
        if let Some(ptr) = v2::unary_f64_transform(&view, simd_op, scalar_op) {
            return Some(ValueWord::from_native_ptr(ptr as usize));
        }
    }
    // Legacy Arc-backed FloatArray — preserves pre-v2 behaviour when a
    // higher-level op produced a FloatArray rather than a v2 typed array.
    if let Some(arr) = arg.as_float_array() {
        use shape_value::aligned_vec::AlignedVec;
        use std::sync::Arc;
        use wide::f64x4;
        const SIMD_THRESHOLD: usize = 16;
        let len = arr.len();
        let mut result = AlignedVec::with_capacity(len);
        if len >= SIMD_THRESHOLD {
            let chunks = len / 4;
            for i in 0..chunks {
                let idx = i * 4;
                let v = f64x4::from(&arr[idx..idx + 4]);
                let r = simd_op(v);
                for &x in r.to_array().iter() {
                    result.push(x);
                }
            }
            for i in (chunks * 4)..len {
                result.push(scalar_op(arr[i]));
            }
        } else {
            for &v in arr.iter() {
                result.push(scalar_op(v));
            }
        }
        return Some(ValueWord::from_float_array(Arc::new(result.into())));
    }
    None
}

impl VirtualMachine {
    pub(in crate::executor) fn builtin_abs(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("abs() requires 1 argument".into()));
        }
        // PC.3: `arr.abs()` lands here when UFCS rewrites the method call
        // to a builtin. Broadcast via SIMD before falling back to scalar.
        if let Some(result) = try_broadcast_f64_unary(&args[0], |v| v.abs(), f64::abs) {
            return Ok(result);
        }
        if let Some(i) = args[0].as_i64() {
            Ok(ValueWord::from_i64(i.abs()))
        } else if let Some(n) = args[0].as_number_coerce() {
            Ok(ValueWord::from_f64(n.abs()))
        } else {
            Err(VMError::RuntimeError(
                "abs() argument must be a number".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_sqrt(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("sqrt() requires 1 argument".into()));
        }
        // PC.3: broadcast over F64 arrays via SIMD; scalar path is preserved.
        if let Some(result) = try_broadcast_f64_unary(&args[0], |v| v.sqrt(), f64::sqrt) {
            return Ok(result);
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.sqrt()))
    }

    pub(in crate::executor) fn builtin_floor(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("floor() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.floor()))
    }

    pub(in crate::executor) fn builtin_ceil(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("ceil() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.ceil()))
    }

    pub(in crate::executor) fn builtin_round(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("round() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.round()))
    }

    pub(in crate::executor) fn builtin_ln(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("ln() requires 1 argument".into()));
        }
        // PC.3: broadcast over F64 arrays via SIMD.
        if let Some(result) = try_broadcast_f64_unary(&args[0], |v| v.ln(), f64::ln) {
            return Ok(result);
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.ln()))
    }

    pub(in crate::executor) fn builtin_exp(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("exp() requires 1 argument".into()));
        }
        // PC.3: broadcast over F64 arrays via SIMD.
        if let Some(result) = try_broadcast_f64_unary(&args[0], |v| v.exp(), f64::exp) {
            return Ok(result);
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.exp()))
    }

    pub(in crate::executor) fn builtin_log(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        match args.len() {
            1 => Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.log10())),
            2 => {
                let val = nb_to_f64(&args[0])?;
                let base = nb_to_f64(&args[1])?;
                Ok(ValueWord::from_f64(val.log(base)))
            }
            _ => Err(VMError::RuntimeError(
                "log() requires 1 or 2 arguments".into(),
            )),
        }
    }

    pub(in crate::executor) fn builtin_pow(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError("pow() requires 2 arguments".into()));
        }
        Ok(ValueWord::from_f64(
            nb_to_f64(&args[0])?.powf(nb_to_f64(&args[1])?),
        ))
    }

    pub(in crate::executor) fn builtin_sin(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("sin() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.sin()))
    }

    pub(in crate::executor) fn builtin_cos(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("cos() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.cos()))
    }

    pub(in crate::executor) fn builtin_tan(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("tan() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.tan()))
    }

    pub(in crate::executor) fn builtin_asin(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("asin() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.asin()))
    }

    pub(in crate::executor) fn builtin_acos(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("acos() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.acos()))
    }

    pub(in crate::executor) fn builtin_atan(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("atan() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.atan()))
    }

    pub(in crate::executor) fn builtin_min(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() == 2 {
            let a = nb_to_f64(&args[0])?;
            let b = nb_to_f64(&args[1])?;
            Ok(ValueWord::from_f64(a.min(b)))
        } else if args.len() == 1 {
            if let Some(view) = args[0].as_any_array() {
                if view.is_empty() {
                    return Ok(ValueWord::none());
                }
                let arr = view.to_generic();
                let mut result = nb_to_f64(&arr[0])?;
                for v in arr.iter().skip(1) {
                    result = result.min(nb_to_f64(v)?);
                }
                Ok(ValueWord::from_f64(result))
            } else {
                Err(VMError::RuntimeError(
                    "min() with 1 argument requires an array".into(),
                ))
            }
        } else {
            Err(VMError::RuntimeError(
                "min() requires 1 or 2 arguments".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_max(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() == 2 {
            let a = nb_to_f64(&args[0])?;
            let b = nb_to_f64(&args[1])?;
            Ok(ValueWord::from_f64(a.max(b)))
        } else if args.len() == 1 {
            if let Some(view) = args[0].as_any_array() {
                if view.is_empty() {
                    return Ok(ValueWord::none());
                }
                let arr = view.to_generic();
                let mut result = nb_to_f64(&arr[0])?;
                for v in arr.iter().skip(1) {
                    result = result.max(nb_to_f64(v)?);
                }
                Ok(ValueWord::from_f64(result))
            } else {
                Err(VMError::RuntimeError(
                    "max() with 1 argument requires an array".into(),
                ))
            }
        } else {
            Err(VMError::RuntimeError(
                "max() requires 1 or 2 arguments".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_sign(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("sign() requires 1 argument".into()));
        }
        if let Some(i) = args[0].as_i64() {
            Ok(ValueWord::from_i64(if i > 0 {
                1
            } else if i < 0 {
                -1
            } else {
                0
            }))
        } else if let Some(n) = args[0].as_number_coerce() {
            Ok(ValueWord::from_f64(if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            }))
        } else {
            Err(VMError::RuntimeError(
                "sign() argument must be a number".into(),
            ))
        }
    }

    pub(in crate::executor) fn builtin_gcd(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError("gcd() requires 2 arguments".into()));
        }
        let mut a = nb_to_f64(&args[0])? as i64;
        let mut b = nb_to_f64(&args[1])? as i64;
        a = a.abs();
        b = b.abs();
        while b != 0 {
            let t = b;
            b = a % b;
            a = t;
        }
        Ok(ValueWord::from_i64(a))
    }

    pub(in crate::executor) fn builtin_lcm(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError("lcm() requires 2 arguments".into()));
        }
        let mut a = nb_to_f64(&args[0])? as i64;
        let mut b = nb_to_f64(&args[1])? as i64;
        if a == 0 && b == 0 {
            return Ok(ValueWord::from_i64(0));
        }
        let a_abs = a.abs();
        let b_abs = b.abs();
        // gcd
        a = a_abs;
        b = b_abs;
        while b != 0 {
            let t = b;
            b = a % b;
            a = t;
        }
        let gcd = a;
        Ok(ValueWord::from_i64(a_abs / gcd * b_abs))
    }

    pub(in crate::executor) fn builtin_hypot(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 2 {
            return Err(VMError::RuntimeError("hypot() requires 2 arguments".into()));
        }
        let a = nb_to_f64(&args[0])?;
        let b = nb_to_f64(&args[1])?;
        Ok(ValueWord::from_f64(a.hypot(b)))
    }

    pub(in crate::executor) fn builtin_clamp(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 3 {
            return Err(VMError::RuntimeError(
                "clamp() requires 3 arguments (x, min, max)".into(),
            ));
        }
        let x = nb_to_f64(&args[0])?;
        let min_val = nb_to_f64(&args[1])?;
        let max_val = nb_to_f64(&args[2])?;
        Ok(ValueWord::from_f64(x.max(min_val).min(max_val)))
    }

    pub(in crate::executor) fn builtin_is_nan(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("isNaN() requires 1 argument".into()));
        }
        if let Some(n) = args[0].as_number_coerce() {
            Ok(ValueWord::from_bool(n.is_nan()))
        } else {
            // Non-numeric values are not NaN
            Ok(ValueWord::from_bool(false))
        }
    }

    pub(in crate::executor) fn builtin_is_finite(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError(
                "isFinite() requires 1 argument".into(),
            ));
        }
        if let Some(i) = args[0].as_i64() {
            // Integers are always finite
            let _ = i;
            Ok(ValueWord::from_bool(true))
        } else if let Some(n) = args[0].as_number_coerce() {
            Ok(ValueWord::from_bool(n.is_finite()))
        } else {
            Ok(ValueWord::from_bool(false))
        }
    }

    pub(in crate::executor) fn builtin_stddev(
        &mut self,
        args: ArgVec,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError(
                "stddev() requires 1 argument (array)".into(),
            ));
        }
        if let Some(view) = args[0].as_any_array() {
            if view.is_empty() {
                return Ok(ValueWord::from_f64(0.0));
            }
            let arr = view.to_generic();
            let values: Result<Vec<f64>, _> = arr.iter().map(|v| nb_to_f64(v)).collect();
            let values = values?;
            let n = values.len() as f64;
            let mean = values.iter().sum::<f64>() / n;
            let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            Ok(ValueWord::from_f64(variance.sqrt()))
        } else {
            Err(VMError::RuntimeError(
                "stddev() argument must be an array".into(),
            ))
        }
    }
}
