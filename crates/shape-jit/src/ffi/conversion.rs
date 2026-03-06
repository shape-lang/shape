// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 3 sites
//     jit_box(HK_STRING, ...) — jit_typeof, jit_to_string, jit_type_check
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Type Conversion FFI Functions for JIT
//!
//! Functions for type checking and conversion in JIT-compiled code.

use super::super::jit_array::JitArray;
use super::super::nan_boxing::*;

// ============================================================================
// Type Checking
// ============================================================================

/// Get typeof a value as a string
pub extern "C" fn jit_typeof(value_bits: u64) -> u64 {
    let type_str = if is_number(value_bits) {
        "number"
    } else if value_bits == TAG_NULL {
        "null"
    } else if value_bits == TAG_BOOL_TRUE || value_bits == TAG_BOOL_FALSE {
        "boolean"
    } else if is_ok_tag(value_bits) || is_err_tag(value_bits) {
        "result"
    } else if is_inline_function(value_bits) {
        "function"
    } else {
        match heap_kind(value_bits) {
            Some(HK_STRING) => "string",
            Some(HK_ARRAY) => "array",
            Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => "object",
            Some(HK_CLOSURE) => "function",
            Some(HK_RANGE) => "range",
            Some(HK_COLUMN_REF) => "series",
            Some(HK_JIT_TABLE_REF) => "series_ref",
            Some(HK_DURATION) => "duration",
            Some(HK_TIME) => "time",
            Some(HK_TIMEFRAME) => "timeframe",
            _ => "unknown",
        }
    };
    jit_box(HK_STRING, type_str.to_string())
}

// ============================================================================
// Type Conversion
// ============================================================================

/// Convert value to string
pub extern "C" fn jit_to_string(value_bits: u64) -> u64 {
    let s = if is_number(value_bits) {
        format!("{}", unbox_number(value_bits))
    } else if value_bits == TAG_NULL {
        "null".to_string()
    } else if value_bits == TAG_BOOL_TRUE {
        "true".to_string()
    } else if value_bits == TAG_BOOL_FALSE {
        "false".to_string()
    } else {
        match heap_kind(value_bits) {
            Some(HK_STRING) => {
                let s = unsafe { jit_unbox::<String>(value_bits) };
                s.clone()
            }
            Some(HK_ARRAY) => "[array]".to_string(),
            Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => "[object]".to_string(),
            _ => "[unknown]".to_string(),
        }
    };
    jit_box(HK_STRING, s)
}

