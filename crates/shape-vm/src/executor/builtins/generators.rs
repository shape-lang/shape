//! Value generation builtin implementations
//!
//! Handles: range, slice

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

impl VirtualMachine {
    /// Range: Generate array of numbers
    /// Supports 1, 2, or 3 arguments:
    /// - range(n) => [0, 1, 2, ..., n-1]
    /// - range(start, end) => [start, start+1, ..., end-1]
    /// - range(start, end, step) => [start, start+step, ..., end-step]
    pub(in crate::executor) fn builtin_range(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        // Check if all arguments are integers — if so, produce Int output
        let all_int = args.iter().all(|a| a.is_i64());

        if all_int {
            return self.builtin_range_int_nb(&args);
        }

        let (start, end, step) = match args.len() {
            1 => {
                let n = args[0].as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("range() argument must be a number".to_string())
                })?;
                (0.0, n, 1.0)
            }
            2 => {
                let start = args[0].as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("range() start must be a number".to_string())
                })?;
                let end = args[1].as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("range() end must be a number".to_string())
                })?;
                (start, end, 1.0)
            }
            3 => {
                let start = args[0].as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("range() start must be a number".to_string())
                })?;
                let end = args[1].as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("range() end must be a number".to_string())
                })?;
                let step = args[2].as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError("range() step must be a number".to_string())
                })?;
                if step == 0.0 {
                    return Err(VMError::RuntimeError(
                        "range() step cannot be zero".to_string(),
                    ));
                }
                (start, end, step)
            }
            _ => {
                return Err(VMError::RuntimeError(
                    "range() requires 1, 2, or 3 arguments".to_string(),
                ));
            }
        };

        let mut values: Vec<ValueWord> = Vec::new();
        if step > 0.0 {
            let mut current = start;
            while current < end {
                values.push(ValueWord::from_f64(current));
                current += step;
            }
        } else {
            let mut current = start;
            while current > end {
                values.push(ValueWord::from_f64(current));
                current += step;
            }
        }

        Ok(ValueWord::from_array(shape_value::vmarray_from_vec(values)))
    }

    /// Integer-only range: produces ValueWord::Int values when all args are Int
    fn builtin_range_int_nb(&mut self, args: &[ValueWord]) -> Result<ValueWord, VMError> {
        let (start, end, step) = match args.len() {
            1 => {
                let n = args[0].as_i64().unwrap();
                (0i64, n, 1i64)
            }
            2 => {
                let s = args[0].as_i64().unwrap();
                let e = args[1].as_i64().unwrap();
                (s, e, 1i64)
            }
            3 => {
                let s = args[0].as_i64().unwrap();
                let e = args[1].as_i64().unwrap();
                let st = args[2].as_i64().unwrap();
                if st == 0 {
                    return Err(VMError::RuntimeError(
                        "range() step cannot be zero".to_string(),
                    ));
                }
                (s, e, st)
            }
            _ => {
                return Err(VMError::RuntimeError(
                    "range() requires 1, 2, or 3 arguments".to_string(),
                ));
            }
        };

        let mut values: Vec<ValueWord> = Vec::new();
        if step > 0 {
            let mut current = start;
            while current < end {
                values.push(ValueWord::from_i64(current));
                current += step;
            }
        } else {
            let mut current = start;
            while current > end {
                values.push(ValueWord::from_i64(current));
                current += step;
            }
        }

        Ok(ValueWord::from_array(shape_value::vmarray_from_vec(values)))
    }

    /// Slice: Extract subarray from start to end indices
    pub(in crate::executor) fn builtin_slice(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(VMError::RuntimeError(
                "slice() requires 2 or 3 arguments (array, start, [end])".to_string(),
            ));
        }

        let array = args[0]
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("slice() first argument must be an array".to_string())
            })?
            .to_generic();

        let start = args[1]
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("slice() start must be a number".to_string()))?
            as isize;

        let end = if args.len() == 3 {
            args[2]
                .as_number_coerce()
                .ok_or_else(|| VMError::RuntimeError("slice() end must be a number".to_string()))?
                as isize
        } else {
            array.len() as isize
        };

        let len = array.len() as isize;
        let start_idx = if start < 0 {
            (len + start).max(0) as usize
        } else {
            start.min(len) as usize
        };

        let end_idx = if end < 0 {
            (len + end).max(0) as usize
        } else {
            end.min(len) as usize
        };

        let sliced = if start_idx <= end_idx {
            array[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        };

        Ok(ValueWord::from_array(shape_value::vmarray_from_vec(sliced)))
    }
}
