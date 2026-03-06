// ============================================================================
// Series/Column Builtin Functions (shift, fillna, rolling, cumulative)
// ============================================================================

use crate::nan_boxing::{
    TAG_NULL, box_column_ref, is_column_ref, is_number, unbox_column_ref, unbox_number,
};

/// Extract a &[f64] slice from column reference bits.
unsafe fn extract_column(bits: u64) -> Option<&'static [f64]> {
    if !is_column_ref(bits) {
        return None;
    }
    let (ptr, len) = unsafe { unbox_column_ref(bits) };
    if ptr.is_null() || len == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Return a new column reference from a Vec<f64>.
fn box_column_result(data: Vec<f64>) -> u64 {
    let len = data.len();
    let leaked = Box::leak(data.into_boxed_slice());
    box_column_ref(leaked.as_ptr(), len)
}

/// Shift a column by n periods, filling with NaN.
pub extern "C" fn jit_series_shift(series_bits: u64, n_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let n = if is_number(n_bits) {
                unbox_number(n_bits) as i64
            } else {
                return TAG_NULL;
            };
            let len = data.len();
            let mut result = vec![f64::NAN; len];

            if n >= 0 {
                let shift = n as usize;
                for i in shift..len {
                    result[i] = data[i - shift];
                }
            } else {
                let shift = (-n) as usize;
                for i in 0..len.saturating_sub(shift) {
                    result[i] = data[i + shift];
                }
            }

            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Fill NaN values in a column with a specified value.
pub extern "C" fn jit_series_fillna(series_bits: u64, fill_value: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let fill = if is_number(fill_value) {
                unbox_number(fill_value)
            } else {
                return TAG_NULL;
            };
            let result: Vec<f64> = data
                .iter()
                .map(|&v| if v.is_nan() { fill } else { v })
                .collect();
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Rolling mean of a column.
pub extern "C" fn jit_series_rolling_mean(series_bits: u64, window_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let window = if is_number(window_bits) {
                unbox_number(window_bits) as usize
            } else {
                return TAG_NULL;
            };
            let result = shape_runtime::simd_rolling::rolling_mean(data, window);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Rolling sum of a column.
pub extern "C" fn jit_series_rolling_sum(series_bits: u64, window_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let window = if is_number(window_bits) {
                unbox_number(window_bits) as usize
            } else {
                return TAG_NULL;
            };
            let result = shape_runtime::simd_rolling::rolling_sum(data, window);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Rolling std of a column.
pub extern "C" fn jit_series_rolling_std(series_bits: u64, window_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let window = if is_number(window_bits) {
                unbox_number(window_bits) as usize
            } else {
                return TAG_NULL;
            };
            let result = shape_runtime::simd_rolling::rolling_std(data, window);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Rolling std using Welford's algorithm (intrinsics variant).
pub extern "C" fn jit_intrinsic_rolling_std(series_bits: u64, window_bits: u64) -> u64 {
    jit_series_rolling_std(series_bits, window_bits)
}

/// Cumulative sum of a column.
pub extern "C" fn jit_series_cumsum(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let mut result = Vec::with_capacity(data.len());
            let mut acc = 0.0;
            for &v in data {
                acc += v;
                result.push(acc);
            }
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}
