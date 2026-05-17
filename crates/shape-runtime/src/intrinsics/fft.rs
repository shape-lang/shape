//! FFT (Fast Fourier Transform) intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, all 5 surviving fft
//! intrinsics (`fft`, `psd`, `dominant_frequency`, `bandpass`, `harmonics`)
//! migrate to `register_typed_fn_N` typed entries via
//! [`create_fft_intrinsics_module`]. The 6th legacy `intrinsic_ifft` was
//! deleted as orphan-cleanup per supervisor sign-off relayed 2026-05-07
//! (N3 → DELETE-NOW): zero stdlib/package consumers verified pre-deletion;
//! N3 architectural decision (polymorphic-input dispatch via TypedObject
//! FFT-result vs (real_arr, imag_arr) two-array form) deferred pending
//! future consumer with similar polymorphic-input shape. Same precedent
//! as scan.rs deletion at `663b63a`.
//!
//! Migrated entries take `Arc<Vec<f64>>` (series + kernel)
//! and scalars (frequencies, sample_rate, num_harmonics); outputs project
//! through `ConcreteReturn::ArrayF64` (psd, bandpass) or
//! `TypedReturn::TypedObject(...)` (fft, dominant_frequency, harmonics).
//!
//! Provides FFT and related spectral analysis functions for:
//! - Medical signal processing (ECG, EEG)
//! - Power electronics (harmonic analysis)
//! - Audio/vibration analysis
//! - General frequency domain analysis

use crate::marshal::{
    register_typed_fn_1, register_typed_fn_2_full, register_typed_fn_4_full,
};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use rustfft::{FftPlanner, num_complex::Complex};
use std::sync::Arc;

// ───────────────────── Module factory (4 typed entries) ─────────────────────

