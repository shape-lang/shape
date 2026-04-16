//! Native method handlers for Range values.
//!
//! Range values (`start..end`, `start..=end`) support iteration via the
//! `Iterable` trait (handled in `iterator_methods.rs`) and property access
//! (handled in `property_access.rs`). This module provides dedicated
//! method handlers for Range-specific operations.
//!
//! Each handler operates on raw `u64` NaN-boxed bits:
//! - `args[0]` is the receiver (a Range)
//! - Returns a raw `u64` result

use crate::executor::objects::raw_helpers::{extract_number_coerce, extract_range, type_error};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapValue;
use shape_value::ValueWordExt;
use shape_value::value_word::*;
use shape_value::VMError;

/// range.contains(value) -- check if a numeric value is within the range.
pub fn range_contains(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = args[0];
    let (start, end, inclusive) = receiver
        .as_range()
        .ok_or_else(|| type_error("Range", receiver))?;

    let needle = extract_number_coerce(args[1])
        .ok_or_else(|| type_error("number or int", args[1]))?;

    let start_val = start
        .and_then(|s| extract_number_coerce(*s))
        .unwrap_or(f64::NEG_INFINITY);
    let end_val = end
        .and_then(|e| extract_number_coerce(*e))
        .unwrap_or(f64::INFINITY);

    let in_range = if inclusive {
        needle >= start_val && needle <= end_val
    } else {
        needle >= start_val && needle < end_val
    };

    Ok(vw_from_bool(in_range).raw_bits())
}

/// range.toArray() -- materialize the range into an array of integers.
pub fn range_to_array(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = args[0];

    if let Some((start, end, inclusive)) = extract_range(receiver)
    {
        let start_val = start
            .and_then(|s| s.as_i64())
            .unwrap_or(0);
        let end_val = end
            .and_then(|e| e.as_i64())
            .ok_or_else(|| {
                VMError::RuntimeError("Range.toArray() requires a finite end bound".to_string())
            })?;

        let mut result = Vec::new();
        if inclusive {
            for i in start_val..=end_val {
                result.push(vw_from_i64(i));
            }
        } else {
            for i in start_val..end_val {
                result.push(vw_from_i64(i));
            }
        }

        Ok(vw_from_array(std::sync::Arc::new(result)).into_raw_bits())
    } else {
        Err(type_error("Range", receiver))
    }
}
