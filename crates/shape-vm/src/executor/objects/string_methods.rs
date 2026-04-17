//! String operations
//!
//! V2 (MethodFnV2) handlers for all string methods.

use crate::executor::VirtualMachine;
use crate::executor::objects::raw_helpers;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// V2 string method handlers
// ═══════════════════════════════════════════════════════════════════════════

/// len / length
pub fn v2_string_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_i64(s.len() as i64).raw_bits())
}

/// toUpperCase / to_upper_case
pub fn v2_string_to_upper(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_string(Arc::new(s.to_uppercase())).raw_bits())
}

/// toLowerCase / to_lower_case
pub fn v2_string_to_lower(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_string(Arc::new(s.to_lowercase())).raw_bits())
}

/// trim
pub fn v2_string_trim(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_string(Arc::new(s.trim().to_string())).raw_bits())
}

/// trimStart / trim_start
pub fn v2_string_trim_start(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_string(Arc::new(s.trim_start().to_string())).raw_bits())
}

/// trimEnd / trim_end
pub fn v2_string_trim_end(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_string(Arc::new(s.trim_end().to_string())).raw_bits())
}

/// toString / to_string
pub fn v2_string_to_string(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Ok(args[0])
}

/// startsWith / starts_with
pub fn v2_string_starts_with(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let prefix = raw_helpers::extract_str(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "startsWith".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    Ok(ValueWord::from_bool(s.starts_with(prefix)).raw_bits())
}

/// endsWith / ends_with
pub fn v2_string_ends_with(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let suffix = raw_helpers::extract_str(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "endsWith".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    Ok(ValueWord::from_bool(s.ends_with(suffix)).raw_bits())
}

/// contains
pub fn v2_string_contains(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let needle = raw_helpers::extract_str(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "contains".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    Ok(ValueWord::from_bool(s.contains(needle)).raw_bits())
}

/// indexOf / index_of
pub fn v2_string_index_of(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let needle = raw_helpers::extract_str(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "indexOf".to_string(),
        message: "requires a string argument".to_string(),
    })?;
    let result = match s.find(needle) {
        Some(pos) => { s[..pos].chars().count() as i64 }
        None => -1,
    };
    Ok(ValueWord::from_i64(result).raw_bits())
}

/// repeat
pub fn v2_string_repeat(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let count = raw_helpers::extract_number_coerce(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "repeat".to_string(), message: "requires a count argument".to_string(),
    })? as usize;
    Ok(ValueWord::from_string(Arc::new(s.repeat(count))).raw_bits())
}

/// charAt / char_at
pub fn v2_string_char_at(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let index = raw_helpers::extract_number_coerce(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "charAt".to_string(), message: "requires an index argument".to_string(),
    })? as usize;
    let result = match s.chars().nth(index) { Some(c) => ValueWord::from_char(c), None => ValueWord::none() };
    Ok(result.raw_bits())
}

/// reverse
pub fn v2_string_reverse(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_string(Arc::new(s.chars().rev().collect::<String>())).raw_bits())
}

/// isDigit / is_digit
pub fn v2_string_is_digit(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())).raw_bits())
}

/// isAlpha / is_alpha
pub fn v2_string_is_alpha(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())).raw_bits())
}

/// isAscii / is_ascii
pub fn v2_string_is_ascii(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_bool(s.is_ascii()).raw_bits())
}

/// toInt / to_int
pub fn v2_string_to_int(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let parsed: i64 = s.trim().parse().map_err(|_| VMError::RuntimeError(format!("Cannot convert '{}' to int", s)))?;
    Ok(ValueWord::from_i64(parsed).raw_bits())
}

/// toNumber / to_number / toFloat / to_float
pub fn v2_string_to_number(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let parsed: f64 = s.trim().parse().map_err(|_| VMError::RuntimeError(format!("Cannot convert '{}' to number", s)))?;
    Ok(ValueWord::from_f64(parsed).raw_bits())
}

/// codePointAt / code_point_at
pub fn v2_string_code_point_at(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let index = raw_helpers::extract_number_coerce(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "codePointAt".to_string(), message: "requires an index argument".to_string(),
    })? as usize;
    let result = match s.chars().nth(index) { Some(c) => c as u32 as i64, None => -1 };
    Ok(ValueWord::from_i64(result).raw_bits())
}

/// graphemeLen / grapheme_len
pub fn v2_string_grapheme_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_segmentation::UnicodeSegmentation;
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    Ok(ValueWord::from_i64(s.graphemes(true).count() as i64).raw_bits())
}

/// padStart / pad_start
pub fn v2_string_pad_start(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let target_len = raw_helpers::extract_number_coerce(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "padStart".to_string(), message: "requires a length argument".to_string(),
    })? as usize;
    let fill = if args.len() > 2 { raw_helpers::extract_str(args[2]).map(|s| s.to_string()).unwrap_or_else(|| " ".to_string()) } else { " ".to_string() };
    let char_count = s.chars().count();
    if char_count >= target_len {
        Ok(ValueWord::from_string(Arc::new(s.to_string())).raw_bits())
    } else {
        let pad_needed = target_len - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        let mut padding = String::with_capacity(pad_needed + s.len());
        for i in 0..pad_needed { padding.push(fill_chars[i % fill_chars.len()]); }
        padding.push_str(s);
        Ok(ValueWord::from_string(Arc::new(padding)).raw_bits())
    }
}