/// Create the fft intrinsics module with 4 typed-marshal entry points
/// (`fft`, `psd`, `dominant_frequency`, `bandpass`, `harmonics`).
/// `ifft` remains as legacy `IntrinsicFn` body in this module until the N3
/// sub-decision (polymorphic input: TypedObject vs (real, imag) arrays)
/// resolves.
pub fn create_fft_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::fft");
    module.description =
        "FFT and frequency-domain analysis intrinsics (typed entries; ifft stays legacy pending N3 sub-decision)"
            .to_string();

    register_typed_fn_1::<_, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_fft",
        "Forward FFT: real series → { real, imag, magnitude, phase, frequencies, n }",
        "series",
        "Array<number>",
        ConcreteType::TypedObject,
        |series, _ctx| {
            let data = series.as_slice();
            let n = data.len();
            if n == 0 {
                return Ok(TypedReturn::TypedObject(empty_fft_pairs()));
            }
            let mut buffer: Vec<Complex<f64>> =
                data.iter().map(|&x| Complex::new(x, 0.0)).collect();
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(n);
            fft.process(&mut buffer);
            let real: Vec<f64> = buffer.iter().map(|c| c.re).collect();
            let imag: Vec<f64> = buffer.iter().map(|c| c.im).collect();
            let magnitude: Vec<f64> = buffer.iter().map(|c| c.norm()).collect();
            let phase: Vec<f64> = buffer.iter().map(|c| c.im.atan2(c.re)).collect();
            let frequencies: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();
            Ok(TypedReturn::TypedObject(vec![
                ("real".to_string(), ConcreteReturn::ArrayF64(real)),
                ("imag".to_string(), ConcreteReturn::ArrayF64(imag)),
                ("magnitude".to_string(), ConcreteReturn::ArrayF64(magnitude)),
                ("phase".to_string(), ConcreteReturn::ArrayF64(phase)),
                (
                    "frequencies".to_string(),
                    ConcreteReturn::ArrayF64(frequencies),
                ),
                ("n".to_string(), ConcreteReturn::F64(n as f64)),
            ]))
        },
    );

    register_typed_fn_1::<_, Arc<Vec<f64>>>(
        &mut module,
        "__intrinsic_psd",
        "Power spectral density: scaled |FFT(x)|^2",
        "series",
        "Array<number>",
        ConcreteType::ArrayNumber,
        |series, _ctx| {
            let data = series.as_slice();
            let n = data.len();
            if n == 0 {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![])));
            }
            let mut buffer: Vec<Complex<f64>> =
                data.iter().map(|&x| Complex::new(x, 0.0)).collect();
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(n);
            fft.process(&mut buffer);
            let scale = 1.0 / n as f64;
            let psd: Vec<f64> = buffer.iter().map(|c| c.norm_sqr() * scale).collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(psd)))
        },
    );

    register_typed_fn_2_full::<_, Arc<Vec<f64>>, f64>(
        &mut module,
        "__intrinsic_dominant_frequency",
        "Dominant frequency: { frequency, magnitude, bin } of strongest spectrum bin",
        [
            ModuleParam {
                name: "series".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Input series".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "sample_rate".to_string(),
                type_name: "number".to_string(),
                required: false,
                description: "Sample rate (default 1.0)".to_string(),
                default_snippet: Some("1.0".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::TypedObject,
        |series, sample_rate, _ctx| {
            let data = series.as_slice();
            let n = data.len();
            if n == 0 {
                return Err("dominant_frequency(): empty series".to_string());
            }
            let mut buffer: Vec<Complex<f64>> =
                data.iter().map(|&x| Complex::new(x, 0.0)).collect();
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(n);
            fft.process(&mut buffer);
            let half_n = n / 2;
            let (max_bin, max_mag) = buffer[1..=half_n]
                .iter()
                .enumerate()
                .map(|(i, c)| (i + 1, c.norm()))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap_or((0, 0.0));
            let frequency = max_bin as f64 * sample_rate / n as f64;
            Ok(TypedReturn::TypedObject(vec![
                ("frequency".to_string(), ConcreteReturn::F64(frequency)),
                ("magnitude".to_string(), ConcreteReturn::F64(max_mag)),
                ("bin".to_string(), ConcreteReturn::F64(max_bin as f64)),
            ]))
        },
    );

    register_typed_fn_4_full::<_, Arc<Vec<f64>>, f64, f64, f64>(
        &mut module,
        "__intrinsic_bandpass",
        "Bandpass filter via FFT: zero frequencies outside [low_freq, high_freq]",
        [
            ModuleParam {
                name: "series".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Input series".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "low_freq".to_string(),
                type_name: "number".to_string(),
                required: true,
                description: "Low cutoff frequency".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "high_freq".to_string(),
                type_name: "number".to_string(),
                required: true,
                description: "High cutoff frequency".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "sample_rate".to_string(),
                type_name: "number".to_string(),
                required: false,
                description: "Sample rate (default 1.0)".to_string(),
                default_snippet: Some("1.0".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::ArrayNumber,
        |series, low_freq, high_freq, sample_rate, _ctx| {
            let data = series.as_slice();
            let n = data.len();
            if n == 0 {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![])));
            }
            let mut buffer: Vec<Complex<f64>> =
                data.iter().map(|&x| Complex::new(x, 0.0)).collect();
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(n);
            fft.process(&mut buffer);
            let freq_resolution = sample_rate / n as f64;
            for (i, c) in buffer.iter_mut().enumerate() {
                let freq = if i <= n / 2 {
                    i as f64 * freq_resolution
                } else {
                    (n - i) as f64 * freq_resolution
                };
                if freq < low_freq || freq > high_freq {
                    *c = Complex::new(0.0, 0.0);
                }
            }
            let ifft = planner.plan_fft_inverse(n);
            ifft.process(&mut buffer);
            let scale = 1.0 / n as f64;
            let result: Vec<f64> = buffer.iter().map(|c| c.re * scale).collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    register_typed_fn_4_full::<_, Arc<Vec<f64>>, f64, i64, f64>(
        &mut module,
        "__intrinsic_harmonics",
        "Harmonic analysis: extract harmonics of a fundamental frequency",
        [
            ModuleParam {
                name: "series".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Input series".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "fundamental_freq".to_string(),
                type_name: "number".to_string(),
                required: true,
                description: "Fundamental frequency".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "num_harmonics".to_string(),
                type_name: "int".to_string(),
                required: true,
                description: "Number of harmonics to extract".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "sample_rate".to_string(),
                type_name: "number".to_string(),
                required: false,
                description: "Sample rate (default 1.0)".to_string(),
                default_snippet: Some("1.0".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::TypedObject,
        |series, fundamental, num_harmonics, sample_rate, _ctx| {
            if num_harmonics < 0 {
                return Err("harmonics(): num_harmonics must be non-negative".to_string());
            }
            let data = series.as_slice();
            let n = data.len();
            if n == 0 {
                return Err("harmonics(): empty series".to_string());
            }
            let num_harmonics = num_harmonics as usize;
            let mut buffer: Vec<Complex<f64>> =
                data.iter().map(|&x| Complex::new(x, 0.0)).collect();
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(n);
            fft.process(&mut buffer);
            let freq_resolution = sample_rate / n as f64;
            let mut harmonic_freqs = Vec::with_capacity(num_harmonics);
            let mut magnitudes = Vec::with_capacity(num_harmonics);
            let mut phases = Vec::with_capacity(num_harmonics);
            for h in 1..=num_harmonics {
                let target_freq = fundamental * h as f64;
                let bin = (target_freq / freq_resolution).round() as usize;
                if bin < n / 2 {
                    let c = buffer[bin];
                    harmonic_freqs.push(target_freq);
                    magnitudes.push(c.norm());
                    phases.push(c.im.atan2(c.re));
                }
            }
            Ok(TypedReturn::TypedObject(vec![
                (
                    "harmonics".to_string(),
                    ConcreteReturn::ArrayF64(harmonic_freqs),
                ),
                (
                    "magnitudes".to_string(),
                    ConcreteReturn::ArrayF64(magnitudes),
                ),
                ("phases".to_string(), ConcreteReturn::ArrayF64(phases)),
            ]))
        },
    );

    module
}

// ───────────────────── Empty-FFT helper ─────────────────────

fn empty_fft_pairs() -> Vec<(String, ConcreteReturn)> {
    vec![
        ("real".to_string(), ConcreteReturn::ArrayF64(vec![])),
        ("imag".to_string(), ConcreteReturn::ArrayF64(vec![])),
        ("magnitude".to_string(), ConcreteReturn::ArrayF64(vec![])),
        ("phase".to_string(), ConcreteReturn::ArrayF64(vec![])),
        ("frequencies".to_string(), ConcreteReturn::ArrayF64(vec![])),
        ("n".to_string(), ConcreteReturn::F64(0.0)),
    ]
}

