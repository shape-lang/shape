// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 18 sites
//     box_string(...) — toUpperCase, toLowerCase, trim, replace,
//     charAt, substring, concat, padStart, padEnd, repeat, trimStart, trimEnd
//     jit_box(HK_ARRAY, ...) — split, chars
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 2 sites (split, chars)
//!
//! String method implementations for JIT

use crate::jit_array::JitArray;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

/// Call a method on a string value
#[inline(always)]
pub fn call_string_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    unsafe {
        let s = unbox_string(receiver_bits);

        match method_name {
            "length" | "len" => box_number(s.len() as f64),
            "toUpperCase" | "to_upper_case" => box_string(s.to_uppercase()),
            "toLowerCase" | "to_lower_case" => box_string(s.to_lowercase()),
            "trim" => box_string(s.trim().to_string()),
            "split" => {
                let separator = if !args.is_empty() && is_heap_kind(args[0], HK_STRING) {
                    unbox_string(args[0])
                } else {
                    " "
                };

                // AUDIT(C2): heap island — each split part is jit_box'd into a
                // JitAlloc<String> and stored as raw u64 in the Vec. These inner
                // string allocations escape into the JitArray element buffer without
                // GC tracking. When GC feature enabled, route through gc_allocator.
                let parts: Vec<u64> = s
                    .split(separator)
                    .map(|part| box_string(part.to_string()))
                    .collect();

                JitArray::from_vec(parts).heap_box()
            }
            "includes" | "contains" => {
                if args.is_empty() {
                    return TAG_BOOL_FALSE;
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let needle = unbox_string(args[0]);
                    if s.contains(needle) {
                        return TAG_BOOL_TRUE;
                    }
                }
                TAG_BOOL_FALSE
            }
            "startsWith" | "starts_with" => {
                if args.is_empty() {
                    return TAG_BOOL_FALSE;
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let prefix = unbox_string(args[0]);
                    if s.starts_with(prefix) {
                        return TAG_BOOL_TRUE;
                    }
                }
                TAG_BOOL_FALSE
            }
            "endsWith" | "ends_with" => {
                if args.is_empty() {
                    return TAG_BOOL_FALSE;
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let suffix = unbox_string(args[0]);
                    if s.ends_with(suffix) {
                        return TAG_BOOL_TRUE;
                    }
                }
                TAG_BOOL_FALSE
            }
            "replace" | "replaceAll" | "replace_all" => {
                if args.len() < 2 {
                    return receiver_bits;
                }
                if is_heap_kind(args[0], HK_STRING) && is_heap_kind(args[1], HK_STRING) {
                    let from = unbox_string(args[0]);
                    let to = unbox_string(args[1]);
                    let replaced = s.replace(from, to);
                    return box_string(replaced);
                }
                receiver_bits
            }
            "charAt" | "char_at" => {
                if args.is_empty() {
                    return box_string(String::new());
                }
                if is_number(args[0]) {
                    let idx = unbox_number(args[0]) as usize;
                    if let Some(ch) = s.chars().nth(idx) {
                        return box_string(ch.to_string());
                    }
                }
                box_string(String::new())
            }
            "substring" | "slice" => {
                if args.is_empty() {
                    return receiver_bits;
                }
                let start = if is_number(args[0]) {
                    unbox_number(args[0]) as usize
                } else {
                    0
                };
                let end = if args.len() > 1 && is_number(args[1]) {
                    unbox_number(args[1]) as usize
                } else {
                    s.len()
                };
                let len = if end >= start { end - start + 1 } else { 0 };
                let sub: String = s.chars().skip(start).take(len).collect();
                box_string(sub)
            }
            "concat" => {
                let mut result = s.to_string();
                for arg in args.iter() {
                    if is_heap_kind(*arg, HK_STRING) {
                        let arg_s = unbox_string(*arg);
                        result.push_str(arg_s);
                    } else if is_number(*arg) {
                        result.push_str(&format!("{}", unbox_number(*arg)));
                    }
                }
                box_string(result)
            }
            "indexOf" | "index_of" => {
                if args.is_empty() {
                    return box_number(-1.0);
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let needle = unbox_string(args[0]);
                    if let Some(idx) = s.find(needle) {
                        return box_number(idx as f64);
                    }
                }
                box_number(-1.0)
            }
            "lastIndexOf" | "last_index_of" => {
                if args.is_empty() {
                    return box_number(-1.0);
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let needle = unbox_string(args[0]);
                    if let Some(idx) = s.rfind(needle) {
                        return box_number(idx as f64);
                    }
                }
                box_number(-1.0)
            }
            "trimStart" | "trim_start" => box_string(s.trim_start().to_string()),
            "trimEnd" | "trim_end" => box_string(s.trim_end().to_string()),
            "toNumber" | "to_number" => match s.trim().parse::<f64>() {
                Ok(n) => box_number(n),
                Err(_) => TAG_NULL,
            },
            "toBool" | "to_bool" => match s.trim() {
                "true" => TAG_BOOL_TRUE,
                "false" => TAG_BOOL_FALSE,
                _ => TAG_NULL,
            },
            "chars" => {
                // AUDIT(C3): heap island — each char is jit_box'd into a
                // JitAlloc<String> and stored as raw u64 in the Vec. These inner
                // string allocations escape into the JitArray element buffer without
                // GC tracking. When GC feature enabled, route through gc_allocator.
                let chars: Vec<u64> = s
                    .chars()
                    .map(|ch| box_string(ch.to_string()))
                    .collect();
                JitArray::from_vec(chars).heap_box()
            }
            "isEmpty" | "is_empty" => {
                if s.is_empty() {
                    TAG_BOOL_TRUE
                } else {
                    TAG_BOOL_FALSE
                }
            }
            "repeat" => {
                if args.is_empty() {
                    return receiver_bits;
                }
                if is_number(args[0]) {
                    let count = unbox_number(args[0]) as usize;
                    let repeated = s.repeat(count);
                    return box_string(repeated);
                }
                receiver_bits
            }
            "padStart" | "pad_start" => {
                if args.is_empty() {
                    return receiver_bits;
                }
                let target_len = if is_number(args[0]) {
                    unbox_number(args[0]) as usize
                } else {
                    return receiver_bits;
                };
                let pad_char = if args.len() > 1 && is_heap_kind(args[1], HK_STRING) {
                    let pad_s = unbox_string(args[1]);
                    pad_s.chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                if s.len() >= target_len {
                    return receiver_bits;
                }
                let padding: String = std::iter::repeat_n(pad_char, target_len - s.len()).collect();
                let padded = format!("{}{}", padding, s);
                box_string(padded)
            }
            "padEnd" | "pad_end" => {
                if args.is_empty() {
                    return receiver_bits;
                }
                let target_len = if is_number(args[0]) {
                    unbox_number(args[0]) as usize
                } else {
                    return receiver_bits;
                };
                let pad_char = if args.len() > 1 && is_heap_kind(args[1], HK_STRING) {
                    let pad_s = unbox_string(args[1]);
                    pad_s.chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                if s.len() >= target_len {
                    return receiver_bits;
                }
                let padding: String = std::iter::repeat_n(pad_char, target_len - s.len()).collect();
                let padded = format!("{}{}", s, padding);
                box_string(padded)
            }
            _ => TAG_NULL,
        }
    }
}
