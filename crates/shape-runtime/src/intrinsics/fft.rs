//! FFT (Fast Fourier Transform) intrinsics for frequency domain analysis
//!
//! Provides FFT, IFFT, and related spectral analysis functions for:
//! - Medical signal processing (ECG, EEG)
//! - Power electronics (harmonic analysis)
//! - Audio/vibration analysis
//! - General frequency domain analysis

use super::{extract_f64, extract_f64_array, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use crate::type_schema::typed_object_from_nb_pairs;
use rustfft::{FftPlanner, num_complex::Complex};
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;

/// FFT intrinsic - compute forward Fast Fourier Transform
///
/// Usage: fft(series) -> { real: [...], imag: [...], magnitude: [...], phase: [...] }
pub fn intrinsic_fft(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "fft() requires a series argument".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Column")?;
    let n = data.len();

    if n == 0 {
        return Ok(empty_fft_result());
    }

    let mut buffer: Vec<Complex<f64>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut buffer);

    let real: Vec<f64> = buffer.iter().map(|c| c.re).collect();
    let imag: Vec<f64> = buffer.iter().map(|c| c.im).collect();
    let magnitude: Vec<f64> = buffer.iter().map(|c| c.norm()).collect();
    let phase: Vec<f64> = buffer.iter().map(|c| c.im.atan2(c.re)).collect();
    let frequencies: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();

    Ok(typed_object_from_nb_pairs(&[
        ("real", f64_vec_to_nb_array(real)),
        ("imag", f64_vec_to_nb_array(imag)),
        ("magnitude", f64_vec_to_nb_array(magnitude)),
        ("phase", f64_vec_to_nb_array(phase)),
        ("frequencies", f64_vec_to_nb_array(frequencies)),
        ("n", ValueWord::from_f64(n as f64)),
    ]))
}

/// IFFT intrinsic - compute inverse Fast Fourier Transform
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
        // FFT result object: extract "real" and "imag" fields
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

/// Power spectral density - magnitude squared of FFT
///
/// Usage: psd(series) -> series
pub fn intrinsic_psd(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "psd() requires a series argument".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Column")?;
    let n = data.len();

    if n == 0 {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    let mut buffer: Vec<Complex<f64>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut buffer);

    let scale = 1.0 / n as f64;
    let psd: Vec<f64> = buffer.iter().map(|c| c.norm_sqr() * scale).collect();

    Ok(f64_vec_to_nb_array(psd))
}

/// Dominant frequency - find the frequency with highest magnitude
///
/// Usage: dominant_frequency(series, sample_rate?) -> { frequency, magnitude, bin }
pub fn intrinsic_dominant_frequency(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "dominant_frequency() requires a series argument".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Column")?;
    let sample_rate = if args.len() > 1 {
        args[1].as_number_coerce().unwrap_or(1.0)
    } else {
        1.0
    };

    let n = data.len();
    if n == 0 {
        return Err(ShapeError::RuntimeError {
            message: "dominant_frequency(): empty series".to_string(),
            location: None,
        });
    }

    let mut buffer: Vec<Complex<f64>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();

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

    Ok(typed_object_from_nb_pairs(&[
        ("frequency", ValueWord::from_f64(frequency)),
        ("magnitude", ValueWord::from_f64(max_mag)),
        ("bin", ValueWord::from_f64(max_bin as f64)),
    ]))
}

