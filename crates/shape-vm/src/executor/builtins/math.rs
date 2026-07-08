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
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};

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
    let x = coerce_to_f64(&args[0]).ok_or_else(|| type_error("abs() argument must be a number"))?;
    match args[0].kind {
        NativeKind::Int64 => Ok(KindedSlot::from_int(x.abs() as i64)),
        NativeKind::Float64 => Ok(KindedSlot::from_number(x.abs())),
        _ => unreachable!("coerce_to_f64 only succeeds on Int64|Float64"),
    }
}

pub(in crate::executor) fn builtin_sqrt(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "sqrt")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("sqrt() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.sqrt()))
}

// floor/ceil/round return a REAL `int` (i64) per the book spec
// (stdlib/native/math.mdx: `floor(x) : (number) -> int`). The result is a
// genuine integer (the value truncated/rounded to an integer), stamped
// `NativeKind::Int64` via `KindedSlot::from_int` — NOT a coerced number and
// NOT a bit-reinterpret. `int` and `number` never unify: `floor(3.7)` is the
// int `3`, usable where an int is expected (e.g. an array index).
pub(in crate::executor) fn builtin_floor(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "floor")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("floor() argument must be a number"))?;
    Ok(KindedSlot::from_int(x.floor() as i64))
}

pub(in crate::executor) fn builtin_ceil(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "ceil")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("ceil() argument must be a number"))?;
    Ok(KindedSlot::from_int(x.ceil() as i64))
}

pub(in crate::executor) fn builtin_round(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "round")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("round() argument must be a number"))?;
    Ok(KindedSlot::from_int(x.round() as i64))
}

pub(in crate::executor) fn builtin_ln(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "ln")?;
    let x = coerce_to_f64(&args[0]).ok_or_else(|| type_error("ln() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.ln()))
}

pub(in crate::executor) fn builtin_exp(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "exp")?;
    let x = coerce_to_f64(&args[0]).ok_or_else(|| type_error("exp() argument must be a number"))?;
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
            let base =
                coerce_to_f64(&args[1]).ok_or_else(|| type_error("log() base must be a number"))?;
            Ok(KindedSlot::from_number(val.log(base)))
        }
        _ => Err(type_error("log() requires 1 or 2 arguments")),
    }
}

pub(in crate::executor) fn builtin_pow(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "pow")?;
    let base = coerce_to_f64(&args[0]).ok_or_else(|| type_error("pow() base must be a number"))?;
    let exp =
        coerce_to_f64(&args[1]).ok_or_else(|| type_error("pow() exponent must be a number"))?;
    Ok(KindedSlot::from_number(base.powf(exp)))
}