/// Check if a value matches a type (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
/// type_name_bits should be a boxed string pointer with encoded type info
pub extern "C" fn jit_type_check(value_bits: u64, type_name_bits: u64) -> u64 {
    // Get type name string
    let type_name = unsafe {
        if !is_heap_kind(type_name_bits, HK_STRING) {
            return TAG_BOOL_FALSE;
        }
        jit_unbox::<String>(type_name_bits).clone()
    };

    let matches = check_type_recursive(value_bits, &type_name);

    if matches {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Recursive helper to check encoded type strings
fn check_type_recursive(value_bits: u64, type_spec: &str) -> bool {
    // Parse type spec: "prefix:content" or just "typename"
    if let Some((prefix, rest)) = type_spec.split_once(':') {
        match prefix {
            "basic" => check_basic_type(value_bits, rest),
            "optional" => {
                // Optional: null matches, or inner type matches
                value_bits == TAG_NULL || check_type_recursive(value_bits, rest)
            }
            "array" => {
                // Array type: check if value is array, then optionally check element types
                if !is_heap_kind(value_bits, HK_ARRAY) {
                    return false;
                }
                let arr = unsafe { jit_unbox::<JitArray>(value_bits) };
                arr.iter().all(|elem| check_type_recursive(*elem, rest))
            }
            "tuple" => {
                // Tuple type: check array with specific element types
                if !is_heap_kind(value_bits, HK_ARRAY) {
                    return false;
                }
                let types: Vec<&str> = rest.split(',').collect();
                let arr = unsafe { jit_unbox::<JitArray>(value_bits) };
                if arr.len() != types.len() {
                    return false;
                }
                arr.iter()
                    .zip(types.iter())
                    .all(|(elem, ty)| check_type_recursive(*elem, ty))
            }
            "generic" => {
                // Generic like Array<T> - check base type only (don't verify element types)
                match rest {
                    "Array" => is_heap_kind(value_bits, HK_ARRAY),
                    "Series" => is_heap_kind(value_bits, HK_COLUMN_REF),
                    _ => false,
                }
            }
            "ref" => {
                // Reference types - not fully supported in JIT yet
                false
            }
            "dyn" => {
                // Dyn trait types - not fully supported in JIT yet
                false
            }
            _ => false,
        }
    } else {
        // No prefix, treat as direct type match
        match type_spec {
            "function" => is_inline_function(value_bits) || is_heap_kind(value_bits, HK_CLOSURE),
            "object" => is_heap_kind(value_bits, HK_TYPED_OBJECT),
            "any" => true,
            "void" => value_bits == TAG_UNIT,
            "never" => false,
            "null" => value_bits == TAG_NULL,
            "undefined" => value_bits == TAG_NULL || value_bits == TAG_UNIT,
            "unknown" => false,
            _ => check_basic_type(value_bits, type_spec),
        }
    }
}

/// Check a basic type name against a value
fn check_basic_type(value_bits: u64, type_name: &str) -> bool {
    if is_number(value_bits) {
        return type_name == "number";
    }
    if value_bits == TAG_NULL {
        return type_name == "null";
    }
    if value_bits == TAG_BOOL_TRUE || value_bits == TAG_BOOL_FALSE {
        return type_name == "boolean" || type_name == "bool";
    }
    if value_bits == TAG_UNIT {
        return type_name == "void" || type_name == "unit";
    }
    if is_inline_function(value_bits) {
        return type_name == "function";
    }
    if is_data_row(value_bits) {
        return type_name == "data_row";
    }
    if is_ok_tag(value_bits) || is_err_tag(value_bits) {
        return type_name == "result";
    }

    match heap_kind(value_bits) {
        Some(HK_STRING) => type_name == "string",
        Some(HK_ARRAY) => type_name == "array",
        Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => type_name == "object",
        Some(HK_CLOSURE) => type_name == "function",
        Some(HK_COLUMN_REF) => type_name == "series",
        Some(HK_TIME) => type_name == "time",
        Some(HK_DURATION) => type_name == "duration",
        Some(HK_TIMEFRAME) => type_name == "timeframe",
        Some(HK_RANGE) => type_name == "range",
        _ => false,
    }
}

/// Print a NaN-boxed value to stdout with a newline
pub extern "C" fn jit_print(value_bits: u64) {
    let s = if is_number(value_bits) {
        let n = unbox_number(value_bits);
        if n.is_finite() && n == n.trunc() && n.abs() < 1e15 {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        }
    } else if value_bits == TAG_BOOL_TRUE {
        "true".to_string()
    } else if value_bits == TAG_BOOL_FALSE {
        "false".to_string()
    } else if value_bits == TAG_NULL {
        "null".to_string()
    } else {
        match heap_kind(value_bits) {
            Some(HK_STRING) => {
                let s = unsafe { jit_unbox::<String>(value_bits) };
                s.clone()
            }
            Some(HK_ARRAY) => {
                let arr = unsafe { jit_unbox::<JitArray>(value_bits) };
                let elems: Vec<String> = arr
                    .iter()
                    .map(|&bits| {
                        if is_number(bits) {
                            let n = unbox_number(bits);
                            if n.is_finite() && n == n.trunc() && n.abs() < 1e15 {
                                format!("{}", n as i64)
                            } else {
                                format!("{}", n)
                            }
                        } else {
                            "[value]".to_string()
                        }
                    })
                    .collect();
                format!("[{}]", elems.join(", "))
            }
            _ => "[object]".to_string(),
        }
    };
    println!("{}", s);
}

/// Convert value to number
pub extern "C" fn jit_to_number(value_bits: u64) -> u64 {
    if is_number(value_bits) {
        return value_bits;
    }

    if value_bits == TAG_NULL {
        return box_number(0.0);
    }
    if value_bits == TAG_BOOL_TRUE {
        return box_number(1.0);
    }
    if value_bits == TAG_BOOL_FALSE {
        return box_number(0.0);
    }

    let num = match heap_kind(value_bits) {
        Some(HK_STRING) => {
            let s = unsafe { jit_unbox::<String>(value_bits) };
            s.parse::<f64>().unwrap_or(f64::NAN)
        }
        _ => f64::NAN,
    };
    box_number(num)
}
