//! Shared utilities for chart calculations

use crate::error::{ChartError, Result};

/// Calculate nice price levels for grid and axis
pub fn calculate_price_levels(price_min: f64, price_max: f64, content_height: f32) -> Vec<f64> {
    let price_range = price_max - price_min;
    let target_label_count = (content_height / 20.0) as i32; // 20 pixels between labels for denser grid
    let raw_step = price_range / target_label_count as f64;
    let nice_step = find_nice_number(raw_step);

    // Start from a nice round number
    let start_price = (price_min / nice_step).floor() * nice_step;
    let mut levels = Vec::new();
    let mut current_price = start_price;

    while current_price <= price_max {
        if current_price >= price_min {
            levels.push(current_price);
        }
        current_price += nice_step;
    }

    levels
}

/// Find a "nice" round number for price steps
pub fn find_nice_number(value: f64) -> f64 {
    let magnitude = 10.0_f64.powf(value.log10().floor());
    let normalized = value / magnitude;

    let nice_normalized = if normalized < 1.5 {
        1.0
    } else if normalized < 2.5 {
        2.0
    } else if normalized < 3.0 {
        2.5
    } else if normalized < 7.0 {
        5.0
    } else {
        10.0
    };

    nice_normalized * magnitude
}

/// Compute the mean squared error (MSE) between generated RGBA image data and a reference PNG file.
/// A lower MSE indicates the generated image is more similar to the reference.
///
/// * `generated_rgba` - Raw RGBA bytes produced by the renderer.
/// * `width` / `height`   - Dimensions of the generated image.
/// * `reference_path`     - Path to the reference image on disk (PNG).
pub fn compare_image_mse(
    generated_rgba: &[u8],
    width: u32,
    height: u32,
    reference_path: &str,
) -> Result<f64> {
    let reference_img = image::open(reference_path)
        .map_err(|e| {
            ChartError::internal(format!(
                "Failed to open reference image {}: {}",
                reference_path, e
            ))
        })?
        .to_rgba8();

    // Verify dimensions match.
    if reference_img.width() != width || reference_img.height() != height {
        return Err(ChartError::internal(format!(
            "Reference image dimensions {}x{} do not match generated {}x{}",
            reference_img.width(),
            reference_img.height(),
            width,
            height
        )));
    }

    let ref_pixels = reference_img.as_raw();
    if ref_pixels.len() != generated_rgba.len() {
        return Err(ChartError::internal(
            "Pixel data length mismatch between images",
        ));
    }

    let mut mse: f64 = 0.0;
    for (a, b) in generated_rgba.iter().zip(ref_pixels.iter()) {
        let diff = (*a as f64) - (*b as f64);
        mse += diff * diff;
    }

    mse /= generated_rgba.len() as f64;
    Ok(mse)
}