pub(in crate::executor) fn builtin_sin(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "sin")?;
    let x = coerce_to_f64(&args[0]).ok_or_else(|| type_error("sin() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.sin()))
}

pub(in crate::executor) fn builtin_cos(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "cos")?;
    let x = coerce_to_f64(&args[0]).ok_or_else(|| type_error("cos() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.cos()))
}

pub(in crate::executor) fn builtin_tan(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "tan")?;
    let x = coerce_to_f64(&args[0]).ok_or_else(|| type_error("tan() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.tan()))
}

pub(in crate::executor) fn builtin_asin(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "asin")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("asin() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.asin()))
}

pub(in crate::executor) fn builtin_acos(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "acos")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("acos() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.acos()))
}

pub(in crate::executor) fn builtin_atan(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "atan")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("atan() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.atan()))
}

// Reduction selector for the variadic / series min / max fold below. `Min`
// and `Max` differ only in the comparison direction; everything else (kind
// preservation, the empty-series error, the array-vs-scalar dispatch) is
// shared.
#[derive(Clone, Copy)]
enum MinMax {
    Min,
    Max,
}

impl MinMax {
    #[inline]
    fn name(self) -> &'static str {
        match self {
            MinMax::Min => "min",
            MinMax::Max => "max",
        }
    }
    #[inline]
    fn pick_f64(self, a: f64, b: f64) -> f64 {
        match self {
            MinMax::Min => a.min(b),
            MinMax::Max => a.max(b),
        }
    }
    #[inline]
    fn pick_i64(self, a: i64, b: i64) -> i64 {
        match self {
            MinMax::Min => a.min(b),
            MinMax::Max => a.max(b),
        }
    }
}

/// Shared body for the bare-name `min` / `max` builtins.
///
/// Book spec (`stdlib/native/math.mdx`): `min(a, b, ...)` works "across
/// arguments OR a series". Two documented call shapes:
///
/// 1. **Variadic** — two or more scalar numeric arguments
///    (`min(3.0, 7.0)`, `max(1, 4, 2, 9)`). Folds left-to-right. The result
///    kind is preserved strictly: an all-`int` fold returns `int`, an
///    all-`number` fold returns `number`. Mixed `int`/`number` is rejected
///    at the type checker (it never reaches here); the runtime stays
///    defensive — if a heterogeneous slot ever arrives it coerces to `f64`
///    and returns `number`, never silently re-stamps an `int`.
///
/// 2. **Series** — exactly one `Array<T>` argument (`min([3.0, 7.0, 2.0])`).
///    The argument arrives as a `Ptr(HeapKind::TypedArray)` slot; the fold
///    reuses the same kind-generic `v2_array_detect::{min,max}_elements`
///    primitive that backs the `arr.min()` / `arr.max()` PHF methods, so a
///    series over `Array<int>` returns `int` and a series over
///    `Array<number>` returns `number` (no int/number mix, no coercion).
///
/// `int` and `number` never unify here: the only place an `int` becomes a
/// `number` is an explicit heterogeneous-scalar fold, which the strict
/// checker already forbids. No bit-reinterpret, no Bool-default.
fn min_max_reduce(op: MinMax, args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    // ── Series form: a single Array<T> argument ───────────────────────────
    if args.len() == 1 {
        if let NativeKind::Ptr(shape_value::HeapKind::TypedArray) = args[0].kind {
            let view = crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(
                args[0].slot().raw(),
                args[0].kind,
            )
            .ok_or_else(|| {
                type_error(format!(
                    "{}() series argument is not a typed array",
                    op.name()
                ))
            })?;
            let pair = match op {
                MinMax::Min => crate::executor::v2_handlers::v2_array_detect::min_elements(&view),
                MinMax::Max => crate::executor::v2_handlers::v2_array_detect::max_elements(&view),
            };
            return match pair {
                Some((bits, kind)) => Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind)),
                None => Err(type_error(format!(
                    "{}() of an empty or non-numeric series",
                    op.name()
                ))),
            };
        }
        // A single scalar argument has no min/max to compute.
        return Err(type_error(format!(
            "{}() requires at least 2 arguments or a single numeric array",
            op.name()
        )));
    }

    // ── Variadic form: two or more scalar numeric arguments ────────────────
    if args.is_empty() {
        return Err(type_error(format!(
            "{}() requires at least 2 arguments or a single numeric array",
            op.name()
        )));
    }

    // All-`int` fold preserves `int`; any non-Int64 slot drops to the f64
    // fold and yields `number`.
    let all_int = args.iter().all(|a| a.kind == NativeKind::Int64);
    if all_int {
        let mut acc = args[0].as_i64().expect("kind=Int64");
        for a in &args[1..] {
            acc = op.pick_i64(acc, a.as_i64().expect("kind=Int64"));
        }
        return Ok(KindedSlot::from_int(acc));
    }

    let mut acc = coerce_to_f64(&args[0])
        .ok_or_else(|| type_error(format!("{}() argument must be a number", op.name())))?;
    for a in &args[1..] {
        let v = coerce_to_f64(a)
            .ok_or_else(|| type_error(format!("{}() argument must be a number", op.name())))?;
        acc = op.pick_f64(acc, v);
    }
    Ok(KindedSlot::from_number(acc))
}

pub(in crate::executor) fn builtin_min(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    min_max_reduce(MinMax::Min, args)
}

pub(in crate::executor) fn builtin_max(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    min_max_reduce(MinMax::Max, args)
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
    let a = coerce_to_f64(&args[0]).ok_or_else(|| type_error("lcm() argument must be a number"))?
        as i64;
    let b = coerce_to_f64(&args[1]).ok_or_else(|| type_error("lcm() argument must be a number"))?
        as i64;
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
    let a =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("hypot() argument must be a number"))?;
    let b =
        coerce_to_f64(&args[1]).ok_or_else(|| type_error("hypot() argument must be a number"))?;
    Ok(KindedSlot::from_number(a.hypot(b)))
}

pub(in crate::executor) fn builtin_clamp(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 3, "clamp")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("clamp() argument must be a number"))?;
    let min_val =
        coerce_to_f64(&args[1]).ok_or_else(|| type_error("clamp() min must be a number"))?;
    let max_val =
        coerce_to_f64(&args[2]).ok_or_else(|| type_error("clamp() max must be a number"))?;
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

// ── Array-statistics helpers (MA1: sum/mean/std/variance documented at
// stdlib/core/math) ────────────────────────────────────────────────────────
//
// The aggregate-statistics intrinsics (`__intrinsic_mean` / `__intrinsic_std`
// / `__intrinsic_variance`) take a `Vec<number>` / `Vec<int>` typed array as
// their single argument. The argument arrives as a single `KindedSlot` with
// kind `Ptr(HeapKind::TypedArray)`; we read its elements through the
// kind-generic v2 typed-array view (`as_v2_typed_array` + `read_element`),
// decoding each numeric element to `f64`. No `ValueWord`, no tag decode —
// the receiver kind is the single discriminator (ADR-005 §1 / ADR-006 §2.7.6).

