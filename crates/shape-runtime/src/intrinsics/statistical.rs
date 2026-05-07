//! Statistical intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table (see
//! `docs/defections.md` 2026-05-07 intrinsics-typed-CC entry's predicted
//! error-drop calibration subsection), all 4 statistical intrinsics
//! (`correlation`, `covariance`, `percentile`, `median`) migrate to
//! `register_typed_fn_N` typed entries via [`create_statistical_intrinsics_module`].
//!
//! `percentile` and `median` mutate an owned f64 copy — the marshal layer's
//! always-clone semantics for owned-data input ensure this is safe (per
//! the let-vs-var Rust-like-lifetime analysis at `docs/defections.md`
//! 2026-05-07 zero-copy entry lines 281-291).
//!
//! Provides efficient implementations of correlation, covariance,
//! percentiles, and other statistical measures.

use crate::marshal::{register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::ModuleExports;
use crate::simd_statistics;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::AlignedTypedBuffer;
use std::sync::Arc;

// ───────────────────── Module factory (4 typed entries) ─────────────────────

/// Create the statistical intrinsics module with 4 typed-marshal entry points.
pub fn create_statistical_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::statistical");
    module.description = "Statistical intrinsics (correlation, covariance, percentile, median)"
        .to_string();

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_correlation",
        "Pearson correlation coefficient between two Vec<number>",
        [("series_a", "Array<number>"), ("series_b", "Array<number>")],
        ConcreteType::Number,
        |a, b, _ctx| {
            let data_a = a.as_slice();
            let data_b = b.as_slice();
            if data_a.len() != data_b.len() {
                return Err(format!(
                    "Column lengths must match: {} != {}",
                    data_a.len(),
                    data_b.len()
                ));
            }
            if data_a.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let result = simd_statistics::correlation(data_a, data_b);
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_covariance",
        "Covariance between two Vec<number>",
        [("series_a", "Array<number>"), ("series_b", "Array<number>")],
        ConcreteType::Number,
        |a, b, _ctx| {
            let data_a = a.as_slice();
            let data_b = b.as_slice();
            if data_a.len() != data_b.len() {
                return Err("Column lengths must match".to_string());
            }
            if data_a.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let result = simd_statistics::covariance(data_a, data_b);
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(result)))
        },
    );

    register_typed_fn_2::<_, Arc<AlignedTypedBuffer>, f64>(
        &mut module,
        "__intrinsic_percentile",
        "Percentile (0-100) of a Vec<number> via O(n) average-case quickselect",
        [("series", "Array<number>"), ("percentile", "number")],
        ConcreteType::Number,
        |series, percentile, _ctx| {
            if !(0.0..=100.0).contains(&percentile) {
                return Err("Percentile must be between 0 and 100".to_string());
            }
            let data = series.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let mut values = data.to_vec();
            let n = values.len();
            let k = ((percentile / 100.0) * (n - 1) as f64).round() as usize;
            let result = quickselect(&mut values, k);
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(result)))
        },
    );

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_median",
        "Median (50th percentile) of a Vec<number>",
        "series",
        "Array<number>",
        ConcreteType::Number,
        |series, _ctx| {
            let slice = series.as_slice();
            if slice.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::F64(f64::NAN)));
            }
            let mut data = slice.to_vec();
            data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = data.len();
            let result = if n % 2 == 0 {
                (data[n / 2 - 1] + data[n / 2]) / 2.0
            } else {
                data[n / 2]
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::F64(result)))
        },
    );

    module
}

// ───────────────────── Helpers (used by typed bodies) ─────────────────────

/// Quickselect algorithm for O(n) average case percentile calculation.
fn quickselect(arr: &mut [f64], k: usize) -> f64 {
    if arr.len() == 1 {
        return arr[0];
    }

    let k = k.min(arr.len() - 1);
    let mut left = 0;
    let mut right = arr.len() - 1;

    loop {
        if left == right {
            return arr[left];
        }

        let mid = left + (right - left) / 2;
        let pivot_idx = median_of_three(arr, left, mid, right);
        let pivot_idx = partition(arr, left, right, pivot_idx);

        if k == pivot_idx {
            return arr[k];
        } else if k < pivot_idx {
            right = pivot_idx - 1;
        } else {
            left = pivot_idx + 1;
        }
    }
}

fn median_of_three(arr: &[f64], a: usize, b: usize, c: usize) -> usize {
    if (arr[a] <= arr[b] && arr[b] <= arr[c]) || (arr[c] <= arr[b] && arr[b] <= arr[a]) {
        b
    } else if (arr[b] <= arr[a] && arr[a] <= arr[c]) || (arr[c] <= arr[a] && arr[a] <= arr[b]) {
        a
    } else {
        c
    }
}

fn partition(arr: &mut [f64], left: usize, right: usize, pivot_idx: usize) -> usize {
    let pivot_value = arr[pivot_idx];
    arr.swap(pivot_idx, right);
    let mut store_idx = left;
    for i in left..right {
        if arr[i] < pivot_value {
            arr.swap(i, store_idx);
            store_idx += 1;
        }
    }
    arr.swap(store_idx, right);
    store_idx
}
