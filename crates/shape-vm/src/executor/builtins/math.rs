//! Native math builtin implementations (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5b body migration: every fn here takes `&[KindedSlot]` and returns
//! `Result<KindedSlot, VMError>`. Heterogeneous-numeric inputs (Int|Float)
//! coerce via the body-side helper `kind_coerce::coerce_to_f64`; per
//! §2.7.6 the carrier `KindedSlot` does NOT expose a cross-kind
//! `as_number_coerce` accessor.
//!
//! Method-form broadcasts (`arr.sqrt()`, `arr.abs()`, etc.) dispatch
//! through the per-type PHF method registries (`ARRAY_METHODS`,
//! etc.) and never reach this file. This file handles only the scalar
//! builtin call form (`abs(x)`, `sqrt(x)`, ...).

use super::kind_coerce::coerce_to_f64;
use shape_value::{KindedSlot, NativeKind, VMError};

/// Construct a runtime type-error `VMError` with a builtin-specific message.
#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Common arity check helper — returns a runtime error if `args.len() != n`.
#[inline]
fn check_arity(args: &[KindedSlot], n: usize, name: &str) -> Result<(), VMError> {
    if args.len() != n {
        return Err(type_error(format!(
            "{}() requires {} argument{}",
            name,
            n,
            if n == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

// ── Heterogeneous-numeric monadic helpers ──────────────────────────────────
//
// These accept either Int or Float and dispatch on input kind to preserve
// kind on output where it makes sense. Per §2.7.6's heterogeneous-kind body
// pattern, we coerce to f64 at the body site, then re-narrow on return.

pub(in crate::executor) fn builtin_abs(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "abs")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("abs() argument must be a number"))?;
    match args[0].kind {
        NativeKind::Int64 => Ok(KindedSlot::from_int(x.abs() as i64)),
        NativeKind::Float64 => Ok(KindedSlot::from_number(x.abs())),
        _ => unreachable!("coerce_to_f64 only succeeds on Int64|Float64"),
    }
}

pub(in crate::executor) fn builtin_sqrt(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "sqrt")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("sqrt() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.sqrt()))
}

pub(in crate::executor) fn builtin_floor(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "floor")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("floor() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.floor()))
}

pub(in crate::executor) fn builtin_ceil(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "ceil")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("ceil() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.ceil()))
}

pub(in crate::executor) fn builtin_round(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "round")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("round() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.round()))
}

pub(in crate::executor) fn builtin_ln(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "ln")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("ln() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.ln()))
}

pub(in crate::executor) fn builtin_exp(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "exp")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("exp() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.exp()))
}

pub(in crate::executor) fn builtin_log(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    match args.len() {
        1 => {
            let x = coerce_to_f64(&args[0])
                .ok_or_else(|| type_error("log() argument must be a number"))?;
            Ok(KindedSlot::from_number(x.log10()))
        }
        2 => {
            let val = coerce_to_f64(&args[0])
                .ok_or_else(|| type_error("log() argument must be a number"))?;
            let base = coerce_to_f64(&args[1])
                .ok_or_else(|| type_error("log() base must be a number"))?;
            Ok(KindedSlot::from_number(val.log(base)))
        }
        _ => Err(type_error("log() requires 1 or 2 arguments")),
    }
}

pub(in crate::executor) fn builtin_pow(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "pow")?;
    let base = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("pow() base must be a number"))?;
    let exp = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("pow() exponent must be a number"))?;
    Ok(KindedSlot::from_number(base.powf(exp)))
}

pub(in crate::executor) fn builtin_sin(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "sin")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("sin() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.sin()))
}

pub(in crate::executor) fn builtin_cos(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "cos")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("cos() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.cos()))
}

pub(in crate::executor) fn builtin_tan(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "tan")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("tan() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.tan()))
}

pub(in crate::executor) fn builtin_asin(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "asin")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("asin() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.asin()))
}

pub(in crate::executor) fn builtin_acos(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "acos")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("acos() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.acos()))
}

pub(in crate::executor) fn builtin_atan(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "atan")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("atan() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.atan()))
}