use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, read_element};

/// Collect a numeric (`Vec<number>` / `Vec<int>`) typed-array argument into a
/// `Vec<f64>`. Each element pair `(bits, kind)` from the v2 typed-array view is
/// decoded to `f64` via a fresh `KindedSlot` + `coerce_to_f64`. Returns a
/// runtime error if the argument is not a numeric typed array, or if any
/// element is non-numeric.
pub(in crate::executor) fn collect_number_series(
    name: &str,
    slot: &KindedSlot,
) -> Result<Vec<f64>, VMError> {
    if slot.kind != NativeKind::Ptr(shape_value::HeapKind::TypedArray) {
        return Err(type_error(format!("{name}() argument must be an array")));
    }
    let view = as_v2_typed_array(slot.slot.raw(), slot.kind).ok_or_else(|| {
        type_error(format!(
            "{name}(): argument bits failed v2 TypedArray detection (kind {:?})",
            slot.kind
        ))
    })?;
    let mut out: Vec<f64> = Vec::with_capacity(view.len as usize);
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i)
            .ok_or_else(|| type_error(format!("{name}(): failed to read array element {i}")))?;
        let elem = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let v = coerce_to_f64(&elem).ok_or_else(|| {
            type_error(format!(
                "{name}(): array element {i} is not a number (kind {:?})",
                kind
            ))
        })?;
        out.push(v);
    }
    Ok(out)
}

/// Population variance of `data`. Empty input ⇒ NaN (mirrors the documented
/// intrinsic contract). Numerically straightforward two-pass formula.
#[inline]
fn population_variance(data: &[f64]) -> f64 {
    if data.is_empty() {
        return f64::NAN;
    }
    let mean = data.iter().sum::<f64>() / data.len() as f64;
    data.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / data.len() as f64
}

/// `__intrinsic_mean(series)` — arithmetic mean of a numeric typed array.
/// Returns a `number` (`Float64`); empty input ⇒ NaN.
pub(in crate::executor) fn builtin_mean(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "mean")?;
    let data = collect_number_series("mean", &args[0])?;
    if data.is_empty() {
        return Ok(KindedSlot::from_number(f64::NAN));
    }
    Ok(KindedSlot::from_number(
        data.iter().sum::<f64>() / data.len() as f64,
    ))
}

/// `__intrinsic_median(series)` — median (50th percentile) of a numeric typed
/// array. Returns a `number` (`Float64`); empty input ⇒ NaN. Mirrors the
/// runtime typed body at `intrinsics/statistical.rs`: sort a copy, then take
/// the midpoint (average of the two central elements for even length).
pub(in crate::executor) fn builtin_median(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "median")?;
    let mut data = collect_number_series("median", &args[0])?;
    if data.is_empty() {
        return Ok(KindedSlot::from_number(f64::NAN));
    }
    data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = data.len();
    let result = if n % 2 == 0 {
        (data[n / 2 - 1] + data[n / 2]) / 2.0
    } else {
        data[n / 2]
    };
    Ok(KindedSlot::from_number(result))
}

/// `__intrinsic_variance(series)` — population variance of a numeric typed
/// array. Returns a `number` (`Float64`); empty input ⇒ NaN.
pub(in crate::executor) fn builtin_variance(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "variance")?;
    let data = collect_number_series("variance", &args[0])?;
    Ok(KindedSlot::from_number(population_variance(&data)))
}

/// `__intrinsic_std(series)` — population standard deviation of a numeric
/// typed array: `sqrt(variance)`. Returns a `number` (`Float64`); empty
/// input ⇒ NaN.
pub(in crate::executor) fn builtin_std(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "std")?;
    let data = collect_number_series("std", &args[0])?;
    Ok(KindedSlot::from_number(population_variance(&data).sqrt()))
}

// ── Scalar trig intrinsics (MA1: atan2/sinh/cosh/tanh) ──────────────────────

/// `__intrinsic_atan2(y, x)` — two-argument arc tangent (libm semantics).
pub(in crate::executor) fn builtin_atan2(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "atan2")?;
    let y =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("atan2() argument must be a number"))?;
    let x =
        coerce_to_f64(&args[1]).ok_or_else(|| type_error("atan2() argument must be a number"))?;
    Ok(KindedSlot::from_number(y.atan2(x)))
}

/// `__intrinsic_sinh(x)` — hyperbolic sine.
pub(in crate::executor) fn builtin_sinh(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "sinh")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("sinh() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.sinh()))
}

