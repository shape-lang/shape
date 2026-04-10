//! String operations
//!
//! Handles: split, join, contains, substring, replace
//! Plus v2 (MethodFnV2) handlers for all non-closure string methods.

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// V2 helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow a ValueWord from raw u64 bits without taking ownership.
/// ManuallyDrop prevents double-free since dispatch_method_handler owns the original.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string method handlers
// ═══════════════════════════════════════════════════════════════════════════

/// len / length
pub fn v2_string_len(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_i64(s.len() as i64).raw_bits())
}

/// toUpperCase / to_upper_case
pub fn v2_string_to_upper(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_string(Arc::new(s.to_uppercase())).raw_bits())
}

/// toLowerCase / to_lower_case
pub fn v2_string_to_lower(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_string(Arc::new(s.to_lowercase())).raw_bits())
}

/// trim
pub fn v2_string_trim(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_string(Arc::new(s.trim().to_string())).raw_bits())
}

/// trimStart / trim_start
pub fn v2_string_trim_start(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_string(Arc::new(s.trim_start().to_string())).raw_bits())
}

/// trimEnd / trim_end
pub fn v2_string_trim_end(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_string(Arc::new(s.trim_end().to_string())).raw_bits())
}

/// toString / to_string
pub fn v2_string_to_string(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // Identity: return the receiver unchanged
    Ok(args[0])
}

/// startsWith / starts_with — args[1] is the prefix
pub fn v2_string_starts_with(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let prefix = vw1.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "startsWith".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    Ok(ValueWord::from_bool(s.starts_with(prefix)).raw_bits())
}

/// endsWith / ends_with — args[1] is the suffix
pub fn v2_string_ends_with(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let suffix = vw1.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "endsWith".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    Ok(ValueWord::from_bool(s.ends_with(suffix)).raw_bits())
}

/// contains — args[1] is the needle
pub fn v2_string_contains(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let needle = vw1.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "contains".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    Ok(ValueWord::from_bool(s.contains(needle)).raw_bits())
}

/// indexOf / index_of — args[1] is the needle
pub fn v2_string_index_of(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let needle = vw1.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "indexOf".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    let result = match s.find(needle) {
        Some(pos) => {
            // Return char index, not byte index
            let char_idx = s[..pos].chars().count() as i64;
            char_idx
        }
        None => -1,
    };
    Ok(ValueWord::from_i64(result).raw_bits())
}

/// repeat — args[1] is count
pub fn v2_string_repeat(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let count = vw1.as_number_coerce().ok_or_else(|| VMError::InvalidArgument {
        function: "repeat".to_string(),
        message: "requires a count argument".to_string(),
    })? as usize;
    Ok(ValueWord::from_string(Arc::new(s.repeat(count))).raw_bits())
}

/// charAt / char_at — args[1] is index
pub fn v2_string_char_at(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let index = vw1.as_number_coerce().ok_or_else(|| VMError::InvalidArgument {
        function: "charAt".to_string(),
        message: "requires an index argument".to_string(),
    })? as usize;
    let result = match s.chars().nth(index) {
        Some(c) => ValueWord::from_char(c),
        None => ValueWord::none(),
    };
    Ok(result.raw_bits())
}

/// reverse
pub fn v2_string_reverse(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let reversed: String = s.chars().rev().collect();
    Ok(ValueWord::from_string(Arc::new(reversed)).raw_bits())
}

/// isDigit / is_digit
pub fn v2_string_is_digit(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())).raw_bits())
}

/// isAlpha / is_alpha
pub fn v2_string_is_alpha(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())).raw_bits())
}

/// isAscii / is_ascii
pub fn v2_string_is_ascii(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_bool(s.is_ascii()).raw_bits())
}

/// toInt / to_int
pub fn v2_string_to_int(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let trimmed = s.trim();
    let parsed: i64 = trimmed.parse().map_err(|_| {
        VMError::RuntimeError(format!("Cannot convert '{}' to int", s))
    })?;
    Ok(ValueWord::from_i64(parsed).raw_bits())
}

/// toNumber / to_number / toFloat / to_float
pub fn v2_string_to_number(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let trimmed = s.trim();
    let parsed: f64 = trimmed.parse().map_err(|_| {
        VMError::RuntimeError(format!("Cannot convert '{}' to number", s))
    })?;
    Ok(ValueWord::from_f64(parsed).raw_bits())
}

/// codePointAt / code_point_at — args[1] is index
pub fn v2_string_code_point_at(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let index = vw1.as_number_coerce().ok_or_else(|| VMError::InvalidArgument {
        function: "codePointAt".to_string(),
        message: "requires an index argument".to_string(),
    })? as usize;
    let result = match s.chars().nth(index) {
        Some(c) => c as u32 as i64,
        None => -1,
    };
    Ok(ValueWord::from_i64(result).raw_bits())
}

/// graphemeLen / grapheme_len
pub fn v2_string_grapheme_len(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_segmentation::UnicodeSegmentation;
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let count = s.graphemes(true).count();
    Ok(ValueWord::from_i64(count as i64).raw_bits())
}

/// padStart / pad_start — args[1] is target_len, args[2] is optional fill
pub fn v2_string_pad_start(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let target_len = vw1.as_number_coerce().ok_or_else(|| VMError::InvalidArgument {
        function: "padStart".to_string(),
        message: "requires a length argument".to_string(),
    })? as usize;
    let fill = if args.len() > 2 {
        let vw2 = borrow_vw(args[2]);
        vw2.as_str().map(|s| s.to_string()).unwrap_or_else(|| " ".to_string())
    } else {
        " ".to_string()
    };
    let char_count = s.chars().count();
    if char_count >= target_len {
        Ok(ValueWord::from_string(Arc::new(s.to_string())).raw_bits())
    } else {
        let pad_needed = target_len - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        let mut padding = String::with_capacity(pad_needed + s.len());
        for i in 0..pad_needed {
            padding.push(fill_chars[i % fill_chars.len()]);
        }
        padding.push_str(s);
        Ok(ValueWord::from_string(Arc::new(padding)).raw_bits())
    }
}

