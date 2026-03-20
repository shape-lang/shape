// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 8 sites
//     jit_box(HK_ARRAY, ...) — reverse, slice, concat, flatten, unique, sort, take, drop
//     jit_box(HK_STRING, ...) — join
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (elements stored in JitArrays are existing u64 values copied from the
//      source array — no new allocations within the element buffer)
//!
//! Array method implementations for JIT

use crate::jit_array::JitArray;
use crate::nan_boxing::*;

/// Call a method on an array value
#[inline(always)]
pub fn call_array_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    unsafe {
        let arr = JitArray::from_heap_bits(receiver_bits);
        let slice = arr.as_slice();

        match method_name {
            "length" | "len" => box_number(arr.len() as f64),
            "first" => arr.first().copied().unwrap_or(TAG_NULL),
            "last" => arr.last().copied().unwrap_or(TAG_NULL),
            "includes" | "contains" => {
                if args.is_empty() {
                    return TAG_BOOL_FALSE;
                }
                let needle = args[0];
                for &elem in slice.iter() {
                    if elem == needle {
                        return TAG_BOOL_TRUE;
                    }
                    if is_number(elem)
                        && is_number(needle)
                        && unbox_number(elem) == unbox_number(needle)
                    {
                        return TAG_BOOL_TRUE;
                    }
                }
                TAG_BOOL_FALSE
            }
            "indexOf" => {
                if args.is_empty() {
                    return box_number(-1.0);
                }
                let needle = args[0];
                for (i, &elem) in slice.iter().enumerate() {
                    if elem == needle {
                        return box_number(i as f64);
                    }
                    if is_number(elem)
                        && is_number(needle)
                        && unbox_number(elem) == unbox_number(needle)
                    {
                        return box_number(i as f64);
                    }
                }
                box_number(-1.0)
            }
            "reverse" => {
                let mut reversed = slice.to_vec();
                reversed.reverse();
                JitArray::from_vec(reversed).heap_box()
            }
            "slice" => {
                let len = arr.len() as i64;
                let start = if !args.is_empty() && is_number(args[0]) {
                    let s = unbox_number(args[0]) as i64;
                    if s < 0 {
                        (len + s).max(0) as usize
                    } else {
                        s.min(len) as usize
                    }
                } else {
                    0
                };
                let end = if args.len() > 1 && is_number(args[1]) {
                    let e = unbox_number(args[1]) as i64;
                    if e < 0 {
                        (len + e).max(0) as usize
                    } else {
                        e.min(len) as usize
                    }
                } else {
                    arr.len()
                };
                let sliced = if start < end && start < arr.len() {
                    JitArray::from_slice(&slice[start..end.min(arr.len())])
                } else {
                    JitArray::new()
                };
                sliced.heap_box()
            }
            "join" => {
                let separator = if !args.is_empty() {
                    if is_heap_kind(args[0], HK_STRING) {
                        jit_unbox::<String>(args[0]).clone()
                    } else {
                        ",".to_string()
                    }
                } else {
                    ",".to_string()
                };

                let parts: Vec<String> = slice
                    .iter()
                    .map(|&elem| {
                        if is_number(elem) {
                            format!("{}", unbox_number(elem))
                        } else if is_heap_kind(elem, HK_STRING) {
                            jit_unbox::<String>(elem).clone()
                        } else if elem == TAG_NULL {
                            "null".to_string()
                        } else if elem == TAG_BOOL_TRUE {
                            "true".to_string()
                        } else if elem == TAG_BOOL_FALSE {
                            "false".to_string()
                        } else {
                            "[object]".to_string()
                        }
                    })
                    .collect();

                let joined = parts.join(&separator);
                jit_box(HK_STRING, joined)
            }
            "sum" => {
                let mut total = 0.0;
                for &elem in slice.iter() {
                    if is_number(elem) {
                        total += unbox_number(elem);
                    }
                }
                box_number(total)
            }
            "avg" | "mean" => {
                if arr.is_empty() {
                    return TAG_NULL;
                }
                let mut total = 0.0;
                for &elem in slice.iter() {
                    if is_number(elem) {
                        total += unbox_number(elem);
                    }
                }
                box_number(total / arr.len() as f64)
            }
            "min" => {
                if arr.is_empty() {
                    return TAG_NULL;
                }
                let mut min_val = f64::INFINITY;
                for &elem in slice.iter() {
                    if is_number(elem) {
                        let v = unbox_number(elem);
                        if v < min_val {
                            min_val = v;
                        }
                    }
                }
                if min_val.is_finite() {
                    box_number(min_val)
                } else {
                    TAG_NULL
                }
            }
            "max" => {
                if arr.is_empty() {
                    return TAG_NULL;
                }
                let mut max_val = f64::NEG_INFINITY;
                for &elem in slice.iter() {
                    if is_number(elem) {
                        let v = unbox_number(elem);
                        if v > max_val {
                            max_val = v;
                        }
                    }
                }
                if max_val.is_finite() {
                    box_number(max_val)
                } else {
                    TAG_NULL
                }
            }
            "take" => {
                if args.is_empty() {
                    return receiver_bits;
                }
                let count = if is_number(args[0]) {
                    (unbox_number(args[0]) as usize).min(arr.len())
                } else {
                    return receiver_bits;
                };
                JitArray::from_slice(&slice[..count]).heap_box()
            }
            "drop" => {
                if args.is_empty() {
                    return receiver_bits;
                }
                let count = if is_number(args[0]) {
                    (unbox_number(args[0]) as usize).min(arr.len())
                } else {
                    return receiver_bits;
                };
                JitArray::from_slice(&slice[count..]).heap_box()
            }
            "concat" => {
                let mut result: Vec<u64> = slice.to_vec();
                for arg in args.iter() {
                    if is_heap_kind(*arg, HK_ARRAY) {
                        let other = JitArray::from_heap_bits(*arg);
                        result.extend_from_slice(other.as_slice());
                    } else {
                        result.push(*arg);
                    }
                }
                JitArray::from_vec(result).heap_box()
            }
            "flatten" | "flat" => {
                let mut result = Vec::new();
                for &elem in slice.iter() {
                    if is_heap_kind(elem, HK_ARRAY) {
                        let inner = JitArray::from_heap_bits(elem);
                        result.extend_from_slice(inner.as_slice());
                    } else {
                        result.push(elem);
                    }
                }
                JitArray::from_vec(result).heap_box()
            }
            "unique" => {
                let mut seen = Vec::new();
                let mut result = Vec::new();
                for &elem in slice.iter() {
                    let is_dup = seen.iter().any(|&s| {
                        if is_number(elem) && is_number(s) {
                            unbox_number(elem) == unbox_number(s)
                        } else {
                            elem == s
                        }
                    });
                    if !is_dup {
                        seen.push(elem);
                        result.push(elem);
                    }
                }
                JitArray::from_vec(result).heap_box()
            }
            "sort" | "sorted" => {
                let mut sorted = slice.to_vec();
                sorted.sort_by(|&a, &b| {
                    if is_number(a) && is_number(b) {
                        unbox_number(a)
                            .partial_cmp(&unbox_number(b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    } else {
                        std::cmp::Ordering::Equal
                    }
                });
                JitArray::from_vec(sorted).heap_box()
            }
            _ => TAG_NULL,
        }
    }
}