/// Bandpass filter using FFT
///
/// Usage: bandpass(series, low_freq, high_freq, sample_rate?) -> series
pub fn intrinsic_bandpass(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() < 3 {
        return Err(ShapeError::RuntimeError {
            message: "bandpass() requires (series, low_freq, high_freq) arguments".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Column")?;
    let low_freq = extract_f64(&args[1], "low_freq")?;
    let high_freq = extract_f64(&args[2], "high_freq")?;
    let sample_rate = if args.len() > 3 {
        args[3].as_number_coerce().unwrap_or(1.0)
    } else {
        1.0
    };

    let n = data.len();
    if n == 0 {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    let mut buffer: Vec<Complex<f64>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();

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

    Ok(f64_vec_to_nb_array(result))
}

/// Harmonic analysis - extract harmonics of a fundamental frequency
///
/// Usage: harmonics(series, fundamental_freq, num_harmonics, sample_rate?) -> { harmonics, magnitudes, phases }
pub fn intrinsic_harmonics(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() < 3 {
        return Err(ShapeError::RuntimeError {
            message: "harmonics() requires (series, fundamental_freq, num_harmonics) arguments"
                .to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Column")?;
    let fundamental = extract_f64(&args[1], "fundamental_freq")?;
    let num_harmonics = args[2]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "harmonics(): num_harmonics must be a number".to_string(),
            location: None,
        })? as usize;
    let sample_rate = if args.len() > 3 {
        args[3].as_number_coerce().unwrap_or(1.0)
    } else {
        1.0
    };

    let n = data.len();
    if n == 0 {
        return Err(ShapeError::RuntimeError {
            message: "harmonics(): empty series".to_string(),
            location: None,
        });
    }

    let mut buffer: Vec<Complex<f64>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();

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

    Ok(typed_object_from_nb_pairs(&[
        ("harmonics", f64_vec_to_nb_array(harmonic_freqs)),
        ("magnitudes", f64_vec_to_nb_array(magnitudes)),
        ("phases", f64_vec_to_nb_array(phases)),
    ]))
}

// ============================================================================
// Helper functions
// ============================================================================

fn empty_fft_result() -> ValueWord {
    let empty = f64_vec_to_nb_array(vec![]);
    typed_object_from_nb_pairs(&[
        ("real", empty.clone()),
        ("imag", empty.clone()),
        ("magnitude", empty.clone()),
        ("phase", empty.clone()),
        ("frequencies", empty),
        ("n", ValueWord::from_f64(0.0)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> ExecutionContext {
        ExecutionContext::new_empty()
    }

    fn make_array(data: Vec<f64>) -> ValueWord {
        f64_vec_to_nb_array(data)
    }

    #[test]
    fn test_fft_simple_sine() {
        let mut ctx = make_ctx();

        let n = 100;
        let freq = 10.0;
        let sample_rate = 100.0;
        let data: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate).sin())
            .collect();

        let result = intrinsic_fft(&[make_array(data)], &mut ctx).unwrap();

        // FFT returns TypedObject via ValueWord
        let vm_result = result.clone();
        assert_eq!(vm_result.type_name(), "object");
    }

    #[test]
    fn test_fft_ifft_roundtrip() {
        let mut ctx = make_ctx();

        let original: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];

        let fft_result = intrinsic_fft(&[make_array(original.clone())], &mut ctx).unwrap();
        let reconstructed = intrinsic_ifft(&[fft_result], &mut ctx).unwrap();

        let arr = reconstructed
            .as_any_array()
            .expect("Expected array")
            .to_generic();
        let data: Vec<f64> = arr
            .iter()
            .map(|v| v.as_number_coerce().expect("Expected number in array"))
            .collect();
        for (i, (&orig, &recon)) in original.iter().zip(data.iter()).enumerate() {
            assert!(
                (orig - recon).abs() < 1e-10,
                "Mismatch at index {}: {} vs {}",
                i,
                orig,
                recon
            );
        }
    }

    #[test]
    fn test_dominant_frequency() {
        let mut ctx = make_ctx();

        let n = 100;
        let freq = 25.0;
        let sample_rate = 100.0;
        let data: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate).sin())
            .collect();

        let result = intrinsic_dominant_frequency(
            &[make_array(data), ValueWord::from_f64(sample_rate)],
            &mut ctx,
        )
        .unwrap();

        let vm_result = result.clone();
        assert_eq!(vm_result.type_name(), "object");
    }
}