/// padEnd / pad_end — args[1] is target_len, args[2] is optional fill
pub fn v2_string_pad_end(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let target_len = vw1.as_number_coerce().ok_or_else(|| VMError::InvalidArgument {
        function: "padEnd".to_string(),
        message: "requires a length argument".to_string(),
    })? as usize;
    let fill = if args.len() > 2 {
        let vw2 = borrow_vw(args[2]);
        vw2.as_str().map(|s| s.to_string()).unwrap_or_else(|| " ".to_string())
    } else {
        " ".to_string()
    };
    let char_count = s.chars().count();
    if char_count >= target_len {
        Ok(ValueWord::from_string(Arc::new(s.to_string())).raw_bits())
    } else {
        let pad_needed = target_len - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        let mut result = s.to_string();
        for i in 0..pad_needed {
            result.push(fill_chars[i % fill_chars.len()]);
        }
        Ok(ValueWord::from_string(Arc::new(result)).raw_bits())
    }
}

/// split — args[1] is delimiter
pub fn v2_string_split(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let sep = vw1.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "split".to_string(),
        message: "requires a separator argument".to_string(),
    })?;
    let parts: Vec<ValueWord> = s
        .split(sep)
        .map(|part| ValueWord::from_string(Arc::new(part.to_string())))
        .collect();
    Ok(ValueWord::from_array(Arc::new(parts)).raw_bits())
}

/// replace — args[1] is from, args[2] is to
pub fn v2_string_replace(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let old = vw1.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "replace".to_string(),
        message: "requires an old argument".to_string(),
    })?;
    let vw2 = borrow_vw(args[2]);
    let new = vw2.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "replace".to_string(),
        message: "requires a new argument".to_string(),
    })?;
    let result = s.replace(old, new);
    Ok(ValueWord::from_string(Arc::new(result)).raw_bits())
}

/// substring — args[1] is start, args[2] is optional end
pub fn v2_string_substring(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let vw1 = borrow_vw(args[1]);
    let start = vw1.as_number_coerce().ok_or_else(|| VMError::TypeError {
        expected: "number",
        got: "other",
    })? as usize;

    let result = if args.len() > 2 {
        let vw2 = borrow_vw(args[2]);
        let end = vw2.as_number_coerce().ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: "other",
        })? as usize;
        let chars: Vec<char> = s.chars().collect();
        let end = end.min(chars.len());
        let start = start.min(end);
        chars[start..end].iter().collect::<String>()
    } else {
        let chars: Vec<char> = s.chars().collect();
        let start = start.min(chars.len());
        chars[start..].iter().collect::<String>()
    };

    Ok(ValueWord::from_string(Arc::new(result)).raw_bits())
}

/// join — receiver (args[0]) is an array, args[1] is optional separator
pub fn v2_string_join(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: vw.type_name(),
        })?
        .to_generic();

    let vw1 = borrow_vw(args[1]);
    let sep = vw1.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw1.type_name(),
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
    Ok(ValueWord::from_string(Arc::new(result)).raw_bits())
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string methods: graphemes, normalize, iter
// ═══════════════════════════════════════════════════════════════════════════

/// graphemes — returns array of grapheme clusters
pub fn v2_string_graphemes(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_segmentation::UnicodeSegmentation;
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let clusters: Vec<ValueWord> = s
        .graphemes(true)
        .map(|g| ValueWord::from_string(Arc::new(g.to_string())))
        .collect();
    Ok(ValueWord::from_array(Arc::new(clusters)).into_raw_bits())
}

/// normalize — Unicode normalization (NFC, NFD, NFKC, NFKD)
pub fn v2_string_normalize(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_normalization::UnicodeNormalization;
    let vw = borrow_vw(args[0]);
    let s = vw.as_str().ok_or_else(|| VMError::TypeError {
        expected: "string",
        got: vw.type_name(),
    })?;
    let form_vw = borrow_vw(args.get(1).copied().ok_or_else(|| VMError::InvalidArgument {
        function: "normalize".to_string(),
        message: "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")".to_string(),
    })?);
    let form = form_vw.as_str().ok_or_else(|| VMError::InvalidArgument {
        function: "normalize".to_string(),
        message: "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")".to_string(),
    })?;
    let normalized: String = match form {
        "NFC" => s.nfc().collect(),
        "NFD" => s.nfd().collect(),
        "NFKC" => s.nfkc().collect(),
        "NFKD" => s.nfkd().collect(),
        _ => {
            return Err(VMError::InvalidArgument {
                function: "normalize".to_string(),
                message: format!(
                    "unknown normalization form '{}', expected NFC/NFD/NFKC/NFKD",
                    form
                ),
            });
        }
    };
    Ok(ValueWord::from_string(Arc::new(normalized)).into_raw_bits())
}

/// iter — returns an Iterator over chars of the string
pub fn v2_string_iter(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use shape_value::heap_value::IteratorState;
    let receiver = (*borrow_vw(args[0])).clone();
    let result = ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver,
        position: 0,
        transforms: vec![],
        done: false,
    }));
    Ok(result.into_raw_bits())
}

// ═══════════════════════════════════════════════════════════════════════════
// Legacy MethodFn string handlers (kept for backward compatibility)
// ═══════════════════════════════════════════════════════════════════════════

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