pub(in crate::executor) fn builtin_min(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    // Two-argument scalar form. Single-argument array form drops here in
    // Wave 5b; array reduction path lives in `Array::min()` PHF method
    // dispatch, not the bare-name builtin. (CLAUDE.md: array methods are
    // dispatch-only; bare-name aliases removed.)
    if args.len() != 2 {
        return Err(type_error("min() requires 2 arguments"));
    }
    let a = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("min() argument must be a number"))?;
    let b = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("min() argument must be a number"))?;
    // Preserve Int kind when both inputs are Int.
    match (args[0].kind, args[1].kind) {
        (NativeKind::Int64, NativeKind::Int64) => {
            let ai = args[0].as_i64().expect("kind=Int64");
            let bi = args[1].as_i64().expect("kind=Int64");
            Ok(KindedSlot::from_int(ai.min(bi)))
        }
        _ => Ok(KindedSlot::from_number(a.min(b))),
    }
}

pub(in crate::executor) fn builtin_max(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("max() requires 2 arguments"));
    }
    let a = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("max() argument must be a number"))?;
    let b = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("max() argument must be a number"))?;
    match (args[0].kind, args[1].kind) {
        (NativeKind::Int64, NativeKind::Int64) => {
            let ai = args[0].as_i64().expect("kind=Int64");
            let bi = args[1].as_i64().expect("kind=Int64");
            Ok(KindedSlot::from_int(ai.max(bi)))
        }
        _ => Ok(KindedSlot::from_number(a.max(b))),
    }
}

pub(in crate::executor) fn builtin_sign(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "sign")?;
    match args[0].kind {
        NativeKind::Int64 => {
            let i = args[0].as_i64().expect("kind=Int64");
            Ok(KindedSlot::from_int(i.signum()))
        }
        NativeKind::Float64 => {
            let n = args[0].as_f64().expect("kind=Float64");
            let s = if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            };
            Ok(KindedSlot::from_number(s))
        }
        _ => Err(type_error("sign() argument must be a number")),
    }
}

pub(in crate::executor) fn builtin_gcd(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "gcd")?;
    let mut a = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("gcd() argument must be a number"))? as i64;
    let mut b = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("gcd() argument must be a number"))? as i64;
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    Ok(KindedSlot::from_int(a))
}

pub(in crate::executor) fn builtin_lcm(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "lcm")?;
    let a = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("lcm() argument must be a number"))? as i64;
    let b = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("lcm() argument must be a number"))? as i64;
    if a == 0 && b == 0 {
        return Ok(KindedSlot::from_int(0));
    }
    let a_abs = a.abs();
    let b_abs = b.abs();
    let mut x = a_abs;
    let mut y = b_abs;
    while y != 0 {
        let t = y;
        y = x % y;
        x = t;
    }
    let gcd = x;
    Ok(KindedSlot::from_int(a_abs / gcd * b_abs))
}

pub(in crate::executor) fn builtin_hypot(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "hypot")?;
    let a = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("hypot() argument must be a number"))?;
    let b = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("hypot() argument must be a number"))?;
    Ok(KindedSlot::from_number(a.hypot(b)))
}

pub(in crate::executor) fn builtin_clamp(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 3, "clamp")?;
    let x = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error("clamp() argument must be a number"))?;
    let min_val = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("clamp() min must be a number"))?;
    let max_val = coerce_to_f64(&args[2])
        .ok_or_else(|| type_error("clamp() max must be a number"))?;
    Ok(KindedSlot::from_number(x.max(min_val).min(max_val)))
}

pub(in crate::executor) fn builtin_is_nan(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "isNaN")?;
    let result = match args[0].kind {
        NativeKind::Float64 => args[0].as_f64().expect("kind=Float64").is_nan(),
        // Integers are never NaN; non-numeric values are not NaN.
        _ => false,
    };
    Ok(KindedSlot::from_bool(result))
}

pub(in crate::executor) fn builtin_is_finite(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "isFinite")?;
    let result = match args[0].kind {
        NativeKind::Int64 => true,
        NativeKind::Float64 => args[0].as_f64().expect("kind=Float64").is_finite(),
        _ => false,
    };
    Ok(KindedSlot::from_bool(result))
}

