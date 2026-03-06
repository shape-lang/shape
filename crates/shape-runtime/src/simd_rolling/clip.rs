//! Clip operations (value clamping)

use wide::f64x4;

use super::SIMD_THRESHOLD;

// ===== Public API - Feature-gated SIMD selection =====

/// Clip values (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn clip(data: &mut [f64], min: f64, max: f64) {
    clip_simd(data, min, max)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn clip(data: &mut [f64], min: f64, max: f64) {
    clip_scalar(data, min, max)
}

// ===== Internal implementations =====

/// SIMD clip: clamp values to [min, max] range
fn clip_simd(data: &mut [f64], min: f64, max: f64) {
    if data.len() < SIMD_THRESHOLD {
        for val in data.iter_mut() {
            *val = val.clamp(min, max);
        }
        return;
    }

    let min_vec = f64x4::splat(min);
    let max_vec = f64x4::splat(max);

    let chunks = data.len() / 4;
    for chunk in 0..chunks {
        let i = chunk * 4;
        let vals = f64x4::new([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let clamped = vals.max(min_vec).min(max_vec);
        let arr = clamped.to_array();
        data[i..i + 4].copy_from_slice(&arr);
    }

    // Remainder
    for i in (chunks * 4)..data.len() {
        data[i] = data[i].clamp(min, max);
    }
}

/// Scalar clip (for non-SIMD builds)
#[cfg(not(feature = "simd"))]
fn clip_scalar(data: &mut [f64], min: f64, max: f64) {
    for val in data.iter_mut() {
        *val = val.clamp(min, max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clip() {
        let mut data = vec![-10.0, 5.0, 15.0, 20.0, 0.0];
        clip(&mut data, 0.0, 10.0);

        assert_eq!(data[0], 0.0); // clamped to min
        assert_eq!(data[1], 5.0); // unchanged
        assert_eq!(data[2], 10.0); // clamped to max
        assert_eq!(data[3], 10.0); // clamped to max
        assert_eq!(data[4], 0.0); // unchanged
    }
}
