//! FFT (Fast Fourier Transform) intrinsics — partial migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, 4 of 5 fft
//! intrinsics (`fft`, `psd`, `dominant_frequency`, `bandpass`, `harmonics`)
//! migrate to `register_typed_fn_N` typed entries via
//! [`create_fft_intrinsics_module`]. `intrinsic_ifft` remains as legacy
//! `IntrinsicFn` body pending the **N3 sub-decision** (polymorphic input:
//! TypedObject FFT-result vs (real_arr, imag_arr) two-array form). N3
//! disposition per supervisor relay (2026-05-07): N3-β (defer permanent
//! legacy) at first landing — ifft is a real DSP primitive; deletion
//! preserved as fallback only if no consumer surfaces.
//!
//! Migrated entries take `Arc<AlignedTypedBuffer>` (series + kernel)
//! and scalars (frequencies, sample_rate, num_harmonics); outputs project
//! through `ConcreteReturn::ArrayF64` (psd, bandpass) or
//! `TypedReturn::TypedObject(...)` (fft, dominant_frequency, harmonics).
//!
//! Provides FFT, IFFT, and related spectral analysis functions for:
//! - Medical signal processing (ECG, EEG)
//! - Power electronics (harmonic analysis)
//! - Audio/vibration analysis
//! - General frequency domain analysis

use super::{extract_f64_array, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use crate::marshal::{
    register_typed_fn_1, register_typed_fn_2_full, register_typed_fn_4_full,
};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::type_schema::typed_object_from_nb_pairs;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use rustfft::{FftPlanner, num_complex::Complex};
use shape_ast::error::{Result, ShapeError};
use shape_value::{AlignedTypedBuffer, ValueWord, ValueWordExt};
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

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
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
                return Ok(TypedReturn::Concrete(ConcreteReturn::TypedObject(
                    empty_fft_pairs(),
                )));
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

    register_typed_fn_1::<_, Arc<AlignedTypedBuffer>>(
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

    register_typed_fn_2_full::<_, Arc<AlignedTypedBuffer>, f64>(
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

    register_typed_fn_4_full::<_, Arc<AlignedTypedBuffer>, f64, f64, f64>(
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

    register_typed_fn_4_full::<_, Arc<AlignedTypedBuffer>, f64, i64, f64>(
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

// ───────────────────── Legacy body (1 polymorphic-input intrinsic) ─────────────────────

/// IFFT intrinsic - compute inverse Fast Fourier Transform.
///
/// **Migration deferred** pending N3 sub-decision (polymorphic input:
/// TypedObject FFT-result vs (real_arr, imag_arr) two-array form). N3-β
/// (defer permanent legacy) chosen at first landing per supervisor relay
/// (2026-05-07): "ifft is a real DSP primitive users would expect; deletion
/// preserved as fallback only if no consumer surfaces."
///
/// Usage: ifft(fft_result) -> series
/// Or:    ifft(real_series, imag_series) -> series
pub fn intrinsic_ifft(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "ifft() requires FFT result or (real, imag) series".to_string(),
            location: None,
        });
    }
    // Try to extract real/imag from a TypedObject (FFT result) or from two separate arrays
    let (real, imag) = if let Some(obj) = crate::type_schema::typed_object_to_hashmap_nb(&args[0]) {
        let real_nb = obj.get("real").ok_or_else(|| ShapeError::RuntimeError {
            message: "ifft(): FFT result missing 'real' field".to_string(),
            location: None,
        })?;
        let imag_nb = obj.get("imag").ok_or_else(|| ShapeError::RuntimeError {
            message: "ifft(): FFT result missing 'imag' field".to_string(),
            location: None,
        })?;
        let real = extract_f64_array(real_nb, "Real series")?;
        let imag = extract_f64_array(imag_nb, "Imag series")?;
        (real, imag)
    } else {
        if args.len() < 2 {
            return Err(ShapeError::RuntimeError {
                message: "ifft() requires FFT result object or (real, imag) series".to_string(),
                location: None,
            });
        }
        let real = extract_f64_array(&args[0], "Real series")?;
        let imag = extract_f64_array(&args[1], "Imag series")?;
        (real, imag)
    };

    if real.len() != imag.len() {
        return Err(ShapeError::RuntimeError {
            message: "ifft(): real and imag series must have same length".to_string(),
            location: None,
        });
    }

    let n = real.len();
    if n == 0 {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    let mut buffer: Vec<Complex<f64>> = real
        .iter()
        .zip(imag.iter())
        .map(|(&r, &i)| Complex::new(r, i))
        .collect();

    let mut planner = FftPlanner::new();
    let ifft = planner.plan_fft_inverse(n);
    ifft.process(&mut buffer);

    let scale = 1.0 / n as f64;
    let result: Vec<f64> = buffer.iter().map(|c| c.re * scale).collect();

    Ok(f64_vec_to_nb_array(result))
}

// Suppress unused-import warning for typed_object_from_nb_pairs (legacy ifft
// helper; new typed entries use TypedReturn::TypedObject directly).
#[allow(dead_code)]
fn _suppress_unused() -> ValueWord {
    typed_object_from_nb_pairs(&[])
}
