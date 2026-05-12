// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 3 sites
//     jit_box(HK_STRING, ...) — jit_typeof, jit_to_string, jit_type_check
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Type Conversion FFI Functions for JIT
//!
//! Functions for type checking and conversion in JIT-compiled code.

// jit_array::JitArray removed — see jit_array.rs SURFACE comment.
// Branches that walked array elements (`type_spec` of shape "array:...",
// "tuple:...") now return `false` rather than fabricating an iteration
// over a deleted heap layout.
use super::jit_kinds::*;
use super::value_ffi::*;

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
                // PHASE_2C / SURFACE (ADR-006 §2.7.4): pre-strict-typing
                // this walked `JitArray` elements and recursed. The
                // `JitArray` heap layout was deleted (see jit_array.rs
                // SURFACE); the strict-typing rebuild target reads
                // elements via `Arc<TypedArrayData>` per-element-kind
                // arms (§2.7.6/Q8). Until that lands, the kind check
                // is the array-shape check only — element-type
                // verification is dropped.
                let _ = rest;
                is_heap_kind(value_bits, HK_ARRAY)
            }
            "tuple" => {
                // Same SURFACE as `array` — the per-element check is
                // dropped pending the §2.7.6/Q8 rebuild.
                let _ = rest;
                is_heap_kind(value_bits, HK_ARRAY)
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

/// Format a JIT-stamped value as a string for display.
///
/// PHASE_2C / SURFACE (ADR-006 §2.7.4 / §2.7.5): pre-strict-typing
/// this function dispatched on `shape_value::tag_bits::is_tagged` /
/// `get_tag == TAG_INT` to decode i48 integer payloads from raw bits.
/// That `tag_bits` decode is exactly the deleted W-series shape
/// (CLAUDE.md "Forbidden Patterns": "Runtime tag_bits dispatch
/// (deleted)"), forbidden under any rebuild.
///
/// The strict-typing rebuild target is `(bits: u64, kind: NativeKind) ->
/// String` so the integer arm dispatches on `kind == NativeKind::Int64`
/// (or the i32/i16/i8 width variants) and reads the payload as a typed
/// scalar without tag decoding. Until callers thread `kind` through,
/// the integer branch is removed — JIT-emitted bytecode that lands a
/// scalar `Int*` here would have routed it through the typed
/// `RETURN_TAG_I64` / `RETURN_TAG_I32` path at `executor.rs:254`
/// already, so this fallback only sees heap-tagged values plus the
/// inline `TAG_NULL`/`TAG_BOOL_*` constants from `value_ffi`.
pub(crate) fn format_value_word(value_bits: u64) -> String {
    if is_number(value_bits) {
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
                // PHASE_2C / SURFACE (ADR-006 §2.7.4): the deleted
                // `JitArray` walk that produced "[a, b, c]" formatting
                // is gone. The strict-typing rebuild target dispatches
                // on the slot's `NativeKind::Ptr(HeapKind::TypedArray)`
                // per-element-kind arm via the §2.7.6/Q8 carrier.
                "[<array>]".to_string()
            }
            Some(HK_OK) => {
                let inner = unsafe { *jit_unbox::<u64>(value_bits) };
                format!("Ok({})", format_value_word(inner))
            }
            Some(HK_ERR) => {
                let inner = unsafe { *jit_unbox::<u64>(value_bits) };
                format!("Err({})", format_value_word(inner))
            }
            Some(HK_SOME) => {
                let inner = unsafe { *jit_unbox::<u64>(value_bits) };
                format!("Some({})", format_value_word(inner))
            }
            _ => "[object]".to_string(),
        }
    }
}

/// Print a ValueWord value to stdout with a newline.
///
/// W11-jit-new-array note: this is the kind-blind fallback retained for
/// receivers whose `NativeKind` the MIR-side print emitter could not
/// prove (heap pointers etc.). The kinded entry points
/// `jit_print_i64` / `jit_print_f64` / `jit_print_bool` below are the
/// §2.7.5 stamp-at-compile-time path the MIR emitter routes through
/// when the operand's slot kind is statically known. Future ADR-006
/// §2.7.5 follow-ups extend this to `jit_print_str` /
/// `jit_print_ptr<HeapKind::*>` per heap arm.
pub extern "C" fn jit_print(value_bits: u64) {
    println!("{}", format_value_word(value_bits));
}

/// Print a raw native i64 to stdout with a newline.
///
/// W11-jit-new-array (ADR-006 §2.7.5 / Q15 stamp-at-compile-time): the
/// MIR-side print emitter dispatches to this entry point whenever the
/// operand slot is proven `NativeKind::Int64` / `UInt64` / `IntSize` /
/// `UIntSize`. The value is the raw native integer, not a NaN-boxed
/// ValueWord — the kind-blind `jit_print` decoded raw int bits as a
/// denormal `f64` and displayed `0.000...208` for `print(42)`, which
/// was the §2.7.5 kind-source gap surfaced by smoke target 1.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_i64(value: i64) {
    println!("{}", value);
}

/// Print a raw native f64 to stdout with a newline.
///
/// W11-jit-new-array companion to `jit_print_i64`: dispatched when the
/// operand slot is proven `NativeKind::Float64`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_f64(value: f64) {
    if value.is_finite() && value == value.trunc() && value.abs() < 1e15 {
        println!("{}", value as i64);
    } else {
        println!("{}", value);
    }
}

/// Print a raw native bool to stdout with a newline.
///
/// W11-jit-new-array companion to `jit_print_i64`: dispatched when the
/// operand slot is proven `NativeKind::Bool`. The Cranelift I8 carrier
/// is widened to a `u8` (0 = false, nonzero = true) at the FFI
/// boundary.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_bool(value: u8) {
    println!("{}", value != 0);
}

/// Concatenate two NaN-boxed string values into a freshly boxed
/// `UnifiedString`. Used by the MIR-lowering path for `BinOp::Add` when both
/// operands have `NativeKind::String`.
///
/// ## Operand decoding
///
/// Handles both the legacy `JitAlloc<String>` and the unified-heap
/// `UnifiedString` string layouts by routing through
/// `value_ffi::unbox_string` (F0's fix) — callers may mix the two freely,
/// e.g. an f-string that interpolates a runtime-produced string captured
/// in a legacy-layout slot with a compile-time literal boxed through
/// `box_string` (unified).
///
/// If either operand is NOT an `HK_STRING` heap value, we format it via
/// `format_value_word`. This matches interpreter semantics for
/// `str + <anything>` — the MIR's `lower_formatted_string` emits
/// `BinaryOp::Add` on whatever an interpolation expression returns, so the
/// operand may legitimately be a number, bool, null, etc.
///
/// ## Return value
///
/// The result is a freshly allocated `UnifiedString` with refcount 1,
/// NaN-boxed via `box_string`. Neither input refcount is modified — the
/// caller's `emit_drop` handles the operand lifetimes per MIR ownership.
pub extern "C" fn jit_string_concat(a_bits: u64, b_bits: u64) -> u64 {
    use super::value_ffi::unbox_string;

    fn stringify(bits: u64) -> String {
        match heap_kind(bits) {
            Some(HK_STRING) => unsafe { unbox_string(bits) }.to_owned(),
            _ => format_value_word(bits),
        }
    }

    let mut out = stringify(a_bits);
    out.push_str(&stringify(b_bits));
    super::value_ffi::box_string(out)
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
