// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 18 sites
//     jit_box(HK_STRING, ...) — toUpperCase, toLowerCase, trim, replace,
//     charAt, substring, concat, padStart, padEnd, repeat, trimStart, trimEnd
//     jit_box(HK_ARRAY, ...) — split, chars
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 2 sites (split, chars)
//!
//! String method implementations for JIT

use crate::jit_array::JitArray;
use crate::nan_boxing::*;

/// Call a method on a string value
#[inline(always)]
pub fn call_string_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    unsafe {
        let s = jit_unbox::<String>(receiver_bits);

        match method_name {
            "length" | "len" => box_number(s.len() as f64),
            "toUpperCase" | "to_upper_case" => jit_box(HK_STRING, s.to_uppercase()),
            "toLowerCase" | "to_lower_case" => jit_box(HK_STRING, s.to_lowercase()),
            "trim" => jit_box(HK_STRING, s.trim().to_string()),
            "split" => {
                let separator = if !args.is_empty() {
                    if is_heap_kind(args[0], HK_STRING) {
                        jit_unbox::<String>(args[0]).clone()
                    } else {
                        " ".to_string()
                    }
                } else {
                    " ".to_string()
                };

                // AUDIT(C2): heap island — each split part is jit_box'd into a
                // JitAlloc<String> and stored as raw u64 in the Vec. These inner
                // string allocations escape into the JitArray element buffer without
                // GC tracking. When GC feature enabled, route through gc_allocator.
                let parts: Vec<u64> = s
                    .split(&separator)
                    .map(|part| jit_box(HK_STRING, part.to_string()))
                    .collect();

                JitArray::from_vec(parts).heap_box()
            }
            "includes" | "contains" => {
                if args.is_empty() {
                    return TAG_BOOL_FALSE;
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let needle = jit_unbox::<String>(args[0]);
                    if s.contains(needle.as_str()) {
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
                    let prefix = jit_unbox::<String>(args[0]);
                    if s.starts_with(prefix.as_str()) {
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
                    let suffix = jit_unbox::<String>(args[0]);
                    if s.ends_with(suffix.as_str()) {
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
                    let from = jit_unbox::<String>(args[0]);
                    let to = jit_unbox::<String>(args[1]);
                    let replaced = s.replace(from.as_str(), to.as_str());
                    return jit_box(HK_STRING, replaced);
                }
                receiver_bits
            }
            "charAt" | "char_at" => {
                if args.is_empty() {
                    return jit_box(HK_STRING, String::new());
                }
                if is_number(args[0]) {
                    let idx = unbox_number(args[0]) as usize;
                    if let Some(ch) = s.chars().nth(idx) {
                        return jit_box(HK_STRING, ch.to_string());
                    }
                }
                jit_box(HK_STRING, String::new())
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
                jit_box(HK_STRING, sub)
            }
            "concat" => {
                let mut result = s.clone();
                for arg in args.iter() {
                    if is_heap_kind(*arg, HK_STRING) {
                        let arg_s = jit_unbox::<String>(*arg);
                        result.push_str(arg_s);
                    } else if is_number(*arg) {
                        result.push_str(&format!("{}", unbox_number(*arg)));
                    }
                }
                jit_box(HK_STRING, result)
            }
            "indexOf" | "index_of" => {
                if args.is_empty() {
                    return box_number(-1.0);
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let needle = jit_unbox::<String>(args[0]);
                    if let Some(idx) = s.find(needle.as_str()) {
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
                    let needle = jit_unbox::<String>(args[0]);
                    if let Some(idx) = s.rfind(needle.as_str()) {
                        return box_number(idx as f64);
                    }
                }
                box_number(-1.0)
            }
            "trimStart" | "trim_start" => jit_box(HK_STRING, s.trim_start().to_string()),
            "trimEnd" | "trim_end" => jit_box(HK_STRING, s.trim_end().to_string()),
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
                    .map(|ch| jit_box(HK_STRING, ch.to_string()))
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
                    return jit_box(HK_STRING, repeated);
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
                    let pad_s = jit_unbox::<String>(args[1]);
                    pad_s.chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                if s.len() >= target_len {
                    return receiver_bits;
                }
                let padding: String = std::iter::repeat_n(pad_char, target_len - s.len()).collect();
                let padded = format!("{}{}", padding, s);
                jit_box(HK_STRING, padded)
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
                    let pad_s = jit_unbox::<String>(args[1]);
                    pad_s.chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                if s.len() >= target_len {
                    return receiver_bits;
                }
                let padding: String = std::iter::repeat_n(pad_char, target_len - s.len()).collect();
                let padded = format!("{}{}", s, padding);
                jit_box(HK_STRING, padded)
            }
            _ => TAG_NULL,
        }
    }
}
