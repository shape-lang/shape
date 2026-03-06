//! SIMD-optimized forward-fill operations for table alignment
//!
//! This module provides high-performance forward-fill operations using
//! SIMD instructions when available.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Forward-fill data using SIMD instructions for x86_64
///
/// This function performs forward-fill on a slice of f64 values,
/// propagating the last non-NaN value forward to fill NaN gaps.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(unused_assignments)]
unsafe fn forward_fill_avx2(data: &mut [f64]) {
    if data.is_empty() {
        return;
    }

    let mut last_valid = f64::NAN;
    let mut i = 0;

    // Find first non-NaN value
    while i < data.len() && data[i].is_nan() {
        i += 1;
    }

    if i < data.len() {
        last_valid = data[i];
    }

    // Process 4 values at a time using AVX2
    while i + 4 <= data.len() {
        let chunk = unsafe { _mm256_loadu_pd(&data[i] as *const f64) };

        // Create mask for NaN values
        let nan_mask = _mm256_cmp_pd(chunk, chunk, _CMP_UNORD_Q);

        // Create vector with last valid value
        let last_valid_vec = _mm256_set1_pd(last_valid);

        // Blend: use original value if not NaN, otherwise use last_valid
        let result = _mm256_blendv_pd(chunk, last_valid_vec, nan_mask);

        // Store result
        unsafe { _mm256_storeu_pd(&mut data[i] as *mut f64, result) };

        // Update last_valid with the last non-NaN value in this chunk
        for j in (i..i + 4).rev() {
            if !data[j].is_nan() {
                last_valid = data[j];
                break;
            }
        }

        i += 4;
    }

    // Handle remaining elements
    while i < data.len() {
        if data[i].is_nan() {
            data[i] = last_valid;
        } else {
            last_valid = data[i];
        }
        i += 1;
    }
}

/// Forward-fill data using SIMD instructions for x86_64 with SSE2
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[allow(unused_assignments)]
unsafe fn forward_fill_sse2(data: &mut [f64]) {
    if data.is_empty() {
        return;
    }

    let mut last_valid = f64::NAN;
    let mut i = 0;

    // Find first non-NaN value
    while i < data.len() && data[i].is_nan() {
        i += 1;
    }

    if i < data.len() {
        last_valid = data[i];
    }

    // Process 2 values at a time using SSE2
    while i + 2 <= data.len() {
        let chunk = unsafe { _mm_loadu_pd(&data[i] as *const f64) };

        // Check for NaN values
        let nan_mask = _mm_cmpunord_pd(chunk, chunk);

        // Create vector with last valid value
        let last_valid_vec = _mm_set1_pd(last_valid);

        // Blend: use original value if not NaN, otherwise use last_valid
        let result = _mm_or_pd(
            _mm_and_pd(nan_mask, last_valid_vec),
            _mm_andnot_pd(nan_mask, chunk),
        );

        // Store result
        unsafe { _mm_storeu_pd(&mut data[i] as *mut f64, result) };

        // Update last_valid with the last non-NaN value in this chunk
        for j in (i..i + 2).rev() {
            if !data[j].is_nan() {
                last_valid = data[j];
                break;
            }
        }

        i += 2;
    }

    // Handle remaining element
    if i < data.len() {
        if data[i].is_nan() {
            data[i] = last_valid;
        } else {
            last_valid = data[i];
        }
    }
}

/// Fallback scalar implementation for forward-fill
fn forward_fill_scalar(data: &mut [f64]) {
    if data.is_empty() {
        return;
    }

    let mut last_valid = f64::NAN;

    for value in data.iter_mut() {
        if value.is_nan() {
            *value = last_valid;
        } else {
            last_valid = *value;
        }
    }
}

/// Forward-fill data with automatic SIMD detection
///
/// This function automatically selects the best available SIMD
/// implementation based on CPU features.
pub fn forward_fill(data: &mut [f64]) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { forward_fill_avx2(data) }
        } else if is_x86_feature_detected!("sse2") {
            unsafe { forward_fill_sse2(data) }
        } else {
            forward_fill_scalar(data)
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        forward_fill_scalar(data)
    }
}

/// Forward-fill with interpolation for upsampling
///
/// This function performs forward-fill with optional linear interpolation
/// between known values for smoother upsampling.
pub fn forward_fill_interpolate(
    source: &[f64],
    source_indices: &[usize],
    target_size: usize,
    interpolate: bool,
) -> Vec<f64> {
    let mut result = vec![f64::NAN; target_size];

    if source.is_empty() || source_indices.is_empty() {
        return result;
    }

    // Place source values at their indices
    for (&idx, &value) in source_indices.iter().zip(source.iter()) {
        if idx < target_size {
            result[idx] = value;
        }
    }

    if interpolate {
        // Linear interpolation between known values
        let mut prev_idx = None;
        let mut prev_val = f64::NAN;

        for (i, &idx) in source_indices.iter().enumerate() {
            if idx >= target_size {
                break;
            }

            let val = source[i];

            if let Some(p_idx) = prev_idx {
                // Interpolate between prev_idx and idx
                let gap = idx - p_idx;
                if gap > 1 {
                    let step = (val - prev_val) / gap as f64;
                    for j in 1..gap {
                        result[p_idx + j] = prev_val + step * j as f64;
                    }
                }
            }

            prev_idx = Some(idx);
            prev_val = val;
        }

        // Forward-fill remaining NaN values
        forward_fill(&mut result);
    } else {
        // Simple forward-fill without interpolation
        forward_fill(&mut result);
    }

    result
}

/// Batch forward-fill for multiple series
///
/// This function performs forward-fill on multiple series in parallel
/// using SIMD instructions for maximum performance.
pub fn batch_forward_fill(series: &mut [Vec<f64>]) {
    // Process each series
    for data in series.iter_mut() {
        forward_fill(data);
    }
}
