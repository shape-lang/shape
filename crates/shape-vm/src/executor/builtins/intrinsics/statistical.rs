//! Statistical intrinsics over v2 numeric typed arrays.
//!
//! The public `std::core::math` wrappers reach these `KindedSlot` handlers.
//! Arrays are decoded by the shared v2 reader; scalar arguments are coerced
//! at the body boundary. This deliberately does not use the retired column
//! carrier or inspect value-box tags.

use crate::executor::builtins::{kind_coerce::coerce_to_f64, math::collect_number_series};
use shape_value::{KindedSlot, VMError};

#[inline]
fn type_error(message: impl Into<String>) -> VMError {
    VMError::RuntimeError(message.into())
}

#[inline]
fn check_arity(args: &[KindedSlot], expected: usize, name: &str) -> Result<(), VMError> {
    if args.len() != expected {
        return Err(type_error(format!(
            "{name}() requires {expected} argument{}",
            if expected == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

/// `__intrinsic_correlation(series_a, series_b)`.
///
/// Mirrors the typed runtime factory: unequal lengths are rejected, while an
/// empty input or a zero-variance series yields IEEE `NaN`.
pub(in crate::executor) fn builtin_correlation(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "correlation")?;
    let series_a = collect_number_series("correlation", &args[0])?;
    let series_b = collect_number_series("correlation", &args[1])?;
    if series_a.len() != series_b.len() {
        return Err(type_error(format!(
            "Column lengths must match: {} != {}",
            series_a.len(),
            series_b.len()
        )));
    }
    if series_a.is_empty() {
        return Ok(KindedSlot::from_number(f64::NAN));
    }
    Ok(KindedSlot::from_number(
        shape_runtime::simd_statistics::correlation(&series_a, &series_b),
    ))
}

/// `__intrinsic_covariance(series_a, series_b)`.
///
/// The runtime kernel uses the sample denominator (`n - 1`), so single-item
/// inputs retain its `NaN` behavior.
pub(in crate::executor) fn builtin_covariance(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "covariance")?;
    let series_a = collect_number_series("covariance", &args[0])?;
    let series_b = collect_number_series("covariance", &args[1])?;
    if series_a.len() != series_b.len() {
        return Err(type_error("Column lengths must match"));
    }
    if series_a.is_empty() {
        return Ok(KindedSlot::from_number(f64::NAN));
    }
    Ok(KindedSlot::from_number(
        shape_runtime::simd_statistics::covariance(&series_a, &series_b),
    ))
}

/// `__intrinsic_percentile(series, percentile)`.
///
/// This retains the runtime factory's rounded order-statistic index and its
/// quickselect ordering behavior.
pub(in crate::executor) fn builtin_percentile(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    check_arity(args, 2, "percentile")?;
    let mut series = collect_number_series("percentile", &args[0])?;
    let percentile = coerce_to_f64(&args[1])
        .ok_or_else(|| type_error("percentile() percentile must be a number"))?;
    if !(0.0..=100.0).contains(&percentile) {
        return Err(type_error("Percentile must be between 0 and 100"));
    }
    if series.is_empty() {
        return Ok(KindedSlot::from_number(f64::NAN));
    }

    let k = ((percentile / 100.0) * (series.len() - 1) as f64).round() as usize;
    Ok(KindedSlot::from_number(quickselect(&mut series, k)))
}

fn quickselect(values: &mut [f64], k: usize) -> f64 {
    if values.len() == 1 {
        return values[0];
    }

    let k = k.min(values.len() - 1);
    let mut left = 0;
    let mut right = values.len() - 1;
    loop {
        if left == right {
            return values[left];
        }

        let middle = left + (right - left) / 2;
        let pivot = median_of_three(values, left, middle, right);
        let pivot = partition(values, left, right, pivot);
        if k == pivot {
            return values[k];
        }
        if k < pivot {
            right = pivot - 1;
        } else {
            left = pivot + 1;
        }
    }
}

fn median_of_three(values: &[f64], a: usize, b: usize, c: usize) -> usize {
    if (values[a] <= values[b] && values[b] <= values[c])
        || (values[c] <= values[b] && values[b] <= values[a])
    {
        b
    } else if (values[b] <= values[a] && values[a] <= values[c])
        || (values[c] <= values[a] && values[a] <= values[b])
    {
        a
    } else {
        c
    }
}

fn partition(values: &mut [f64], left: usize, right: usize, pivot: usize) -> usize {
    let pivot_value = values[pivot];
    values.swap(pivot, right);
    let mut store = left;
    for index in left..right {
        if values[index] < pivot_value {
            values.swap(index, store);
            store += 1;
        }
    }
    values.swap(store, right);
    store
}
