// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 1 site
//     jit_box(HK_STRING, ...) — jit_format result
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! String Formatting Utilities
//!
//! Functions for formatting strings with template substitution.

use super::super::super::context::JITContext;
use super::super::super::nan_boxing::*;

// ============================================================================
// String Formatting
// ============================================================================

/// Format a string with arguments
#[inline(always)]
pub extern "C" fn jit_format(ctx: *mut JITContext, _arg_count: usize) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;

        // Pop arg_count value from stack first
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let arg_count_val = ctx_ref.stack[ctx_ref.stack_ptr];
        let arg_count = if is_number(arg_count_val) {
            unbox_number(arg_count_val) as usize
        } else {
            return TAG_NULL;
        };

        if arg_count == 0 {
            return TAG_NULL;
        }

        // Pop all arguments from stack
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if ctx_ref.stack_ptr == 0 {
                return TAG_NULL;
            }
            ctx_ref.stack_ptr -= 1;
            args.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        args.reverse(); // Restore original order

        // First arg is the format string
        let template_bits = args[0];
        if !is_heap_kind(template_bits, HK_STRING) {
            return TAG_NULL;
        }
        let template = unbox_string(template_bits).to_string();

        // Substitute placeholders
        let mut result = template;
        let format_args = &args[1..];

        // Handle {} placeholders (positional)
        let mut arg_idx = 0;
        while let Some(pos) = result.find("{}") {
            if arg_idx < format_args.len() {
                let replacement = value_to_string(format_args[arg_idx]);
                result = format!("{}{}{}", &result[..pos], replacement, &result[pos + 2..]);
                arg_idx += 1;
            } else {
                break;
            }
        }

        // Handle {0}, {1}, etc. (indexed placeholders)
        for i in 0..format_args.len() {
            let placeholder = format!("{{{}}}", i);
            if result.contains(&placeholder) {
                let replacement = value_to_string(format_args[i]);
                result = result.replace(&placeholder, &replacement);
            }
        }

        box_string(result)
    }
}

/// Helper to convert a value to string for format
pub(crate) fn value_to_string(bits: u64) -> String {
    if is_number(bits) {
        let n = unbox_number(bits);
        if n.fract() == 0.0 && n.abs() < 1e15 {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        }
    } else if bits == TAG_NULL {
        "null".to_string()
    } else if bits == TAG_BOOL_TRUE {
        "true".to_string()
    } else if bits == TAG_BOOL_FALSE {
        "false".to_string()
    } else {
        match heap_kind(bits) {
            Some(HK_STRING) => unsafe { unbox_string(bits) }.to_string(),
            Some(HK_ARRAY) => "[array]".to_string(),
            Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => "[object]".to_string(),
            _ => "[unknown]".to_string(),
        }
    }
}
