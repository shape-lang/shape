//! Column intrinsic functions

use super::{
    extract_f64, extract_f64_array, extract_usize, f64_vec_to_nb_array, i64_vec_to_nb_int_array,
    option_i64_vec_to_nb, try_extract_i64_slice,
};
use crate::context::ExecutionContext;
use crate::simd_i64;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;

pub fn intrinsic_column_select(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_series requires exactly 1 argument".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "series")?;
    Ok(f64_vec_to_nb_array(data))
}

pub fn intrinsic_shift(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "shift() requires 2 arguments (series, shift)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "shift")?;
    let shift = extract_f64(&args[1], "shift() second argument")? as i64;

    let mut result = Vec::with_capacity(data.len());

    if shift > 0 {
        let shift_usize = shift.min(data.len() as i64) as usize;
        for _ in 0..shift_usize {
            result.push(f64::NAN);
        }
        result.extend_from_slice(&data[..data.len().saturating_sub(shift_usize)]);
    } else if shift < 0 {
        let shift_usize = (-shift).min(data.len() as i64) as usize;
        result.extend_from_slice(&data[shift_usize..]);
        for _ in 0..shift_usize {
            result.push(f64::NAN);
        }
    } else {
        result.extend_from_slice(&data);
    }

    Ok(f64_vec_to_nb_array(result))
}

pub fn intrinsic_diff(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() || args.len() > 2 {
        return Err(ShapeError::RuntimeError {
            message: "diff() requires 1 or 2 arguments (series, [period])".to_string(),
            location: None,
        });
    }

    let period = if args.len() == 2 {
        extract_usize(&args[1], "diff() second argument")?
    } else {
        1
    };

    // i64 fast path
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        return Ok(option_i64_vec_to_nb(simd_i64::diff_i64(slice, period)));
    }

    let data = extract_f64_array(&args[0], "diff")?;

    let mut result = Vec::with_capacity(data.len());
    for i in 0..data.len() {
        if i < period {
            result.push(f64::NAN);
        } else {
            result.push(data[i] - data[i - period]);
        }
    }

    Ok(f64_vec_to_nb_array(result))
}

pub fn intrinsic_pct_change(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() || args.len() > 2 {
        return Err(ShapeError::RuntimeError {
            message: "pct_change() requires 1 or 2 arguments (series, [period])".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "pct_change")?;

    let period = if args.len() == 2 {
        extract_usize(&args[1], "pct_change() second argument")?
    } else {
        1
    };

    let mut result = Vec::with_capacity(data.len());
    for i in 0..data.len() {
        if i < period {
            result.push(f64::NAN);
        } else {
            let prev = data[i - period];
            if prev == 0.0 {
                result.push(f64::NAN);
            } else {
                result.push((data[i] - prev) / prev);
            }
        }
    }

    Ok(f64_vec_to_nb_array(result))
}

pub fn intrinsic_fillna(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "fillna() requires 2 arguments (series, value)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "fillna")?;
    let fill_value = extract_f64(&args[1], "fillna() second argument")?;

    let result: Vec<f64> = data
        .iter()
        .map(|&v| if v.is_nan() { fill_value } else { v })
        .collect();

    Ok(f64_vec_to_nb_array(result))
}

pub fn intrinsic_cumsum(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "cumsum() requires 1 argument (series)".to_string(),
            location: None,
        });
    }

    // i64 fast path
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        return Ok(i64_vec_to_nb_int_array(simd_i64::cumsum_i64(slice)));
    }

    let data = extract_f64_array(&args[0], "cumsum")?;
    let mut result = Vec::with_capacity(data.len());
    let mut acc = 0.0;
    for &v in &data {
        acc += v;
        result.push(acc);
    }

    Ok(f64_vec_to_nb_array(result))
}

pub fn intrinsic_cumprod(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 1 {
        return Err(ShapeError::RuntimeError {
            message: "cumprod() requires 1 argument (series)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "cumprod")?;
    let mut result = Vec::with_capacity(data.len());
    let mut acc = 1.0;
    for &v in &data {
        acc *= v;
        result.push(acc);
    }

    Ok(f64_vec_to_nb_array(result))
}

pub fn intrinsic_clip(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 3 {
        return Err(ShapeError::RuntimeError {
            message: "clip() requires 3 arguments (series, min, max)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "clip")?;
    let min = extract_f64(&args[1], "clip() second argument")?;
    let max = extract_f64(&args[2], "clip() third argument")?;

    let result: Vec<f64> = data.iter().map(|&v| v.max(min).min(max)).collect();

    Ok(f64_vec_to_nb_array(result))
}
