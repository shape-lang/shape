//! String operations
//!
//! Handles: split, join, contains, substring, replace

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Handle split(separator) - Split string into array
pub(crate) fn handle_split(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let string_val = args
        .get(0)
        .ok_or(VMError::StackUnderflow)?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;
    let sep = args
        .get(1)
        .ok_or_else(|| VMError::InvalidArgument {
            function: "split".to_string(),
            message: "requires a separator argument".to_string(),
        })?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;

    let parts: Vec<ValueWord> = string_val
        .split(sep)
        .map(|s| ValueWord::from_string(Arc::new(s.to_string())))
        .collect();

    Ok(ValueWord::from_array(Arc::new(parts)))
}

/// Handle join(array, separator) - Join array into string
/// Note: This is implemented as a string method but takes an array as the receiver
pub(crate) fn handle_join(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = args.get(0).ok_or(VMError::StackUnderflow)?;
    let arr = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    let sep = args
        .get(1)
        .ok_or_else(|| VMError::InvalidArgument {
            function: "join".to_string(),
            message: "requires a separator argument".to_string(),
        })?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;

    let strings: Result<Vec<String>, VMError> = arr
        .iter()
        .map(|nb| {
            if let Some(s) = nb.as_str() {
                Ok(s.to_string())
            } else if let Some(n) = nb.as_f64() {
                Ok(n.to_string())
            } else if let Some(i) = nb.as_i64() {
                Ok(i.to_string())
            } else if let Some(b) = nb.as_bool() {
                Ok(b.to_string())
            } else {
                Err(VMError::InvalidArgument {
                    function: "join".to_string(),
                    message: format!("cannot join non-stringable value: {}", nb.type_name()),
                })
            }
        })
        .collect();

    let result = strings?.join(sep);
    Ok(ValueWord::from_string(Arc::new(result)))
}

/// Handle contains(substring) - Check if string contains substring
pub(crate) fn handle_contains(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let string_val = args
        .get(0)
        .ok_or(VMError::StackUnderflow)?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;
    let substr = args
        .get(1)
        .ok_or_else(|| VMError::InvalidArgument {
            function: "contains".to_string(),
            message: "requires a substring argument".to_string(),
        })?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;

    let result = string_val.contains(substr);
    Ok(ValueWord::from_bool(result))
}

/// Handle substring(start, end) - Extract substring
pub(crate) fn handle_substring(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let string_val = args
        .get(0)
        .ok_or(VMError::StackUnderflow)?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;
    let start = args
        .get(1)
        .ok_or_else(|| VMError::InvalidArgument {
            function: "substring".to_string(),
            message: "requires a start argument".to_string(),
        })?
        .as_number_coerce()
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: "other",
        })? as usize;

    // Optional end parameter
    let result = if let Some(end_nb) = args.get(2) {
        let end = end_nb
            .as_number_coerce()
            .ok_or_else(|| VMError::TypeError {
                expected: "number",
                got: "other",
            })? as usize;
        let chars: Vec<char> = string_val.chars().collect();
        let end = end.min(chars.len());
        let start = start.min(end);
        chars[start..end].iter().collect::<String>()
    } else {
        let chars: Vec<char> = string_val.chars().collect();
        let start = start.min(chars.len());
        chars[start..].iter().collect::<String>()
    };

    Ok(ValueWord::from_string(Arc::new(result)))
}

/// Handle replace(old, new) - Replace all occurrences of substring
pub(crate) fn handle_replace(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let string_val = args
        .get(0)
        .ok_or(VMError::StackUnderflow)?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;
    let old = args
        .get(1)
        .ok_or_else(|| VMError::InvalidArgument {
            function: "replace".to_string(),
            message: "requires an old argument".to_string(),
        })?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;
    let new = args
        .get(2)
        .ok_or_else(|| VMError::InvalidArgument {
            function: "replace".to_string(),
            message: "requires a new argument".to_string(),
        })?
        .as_str()
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: "other",
        })?;

    let result = string_val.replace(old, new);
    Ok(ValueWord::from_string(Arc::new(result)))
}