/// `__intrinsic_cosh(x)` — hyperbolic cosine.
pub(in crate::executor) fn builtin_cosh(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "cosh")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("cosh() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.cosh()))
}

/// `__intrinsic_tanh(x)` — hyperbolic tangent.
pub(in crate::executor) fn builtin_tanh(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "tanh")?;
    let x =
        coerce_to_f64(&args[0]).ok_or_else(|| type_error("tanh() argument must be a number"))?;
    Ok(KindedSlot::from_number(x.tanh()))
}

/// `stddev(arr)` — population standard deviation over a `Vec<number>` /
/// `Vec<int>` typed array. Bare-builtin alias for the `std` aggregate;
/// shares the v2 typed-array reader.
pub(in crate::executor) fn builtin_stddev(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 1, "stddev")?;
    let data = collect_number_series("stddev", &args[0])?;
    Ok(KindedSlot::from_number(population_variance(&data).sqrt()))
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
        let r = builtin_min(&[KindedSlot::from_int(3), KindedSlot::from_int(7)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(3));
    }

    #[test]
    fn min_int_float_widens() {
        let r = builtin_min(&[KindedSlot::from_int(3), KindedSlot::from_number(1.5)]).unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(1.5));
    }

    // MA3: variadic form — `min`/`max` fold "across arguments" per the book.
    // All-int variadic preserves int; all-number stays number. (Series-array
    // form is covered end-to-end by the bytecode integration suite — it needs
    // a live heap-allocated TypedArray that the scalar-only unit harness here
    // cannot construct.)
    #[test]
    fn max_variadic_all_int_stays_int() {
        let r = builtin_max(&[
            KindedSlot::from_int(3),
            KindedSlot::from_int(7),
            KindedSlot::from_int(2),
            KindedSlot::from_int(9),
            KindedSlot::from_int(1),
        ])
        .unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(9));
    }

    #[test]
    fn min_variadic_all_int_stays_int() {
        let r = builtin_min(&[
            KindedSlot::from_int(3),
            KindedSlot::from_int(7),
            KindedSlot::from_int(2),
        ])
        .unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(2));
    }

    #[test]
    fn max_variadic_all_number_stays_number() {
        let r = builtin_max(&[
            KindedSlot::from_number(3.0),
            KindedSlot::from_number(7.0),
            KindedSlot::from_number(2.0),
            KindedSlot::from_number(9.0),
            KindedSlot::from_number(1.0),
        ])
        .unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(9.0));
    }

    #[test]
    fn min_two_arg_form_preserved() {
        let r = builtin_max(&[KindedSlot::from_number(3.0), KindedSlot::from_number(7.0)]).unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(7.0));
    }

    #[test]
    fn min_single_scalar_is_error() {
        // A lone scalar (no array) has no min to compute.
        assert!(builtin_min(&[KindedSlot::from_number(5.0)]).is_err());
        assert!(builtin_max(&[KindedSlot::from_int(5)]).is_err());
    }

    // floor/ceil/round return a REAL int (book spec: (number) -> int).
    // The result carries NativeKind::Int64 — a genuine integer, not a coerced
    // number and not a bit-reinterpret of the f64.
    #[test]
    fn floor_returns_int64() {
        let r = builtin_floor(&[KindedSlot::from_number(3.7)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(3));
    }

    #[test]
    fn ceil_returns_int64() {
        let r = builtin_ceil(&[KindedSlot::from_number(3.2)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(4));
    }

    #[test]
    fn round_returns_int64_half_away_from_zero() {
        let r = builtin_round(&[KindedSlot::from_number(2.5)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(3));
        let neg = builtin_round(&[KindedSlot::from_number(-2.5)]).unwrap();
        assert_eq!(neg.kind, NativeKind::Int64);
        assert_eq!(neg.as_i64(), Some(-3));
    }

    #[test]
    fn floor_negative_returns_int64() {
        let r = builtin_floor(&[KindedSlot::from_number(-3.2)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(-4));
    }

    #[test]
    fn sign_int_preserves_int() {
        let r = builtin_sign(&[KindedSlot::from_int(-42)]).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(-1));
    }

    #[test]
    fn gcd_basic() {
        let r = builtin_gcd(&[KindedSlot::from_int(12), KindedSlot::from_int(18)]).unwrap();
        assert_eq!(r.as_i64(), Some(6));
    }

    #[test]
    fn lcm_basic() {
        let r = builtin_lcm(&[KindedSlot::from_int(4), KindedSlot::from_int(6)]).unwrap();
        assert_eq!(r.as_i64(), Some(12));
    }

    #[test]
    fn hypot_basic() {
        let r =
            builtin_hypot(&[KindedSlot::from_number(3.0), KindedSlot::from_number(4.0)]).unwrap();
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
