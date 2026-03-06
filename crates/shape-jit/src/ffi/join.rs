// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (JOIN helpers only operate on NaN-boxed numbers and tags, no heap allocations)
//!
//! JOIN FFI Functions for JIT
//!
//! External C functions for JOIN operations called from JIT-compiled code.
//! JOIN operations are typically executed at the query level, not within
//! tight expression loops, so these functions delegate to the runtime.

use super::super::nan_boxing::*;

// ============================================================================
// JOIN Condition Helpers
// ============================================================================

/// Check if two values match for JOIN condition (equality)
/// Args: left_value, right_value
/// Returns: 1 (true) or 0 (false)
#[unsafe(no_mangle)]
pub extern "C" fn jit_join_values_equal(left: u64, right: u64) -> u64 {
    // Handle NULL comparisons
    if left == TAG_NULL || right == TAG_NULL {
        return box_number(0.0); // NULL != anything
    }

    // Compare numbers
    if is_number(left) && is_number(right) {
        let l = unbox_number(left);
        let r = unbox_number(right);

        // NaN handling
        if l.is_nan() && r.is_nan() {
            return box_number(1.0); // NaN == NaN for JOIN purposes
        }
        if l.is_nan() || r.is_nan() {
            return box_number(0.0);
        }

        // Float comparison with epsilon
        if (l - r).abs() < f64::EPSILON {
            return box_number(1.0);
        }
        return box_number(0.0);
    }

    // For pointers (strings, objects), compare bit patterns
    // This is a simplified comparison - full string/object comparison
    // would require dereferencing
    if left == right {
        box_number(1.0)
    } else {
        box_number(0.0)
    }
}

/// Temporal JOIN match check
/// Args: left_timestamp, right_timestamp, tolerance_ms
/// Returns: 1 (true) if within tolerance, 0 (false) otherwise
#[unsafe(no_mangle)]
pub extern "C" fn jit_temporal_match(
    left_timestamp: u64,
    right_timestamp: u64,
    tolerance_ms: u64,
) -> u64 {
    let l_ts = if is_number(left_timestamp) {
        unbox_number(left_timestamp)
    } else {
        return box_number(0.0);
    };

    let r_ts = if is_number(right_timestamp) {
        unbox_number(right_timestamp)
    } else {
        return box_number(0.0);
    };

    let tolerance = if is_number(tolerance_ms) {
        unbox_number(tolerance_ms)
    } else {
        return box_number(0.0);
    };

    let diff = (l_ts - r_ts).abs();

    if diff <= tolerance {
        box_number(1.0)
    } else {
        box_number(0.0)
    }
}

/// Check if a value is NULL (for LEFT/RIGHT/FULL JOIN null handling)
#[unsafe(no_mangle)]
pub extern "C" fn jit_join_is_null(value: u64) -> u64 {
    if value == TAG_NULL {
        box_number(1.0)
    } else {
        box_number(0.0)
    }
}

/// Return NULL value (for outer join null filling)
#[unsafe(no_mangle)]
pub extern "C" fn jit_join_null() -> u64 {
    TAG_NULL
}

/// Coalesce: return first non-NULL value
/// Args: left, right
#[unsafe(no_mangle)]
pub extern "C" fn jit_join_coalesce(left: u64, right: u64) -> u64 {
    if left != TAG_NULL { left } else { right }
}
