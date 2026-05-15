//! Convolution intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, the single
//! convolution intrinsic (`stencil`) migrates to a `register_typed_fn_3_full`
//! typed entry via [`create_convolution_intrinsics_module`]. Inputs are
//! `Arc<Vec<f64>>` (series + kernel) and an optional
//! `Arc<String>` mode keyword (default `"same"`). Output projects through
//! `ConcreteReturn::ArrayF64`.
//!
//! `__intrinsic_stencil` was found to have **zero stdlib/package
//! consumers** via post-bulldozer `rg` across `crates/shape-runtime/stdlib-src/`
//! and `packages/`; full-migrate-anyway maintains consumer-surface parity
//! for any in-flight Shape consumer and flags the function as a
//! deletion-candidate for shape-vm cleanup workstream's natural scope
//! (analogous to the scan.rs zero-consumer flag in N1's queue subsection).
//!
//! Provides SIMD-accelerated 1D convolution for:
//! - Physics simulations (heat diffusion, wave equation)
//! - Signal processing (FIR filters)
//! - Spatial operations (smoothing, edge detection)

use crate::marshal::register_typed_fn_3_full;
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;
use wide::f64x4;

// ───────────────────── Module factory (1 typed entry) ─────────────────────

/// Create the convolution intrinsics module with the single typed-marshal
/// entry point `__intrinsic_stencil`.
pub fn create_convolution_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::convolution");
    module.description = "1D stencil/convolution intrinsics (SIMD-accelerated)".to_string();

    register_typed_fn_3_full::<_, Arc<Vec<f64>>, Arc<Vec<f64>>, Arc<String>>(
        &mut module,
        "__intrinsic_stencil",
        "1D convolution of a Vec<number> series against a kernel; modes 'valid' / 'same' / 'full'",
        [
            ModuleParam {
                name: "series".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Input series".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "kernel".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Convolution kernel".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "mode".to_string(),
                type_name: "string".to_string(),
                required: false,
                description: "Boundary mode: 'valid', 'same' (default), or 'full'".to_string(),
                default_snippet: Some("\"same\"".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::ArrayNumber,
        |series, kernel, mode, _ctx| {
            let kernel_slice = kernel.as_slice();
            if kernel_slice.is_empty() {
                return Err("Kernel cannot be empty".to_string());
            }
            let result = convolve_1d(series.as_slice(), kernel_slice, mode.as_str())?;
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    module
}

// ───────────────────── Helpers (used by typed body) ─────────────────────

/// 1D convolution with SIMD acceleration. Reverses `kernel` once at the boundary
/// then dispatches to the `valid` / `same` / `full` shape per `mode`.
fn convolve_1d(data: &[f64], kernel: &[f64], mode: &str) -> Result<Vec<f64>, String> {
    if data.is_empty() {
        return Ok(vec![]);
    }

    let kernel_rev: Vec<f64> = kernel.iter().rev().copied().collect();

    match mode {
        "valid" => Ok(convolve_valid(data, &kernel_rev)),
        "same" => Ok(convolve_same(data, &kernel_rev)),
        "full" => Ok(convolve_full(data, &kernel_rev)),
        _ => Err(format!(
            "Unknown convolution mode: {}. Use 'valid', 'same', or 'full'",
            mode
        )),
    }
}

fn convolve_valid(data: &[f64], kernel: &[f64]) -> Vec<f64> {
    let n = data.len();
    let k = kernel.len();

    if k > n {
        return vec![];
    }

    let out_len = n - k + 1;
    let mut result = vec![0.0; out_len];

    const SIMD_THRESHOLD: usize = 16;

    if k >= 4 && out_len >= SIMD_THRESHOLD {
        let k_chunks = k / 4;

        for i in 0..out_len {
            let mut sum = f64x4::splat(0.0);
            for j in 0..k_chunks {
                let idx = j * 4;
                let d = f64x4::from(&data[i + idx..i + idx + 4]);
                let kv = f64x4::from(&kernel[idx..idx + 4]);
                sum += d * kv;
            }
            let arr = sum.to_array();
            let mut total = arr[0] + arr[1] + arr[2] + arr[3];
            for j in (k_chunks * 4)..k {
                total += data[i + j] * kernel[j];
            }
            result[i] = total;
        }
    } else {
        for i in 0..out_len {
            let mut sum = 0.0;
            for j in 0..k {
                sum += data[i + j] * kernel[j];
            }
            result[i] = sum;
        }
    }

    result
}

fn convolve_same(data: &[f64], kernel: &[f64]) -> Vec<f64> {
    let n = data.len();
    let k = kernel.len();

    if n == 0 {
        return vec![];
    }

    let mut result = vec![0.0; n];
    let half = k / 2;

    for i in 0..n {
        let mut sum = 0.0;
        for j in 0..k {
            let data_idx = i as isize + j as isize - half as isize;
            if data_idx >= 0 && (data_idx as usize) < n {
                sum += data[data_idx as usize] * kernel[j];
            }
        }
        result[i] = sum;
    }

    result
}

fn convolve_full(data: &[f64], kernel: &[f64]) -> Vec<f64> {
    let n = data.len();
    let k = kernel.len();

    if n == 0 || k == 0 {
        return vec![];
    }

    let out_len = n + k - 1;
    let mut result = vec![0.0; out_len];

    for i in 0..out_len {
        let mut sum = 0.0;
        for j in 0..k {
            let data_idx = i as isize - j as isize;
            if data_idx >= 0 && (data_idx as usize) < n {
                sum += data[data_idx as usize] * kernel[j];
            }
        }
        result[i] = sum;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stencil_smoothing() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let kernel = vec![0.25, 0.5, 0.25];
        let kernel_rev: Vec<f64> = kernel.iter().rev().copied().collect();
        let result = convolve_same(&data, &kernel_rev);
        assert_eq!(result.len(), 5);
        assert!((result[2] - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_stencil_valid() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let kernel = vec![1.0, 0.0, -1.0];
        let kernel_rev: Vec<f64> = kernel.iter().rev().copied().collect();
        let result = convolve_valid(&data, &kernel_rev);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_stencil_heat_diffusion() {
        let data = vec![1.0, 1.0, 1.0, 1.0, 1.0];
        let kernel = vec![0.25, 0.5, 0.25];
        let kernel_rev: Vec<f64> = kernel.iter().rev().copied().collect();
        let result = convolve_same(&data, &kernel_rev);
        assert!((result[2] - 1.0).abs() < 0.01);
    }
}