/// `stddev(arr)` — population standard deviation over a `Vec<number>` /
/// `Vec<int>` typed array. Single-array form. Per ADR-005 §1, heap dispatch
/// routes through `slot.as_heap_value()` + `HeapValue` match.
pub(in crate::executor) fn builtin_stddev(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "stddev")?;
    match args[0].kind {
        NativeKind::Ptr(shape_value::HeapKind::TypedArray) => {
            // V3-S5 ckpt-5: TypedArrayData numeric arms (F64/I64/I32/F32)
            // deleted at ckpt-1..ckpt-4 per W12 audit §3.5. Rebuild at
            // ckpt-6 STRICT close per per-T v2-raw `TypedArray<T>`
            // direct-access target. Refusal #1.
            Err(VMError::NotImplemented(
                "stddev: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3. \
                 `Arc<TypedArrayData>` numeric-arm dispatch DELETED at \
                 ckpt-1..ckpt-4. Rebuild at ckpt-6 STRICT close per v2-raw \
                 `TypedArray<T>` direct-access. Refusal #1."
                    .to_string(),
            ))
        }
        _ => Err(type_error("stddev() argument must be an array")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abs_int_preserves_int_kind() {
        let s = KindedSlot::from_int(-7);
        let r = builtin_abs(&[s]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(7));
    }

    #[test]
    fn abs_float_preserves_float_kind() {
        let s = KindedSlot::from_number(-2.5);
        let r = builtin_abs(&[s]).unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(2.5));
    }

    #[test]
    fn sqrt_int_widens_to_float() {
        let s = KindedSlot::from_int(9);
        let r = builtin_sqrt(&[s]).unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(3.0));
    }

    #[test]
    fn pow_basic() {
        let r = builtin_pow(&[KindedSlot::from_int(2), KindedSlot::from_int(8)]).unwrap();
        assert_eq!(r.as_f64(), Some(256.0));
    }

    #[test]
    fn min_two_ints_stays_int() {
        let r =
            builtin_min(&[KindedSlot::from_int(3), KindedSlot::from_int(7)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(3));
    }

    #[test]
    fn min_int_float_widens() {
        let r =
            builtin_min(&[KindedSlot::from_int(3), KindedSlot::from_number(1.5)]).unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(1.5));
    }

    #[test]
    fn sign_int_preserves_int() {
        let r = builtin_sign(&[KindedSlot::from_int(-42)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(-1));
    }

    #[test]
    fn gcd_basic() {
        let r =
            builtin_gcd(&[KindedSlot::from_int(12), KindedSlot::from_int(18)]).unwrap();
        assert_eq!(r.as_i64(), Some(6));
    }

    #[test]
    fn lcm_basic() {
        let r =
            builtin_lcm(&[KindedSlot::from_int(4), KindedSlot::from_int(6)]).unwrap();
        assert_eq!(r.as_i64(), Some(12));
    }

    #[test]
    fn hypot_basic() {
        let r = builtin_hypot(&[KindedSlot::from_number(3.0), KindedSlot::from_number(4.0)])
            .unwrap();
        assert_eq!(r.as_f64(), Some(5.0));
    }

    #[test]
    fn clamp_basic() {
        let r = builtin_clamp(&[
            KindedSlot::from_number(15.0),
            KindedSlot::from_number(0.0),
            KindedSlot::from_number(10.0),
        ])
        .unwrap();
        assert_eq!(r.as_f64(), Some(10.0));
    }

    #[test]
    fn is_nan_int_false() {
        let r = builtin_is_nan(&[KindedSlot::from_int(0)]).unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn is_nan_float_nan_true() {
        let r = builtin_is_nan(&[KindedSlot::from_number(f64::NAN)]).unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn is_finite_int_true() {
        let r = builtin_is_finite(&[KindedSlot::from_int(42)]).unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn is_finite_inf_false() {
        let r = builtin_is_finite(&[KindedSlot::from_number(f64::INFINITY)]).unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn arity_check_fires() {
        let err = builtin_sqrt(&[]).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("sqrt")),
            other => panic!("expected RuntimeError, got {:?}", other),
        }
    }

    #[test]
    fn type_check_fires() {
        let err = builtin_sqrt(&[KindedSlot::from_bool(true)]).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("number")),
            other => panic!("expected RuntimeError, got {:?}", other),
        }
    }
}
