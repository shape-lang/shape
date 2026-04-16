//! Recurrence intrinsics
//! Optimized linear recurrence relations for recursive indicators (EMA, etc.)

use super::{extract_f64, extract_f64_array, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};

/// Intrinsic: Linear Recurrence
///
/// Computes y[t] = y[t-1] * decay + input[t]
pub fn intrinsic_linear_recurrence(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_linear_recurrence requires 2 or 3 arguments".to_string(),
            location: None,
        });
    }

    let decay = extract_f64(&args[1], "Decay factor")?;

    let initial_value = if args.len() == 3 {
        if args[2].is_none() {
            None
        } else {
            Some(extract_f64(&args[2], "Initial value")?)
        }
    } else {
        None
    };

    let data = extract_f64_array(&args[0], "Input array")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    let mut result = Vec::with_capacity(data.len());

    if let Some(init) = initial_value {
        let mut prev = init;
        for &val in &data {
            let curr = prev * decay + val;
            result.push(curr);
            prev = curr;
        }
    } else {
        let first = data[0];
        result.push(first);
        let mut prev = first;
        for &val in &data[1..] {
            let curr = prev * decay + val;
            result.push(curr);
            prev = curr;
        }
    }

    Ok(f64_vec_to_nb_array(result))
}
