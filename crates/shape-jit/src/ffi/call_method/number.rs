// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     box_string(...) — toFixed, toString
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Number method implementations for JIT

use crate::nan_boxing::*;

/// Call a method on a number value
#[inline(always)]
pub fn call_number_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    let num = unbox_number(receiver_bits);
    match method_name {
        "abs" => box_number(num.abs()),
        "floor" => box_number(num.floor()),
        "ceil" => box_number(num.ceil()),
        "round" => box_number(num.round()),
        "sqrt" => box_number(num.sqrt()),
        "toFixed" | "to_fixed" => {
            let precision = if !args.is_empty() && is_number(args[0]) {
                unbox_number(args[0]) as usize
            } else {
                2
            };
            let formatted = format!("{:.prec$}", num, prec = precision);
            box_string(formatted)
        }
        "toString" | "to_string" => {
            let s = format!("{}", num);
            box_string(s)
        }
        _ => TAG_NULL,
    }
}
