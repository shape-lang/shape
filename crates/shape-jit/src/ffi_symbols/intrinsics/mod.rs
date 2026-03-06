// ============================================================================
// Intrinsic Aggregation Functions (operate on column references)
// ============================================================================

use crate::context::JITContext;
use crate::jit_array::JitArray;
use crate::nan_boxing::*;

/// Extract a &[f64] slice from column reference bits.
/// Returns None if not a valid column reference.
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

/// Return the result of a column operation as a new boxed column reference.
fn box_column_result(data: Vec<f64>) -> u64 {
    let len = data.len();
    let leaked = Box::leak(data.into_boxed_slice());
    box_column_ref(leaked.as_ptr(), len)
}

/// Intrinsic sum: compute sum of all values in a column.
pub extern "C" fn jit_intrinsic_sum(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            box_number(shape_runtime::columnar_aggregations::sum_f64_slice(data))
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic mean: compute mean of all values in a column.
pub extern "C" fn jit_intrinsic_mean(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            box_number(shape_runtime::columnar_aggregations::mean_f64_slice(data))
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic min: compute minimum of all values in a column.
pub extern "C" fn jit_intrinsic_min(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            box_number(shape_runtime::columnar_aggregations::min_f64_slice(data))
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic max: compute maximum of all values in a column.
pub extern "C" fn jit_intrinsic_max(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            box_number(shape_runtime::columnar_aggregations::max_f64_slice(data))
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic variance: compute variance of all values in a column.
pub extern "C" fn jit_intrinsic_variance(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let mean = shape_runtime::columnar_aggregations::mean_f64_slice(data);
            let n = data.len() as f64;
            if n < 2.0 {
                return box_number(f64::NAN);
            }
            let sum_sq: f64 = data.iter().map(|&x| (x - mean) * (x - mean)).sum();
            box_number(sum_sq / (n - 1.0))
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic std: compute standard deviation of all values in a column.
pub extern "C" fn jit_intrinsic_std(series_bits: u64) -> u64 {
    let var_bits = jit_intrinsic_variance(series_bits);
    if is_number(var_bits) {
        let var = unbox_number(var_bits);
        box_number(var.sqrt())
    } else {
        box_number(f64::NAN)
    }
}

/// Intrinsic median: compute median of all values in a column.
pub extern "C" fn jit_intrinsic_median(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let mut sorted: Vec<f64> = data.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = sorted.len();
            if n == 0 {
                return box_number(f64::NAN);
            }
            let median = if n % 2 == 0 {
                (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
            } else {
                sorted[n / 2]
            };
            box_number(median)
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic percentile: compute percentile of column values.
pub extern "C" fn jit_intrinsic_percentile(series_bits: u64, percentile_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let pct = if is_number(percentile_bits) {
                unbox_number(percentile_bits)
            } else {
                return box_number(f64::NAN);
            };

            let mut sorted: Vec<f64> = data.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = sorted.len();
            if n == 0 {
                return box_number(f64::NAN);
            }
            let idx = (pct / 100.0 * (n - 1) as f64).round() as usize;
            let idx = idx.min(n - 1);
            box_number(sorted[idx])
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic correlation: compute Pearson correlation between two columns.
pub extern "C" fn jit_intrinsic_correlation(a_bits: u64, b_bits: u64) -> u64 {
    unsafe {
        if let (Some(a), Some(b)) = (extract_column(a_bits), extract_column(b_bits)) {
            if a.len() != b.len() || a.is_empty() {
                return box_number(f64::NAN);
            }
            box_number(shape_runtime::simd_statistics::correlation(a, b))
        } else {
            box_number(f64::NAN)
        }
    }
}

/// Intrinsic covariance: compute sample covariance between two columns.
pub extern "C" fn jit_intrinsic_covariance(a_bits: u64, b_bits: u64) -> u64 {
    unsafe {
        if let (Some(a), Some(b)) = (extract_column(a_bits), extract_column(b_bits)) {
            if a.len() != b.len() || a.is_empty() {
                return box_number(f64::NAN);
            }
            box_number(shape_runtime::simd_statistics::covariance(a, b))
        } else {
            box_number(f64::NAN)
        }
    }
}

// ============================================================================
// Rolling/Transform Intrinsics (return column references)
// ============================================================================

/// Rolling min of a column.
pub extern "C" fn jit_series_rolling_min(series_bits: u64, window_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let window = if is_number(window_bits) {
                unbox_number(window_bits) as usize
            } else {
                return TAG_NULL;
            };
            let result = shape_runtime::simd_rolling::rolling_min_deque(data, window);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Rolling max of a column.
pub extern "C" fn jit_series_rolling_max(series_bits: u64, window_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let window = if is_number(window_bits) {
                unbox_number(window_bits) as usize
            } else {
                return TAG_NULL;
            };
            let result = shape_runtime::simd_rolling::rolling_max_deque(data, window);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// EMA (exponential moving average) of a column.
pub extern "C" fn jit_series_ema(series_bits: u64, period_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let period = if is_number(period_bits) {
                unbox_number(period_bits) as usize
            } else {
                return TAG_NULL;
            };
            if period == 0 || data.is_empty() {
                return TAG_NULL;
            }
            let alpha = 2.0 / (period as f64 + 1.0);
            let mut result = Vec::with_capacity(data.len());
            result.push(data[0]);
            for i in 1..data.len() {
                let ema = alpha * data[i] + (1.0 - alpha) * result[i - 1];
                result.push(ema);
            }
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Diff (first difference) of a column.
pub extern "C" fn jit_series_diff(series_bits: u64, _periods_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let result = shape_runtime::simd_rolling::diff(data);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Percent change of a column.
pub extern "C" fn jit_series_pct_change(series_bits: u64, _periods_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let result = shape_runtime::simd_rolling::pct_change(data);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Cumulative product of a column.
pub extern "C" fn jit_series_cumprod(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let mut result = Vec::with_capacity(data.len());
            let mut acc = 1.0;
            for &v in data {
                acc *= v;
                result.push(acc);
            }
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Clip values in a column to [min, max] range.
pub extern "C" fn jit_series_clip(series_bits: u64, min_bits: u64, max_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            let min_val = if is_number(min_bits) {
                unbox_number(min_bits)
            } else {
                return TAG_NULL;
            };
            let max_val = if is_number(max_bits) {
                unbox_number(max_bits)
            } else {
                return TAG_NULL;
            };
            let mut result = data.to_vec();
            shape_runtime::simd_rolling::clip(&mut result, min_val, max_val);
            box_column_result(result)
        } else {
            TAG_NULL
        }
    }
}

/// Broadcast a scalar value to a column of given length.
pub extern "C" fn jit_series_broadcast(value_bits: u64, len_bits: u64) -> u64 {
    let value = if is_number(value_bits) {
        unbox_number(value_bits)
    } else {
        return TAG_NULL;
    };
    let len = if is_number(len_bits) {
        unbox_number(len_bits) as usize
    } else {
        return TAG_NULL;
    };
    let result = vec![value; len];
    box_column_result(result)
}

/// Get index of highest value in a column.
pub extern "C" fn jit_series_highest_index(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            if data.is_empty() {
                return TAG_NULL;
            }
            let mut max_idx = 0;
            let mut max_val = f64::NEG_INFINITY;
            for (i, &v) in data.iter().enumerate() {
                if v > max_val {
                    max_val = v;
                    max_idx = i;
                }
            }
            box_number(max_idx as f64)
        } else {
            TAG_NULL
        }
    }
}

/// Get index of lowest value in a column.
pub extern "C" fn jit_series_lowest_index(series_bits: u64) -> u64 {
    unsafe {
        if let Some(data) = extract_column(series_bits) {
            if data.is_empty() {
                return TAG_NULL;
            }
            let mut min_idx = 0;
            let mut min_val = f64::INFINITY;
            for (i, &v) in data.iter().enumerate() {
                if v < min_val {
                    min_val = v;
                    min_idx = i;
                }
            }
            box_number(min_idx as f64)
        } else {
            TAG_NULL
        }
    }
}

/// Get current time from execution context
pub extern "C" fn jit_time_current_time(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Priority 1: Use timestamp from timestamps_ptr at current_row if available
        // Note: timestamps are stored in MICROSECONDS, convert to SECONDS for TAG_TIME
        if !ctx_ref.timestamps_ptr.is_null() && ctx_ref.current_row < ctx_ref.row_count {
            let ts_micros = *ctx_ref.timestamps_ptr.add(ctx_ref.current_row);
            if ts_micros != 0 {
                let ts_seconds = ts_micros / 1_000_000;
                return jit_box(HK_TIME, ts_seconds);
            }
        }

        // Priority 2: Fall back to ExecutionContext reference datetime
        if !ctx_ref.exec_context_ptr.is_null() {
            use shape_runtime::context::ExecutionContext;
            let exec_ctx = &*(ctx_ref.exec_context_ptr as *const ExecutionContext);
            if let Some(ref_dt) = exec_ctx.get_reference_datetime() {
                let ts = ref_dt.timestamp();
                return jit_box(HK_TIME, ts);
            }
        }

        TAG_NULL
    }
}

/// Get current symbol from execution context
pub extern "C" fn jit_time_symbol(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Try to get symbol from exec_context using the public method
        if !ctx_ref.exec_context_ptr.is_null() {
            use shape_runtime::context::ExecutionContext;
            let exec_ctx = &*(ctx_ref.exec_context_ptr as *const ExecutionContext);
            if let Ok(id) = exec_ctx.get_current_id() {
                return jit_box(HK_STRING, id);
            }
        }
        TAG_NULL
    }
}

/// Get last data row from execution context (the actual last row in the dataset)
/// Returns a TAG_INT with the last row index
pub extern "C" fn jit_time_last_row(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Use DataFrame row count from JITContext
        // Note: get_all_rows() requires &mut self which we can't get from a raw pointer safely
        if ctx_ref.row_count > 0 && !ctx_ref.column_ptrs.is_null() {
            let last_idx = ctx_ref.row_count - 1;
            // Return data row index for the last row
            return box_data_row(last_idx);
        }

        TAG_NULL
    }
}

/// Generate a range of Time values
/// time_range(start, end, step) -> Array of Time values
pub extern "C" fn jit_time_range(start_bits: u64, end_bits: u64, step_bits: u64) -> u64 {
    use crate::context::JITDuration;

    // Extract start time
    let start_ts = if is_heap_kind(start_bits, HK_TIME) {
        unsafe { *jit_unbox::<i64>(start_bits) }
    } else if is_number(start_bits) {
        unbox_number(start_bits) as i64
    } else {
        return TAG_NULL;
    };

    // Extract end time
    let end_ts = if is_heap_kind(end_bits, HK_TIME) {
        unsafe { *jit_unbox::<i64>(end_bits) }
    } else if is_number(end_bits) {
        unbox_number(end_bits) as i64
    } else {
        return TAG_NULL;
    };

    // Extract step duration
    let step_secs = if is_heap_kind(step_bits, HK_DURATION) {
        unsafe {
            let dur = jit_unbox::<JITDuration>(step_bits);
            // Convert to seconds based on unit
            let secs = match dur.unit {
                0 => dur.value,            // seconds
                1 => dur.value * 60.0,     // minutes
                2 => dur.value * 3600.0,   // hours
                3 => dur.value * 86400.0,  // days
                4 => dur.value * 604800.0, // weeks
                _ => dur.value,            // default to seconds
            };
            secs as i64
        }
    } else if is_number(step_bits) {
        unbox_number(step_bits) as i64
    } else {
        return TAG_NULL;
    };

    if step_secs <= 0 {
        return TAG_NULL;
    }

    // Generate time values
    let mut times: Vec<u64> = Vec::new();
    let mut current = start_ts;

    while current < end_ts {
        // Box each time value as heap-allocated
        times.push(jit_box(HK_TIME, current));
        current += step_secs;
    }

    // Return as boxed array
    jit_box(HK_ARRAY, JitArray::from_vec(times))
}