/// padEnd / pad_end
pub fn v2_string_pad_end(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let target_len = raw_helpers::extract_number_coerce(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "padEnd".to_string(), message: "requires a length argument".to_string(),
    })? as usize;
    let fill = if args.len() > 2 { raw_helpers::extract_str(args[2]).map(|s| s.to_string()).unwrap_or_else(|| " ".to_string()) } else { " ".to_string() };
    let char_count = s.chars().count();
    if char_count >= target_len {
        Ok(ValueWord::from_string(Arc::new(s.to_string())).raw_bits())
    } else {
        let pad_needed = target_len - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        let mut result = s.to_string();
        for i in 0..pad_needed { result.push(fill_chars[i % fill_chars.len()]); }
        Ok(ValueWord::from_string(Arc::new(result)).raw_bits())
    }
}

/// split
pub fn v2_string_split(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let sep = raw_helpers::extract_str(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "split".to_string(), message: "requires a separator argument".to_string(),
    })?;
    let parts: Vec<ValueWord> = s.split(sep).map(|part| ValueWord::from_string(Arc::new(part.to_string()))).collect();
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(parts)).raw_bits())
}

/// replace
pub fn v2_string_replace(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let old = raw_helpers::extract_str(args[1]).ok_or_else(|| VMError::InvalidArgument {
        function: "replace".to_string(), message: "requires an old argument".to_string(),
    })?;
    let new = raw_helpers::extract_str(args[2]).ok_or_else(|| VMError::InvalidArgument {
        function: "replace".to_string(), message: "requires a new argument".to_string(),
    })?;
    Ok(ValueWord::from_string(Arc::new(s.replace(old, new))).raw_bits())
}

/// substring
pub fn v2_string_substring(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let start = raw_helpers::extract_number_coerce(args[1]).ok_or_else(|| VMError::TypeError { expected: "number", got: "other" })? as usize;
    let result = if args.len() > 2 {
        let end = raw_helpers::extract_number_coerce(args[2]).ok_or_else(|| VMError::TypeError { expected: "number", got: "other" })? as usize;
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

/// join
pub fn v2_string_join(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let arr = raw_helpers::extract_any_array(args[0])
        .ok_or_else(|| raw_helpers::type_error("array", args[0]))?.to_generic();
    let sep = raw_helpers::extract_str(args[1])
        .ok_or_else(|| raw_helpers::type_error("string", args[1]))?;
    let strings: Result<Vec<String>, VMError> = arr.iter().map(|nb| {
        if let Some(s) = nb.as_str() { Ok(s.to_string()) }
        else if let Some(n) = nb.as_f64() { Ok(n.to_string()) }
        else if let Some(i) = nb.as_i64() { Ok(i.to_string()) }
        else if let Some(b) = nb.as_bool() { Ok(b.to_string()) }
        else { Err(VMError::InvalidArgument { function: "join".to_string(), message: format!("cannot join non-stringable value: {}", nb.type_name()) }) }
    }).collect();
    Ok(ValueWord::from_string(Arc::new(strings?.join(sep))).raw_bits())
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string methods: graphemes, normalize, iter
// ═══════════════════════════════════════════════════════════════════════════

/// graphemes
pub fn v2_string_graphemes(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_segmentation::UnicodeSegmentation;
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let clusters: Vec<ValueWord> = s.graphemes(true).map(|g| ValueWord::from_string(Arc::new(g.to_string()))).collect();
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(clusters)).into_raw_bits())
}

/// normalize
pub fn v2_string_normalize(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_normalization::UnicodeNormalization;
    let s = raw_helpers::extract_str(args[0])
        .ok_or_else(|| raw_helpers::type_error("string", args[0]))?;
    let form_bits = args.get(1).copied().ok_or_else(|| VMError::InvalidArgument {
        function: "normalize".to_string(), message: "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")".to_string(),
    })?;
    let form = raw_helpers::extract_str(form_bits).ok_or_else(|| VMError::InvalidArgument {
        function: "normalize".to_string(), message: "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")".to_string(),
    })?;
    let normalized: String = match form {
        "NFC" => s.nfc().collect(), "NFD" => s.nfd().collect(),
        "NFKC" => s.nfkc().collect(), "NFKD" => s.nfkd().collect(),
        _ => return Err(VMError::InvalidArgument { function: "normalize".to_string(), message: format!("unknown normalization form '{}', expected NFC/NFD/NFKC/NFKD", form) }),
    };
    Ok(ValueWord::from_string(Arc::new(normalized)).into_raw_bits())
}

/// iter
pub fn v2_string_iter(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use shape_value::heap_value::IteratorState;
    let receiver = unsafe { ValueWord::clone_from_bits(args[0]) };
    Ok(ValueWord::from_iterator(Box::new(IteratorState {
        source: receiver, position: 0, transforms: vec![], done: false,
    })).into_raw_bits())
}
