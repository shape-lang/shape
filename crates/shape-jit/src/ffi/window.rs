// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (Window functions only operate on NaN-boxed numbers, no heap allocations)
//!
//! Window Function FFI for JIT
//!
//! External C functions for SQL-style window functions called from JIT-compiled code.
//! Window functions require partition and ordering context, so these FFI functions
//! operate on pre-computed partition data passed from the query executor.

use super::super::nan_boxing::*;

// ============================================================================
// Ranking Window Functions
// ============================================================================

/// ROW_NUMBER() - Returns sequential row number within partition
/// Args: current_idx (0-based position in partition)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_row_number(current_idx: u64) -> u64 {
    let idx = if is_number(current_idx) {
        unbox_number(current_idx) as usize
    } else {
        return box_number(f64::NAN);
    };
    // ROW_NUMBER is 1-based
    box_number((idx + 1) as f64)
}

/// RANK() - Returns rank with gaps for ties
/// Args: current_idx, partition_size
/// Note: Simplified - assumes ORDER BY already applied and no ties
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_rank(current_idx: u64, _partition_size: u64) -> u64 {
    let idx = if is_number(current_idx) {
        unbox_number(current_idx) as usize
    } else {
        return box_number(f64::NAN);
    };
    // Simplified: each row gets sequential rank (no tie detection)
    box_number((idx + 1) as f64)
}

/// DENSE_RANK() - Returns rank without gaps for ties
/// Args: current_idx, partition_size
/// Note: Simplified - assumes ORDER BY already applied and no ties
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_dense_rank(current_idx: u64, _partition_size: u64) -> u64 {
    let idx = if is_number(current_idx) {
        unbox_number(current_idx) as usize
    } else {
        return box_number(f64::NAN);
    };
    // Simplified: each row gets sequential rank (no tie detection)
    box_number((idx + 1) as f64)
}

/// NTILE(n) - Distributes rows into n buckets
/// Args: current_idx, partition_size, num_buckets
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_ntile(current_idx: u64, partition_size: u64, num_buckets: u64) -> u64 {
    let idx = if is_number(current_idx) {
        unbox_number(current_idx) as usize
    } else {
        return box_number(f64::NAN);
    };
    let size = if is_number(partition_size) {
        unbox_number(partition_size) as usize
    } else {
        return box_number(f64::NAN);
    };
    let n = if is_number(num_buckets) {
        unbox_number(num_buckets) as usize
    } else {
        return box_number(f64::NAN);
    };

    if size == 0 || n == 0 {
        return box_number(1.0);
    }

    let bucket = (idx * n / size) + 1;
    box_number(bucket as f64)
}

// ============================================================================
// Navigation Window Functions
// ============================================================================

/// LAG(value, offset) - Returns value from offset rows before current
/// Args: current_value, offset, default_value, is_valid (1 if in bounds, 0 if out)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_lag(
    current_value: u64,
    _offset: u64,
    default_value: u64,
    is_valid: u64,
) -> u64 {
    let valid = if is_number(is_valid) {
        unbox_number(is_valid) as i32
    } else {
        0
    };

    if valid != 0 {
        current_value
    } else {
        default_value
    }
}

/// LEAD(value, offset) - Returns value from offset rows after current
/// Args: current_value, offset, default_value, is_valid (1 if in bounds, 0 if out)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_lead(
    current_value: u64,
    _offset: u64,
    default_value: u64,
    is_valid: u64,
) -> u64 {
    let valid = if is_number(is_valid) {
        unbox_number(is_valid) as i32
    } else {
        0
    };

    if valid != 0 {
        current_value
    } else {
        default_value
    }
}

/// FIRST_VALUE(expr) - Returns first value in the window frame
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_first_value(first_value: u64) -> u64 {
    first_value
}

/// LAST_VALUE(expr) - Returns last value in the window frame
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_last_value(last_value: u64) -> u64 {
    last_value
}

/// NTH_VALUE(expr, n) - Returns nth value in the window frame
/// Args: nth_value, is_valid (1 if nth exists, 0 if not)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_nth_value(nth_value: u64, is_valid: u64) -> u64 {
    let valid = if is_number(is_valid) {
        unbox_number(is_valid) as i32
    } else {
        0
    };

    if valid != 0 { nth_value } else { TAG_NULL }
}

// ============================================================================
// Aggregate Window Functions
// ============================================================================

/// SUM() over window frame
/// Args: running_sum (pre-computed sum over frame)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_sum(running_sum: u64) -> u64 {
    running_sum
}

/// AVG() over window frame
/// Args: running_sum, count
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_avg(running_sum: u64, count: u64) -> u64 {
    let sum = if is_number(running_sum) {
        unbox_number(running_sum)
    } else {
        return box_number(f64::NAN);
    };
    let cnt = if is_number(count) {
        unbox_number(count)
    } else {
        return box_number(f64::NAN);
    };

    if cnt == 0.0 {
        return TAG_NULL;
    }

    box_number(sum / cnt)
}

/// MIN() over window frame
/// Args: running_min (pre-computed min over frame)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_min(running_min: u64) -> u64 {
    running_min
}

/// MAX() over window frame
/// Args: running_max (pre-computed max over frame)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_max(running_max: u64) -> u64 {
    running_max
}

/// COUNT() over window frame
/// Args: count (pre-computed count over frame)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_count(count: u64) -> u64 {
    count
}

// ============================================================================
// Window Frame Helpers
// ============================================================================

/// Calculate frame start index
/// Args: current_idx, preceding (n preceding or -1 for unbounded)
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_frame_start(current_idx: u64, preceding: u64) -> u64 {
    let idx = if is_number(current_idx) {
        unbox_number(current_idx) as i64
    } else {
        return box_number(0.0);
    };
    let prec = if is_number(preceding) {
        unbox_number(preceding) as i64
    } else {
        return box_number(0.0);
    };

    if prec < 0 {
        // Unbounded preceding
        box_number(0.0)
    } else {
        box_number((idx - prec).max(0) as f64)
    }
}

/// Calculate frame end index
/// Args: current_idx, following (n following or -1 for unbounded), partition_size
#[unsafe(no_mangle)]
pub extern "C" fn jit_window_frame_end(
    current_idx: u64,
    following: u64,
    partition_size: u64,
) -> u64 {
    let idx = if is_number(current_idx) {
        unbox_number(current_idx) as i64
    } else {
        return box_number(0.0);
    };
    let foll = if is_number(following) {
        unbox_number(following) as i64
    } else {
        return box_number(0.0);
    };
    let size = if is_number(partition_size) {
        unbox_number(partition_size) as i64
    } else {
        return box_number(0.0);
    };

    if foll < 0 {
        // Unbounded following
        box_number((size - 1).max(0) as f64)
    } else {
        box_number((idx + foll).min(size - 1).max(0) as f64)
    }
}
