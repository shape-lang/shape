//! Native math and stats builtin implementations
//!
//! Direct builtin methods — no string-based dispatch.

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};

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

impl VirtualMachine {
    pub(in crate::executor) fn builtin_abs(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("abs() requires 1 argument".into()));
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
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("sqrt() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.sqrt()))
    }

    pub(in crate::executor) fn builtin_floor(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("floor() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.floor()))
    }

    pub(in crate::executor) fn builtin_ceil(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("ceil() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.ceil()))
    }

    pub(in crate::executor) fn builtin_round(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("round() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.round()))
    }

    pub(in crate::executor) fn builtin_ln(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("ln() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.ln()))
    }

    pub(in crate::executor) fn builtin_exp(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("exp() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.exp()))
    }

    pub(in crate::executor) fn builtin_log(
        &mut self,
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("sin() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.sin()))
    }

    pub(in crate::executor) fn builtin_cos(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("cos() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.cos()))
    }

    pub(in crate::executor) fn builtin_tan(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("tan() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.tan()))
    }

    pub(in crate::executor) fn builtin_asin(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("asin() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.asin()))
    }

    pub(in crate::executor) fn builtin_acos(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("acos() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.acos()))
    }

    pub(in crate::executor) fn builtin_atan(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() != 1 {
            return Err(VMError::RuntimeError("atan() requires 1 argument".into()));
        }
        Ok(ValueWord::from_f64(nb_to_f64(&args[0])?.atan()))
    }

    pub(in crate::executor) fn builtin_min(
        &mut self,
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
        args: Vec<ValueWord>,
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
