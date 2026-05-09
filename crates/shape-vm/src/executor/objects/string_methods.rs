//! String operations
//!
//! V2 (`MethodFnV2`) handlers for string methods.
//!
//! ## Phase 1.B-vm Wave-β `M-string` migration (playbook §10)
//!
//! Per the dispatcher contract for `STRING_METHODS` (PHF map in
//! `method_registry.rs`), these handlers are only invoked when the receiver
//! `args[0]` is statically `NativeKind::String` (i.e. `Arc::into_raw(Arc<
//! String>) as u64`). The dispatcher owns one strong-count share for the
//! receiver and drops it after the handler returns. We therefore borrow
//! `args[0]` as `&String` via `Arc::from_raw` paired with `Arc::into_raw`
//! (no refcount bump, no refcount release — pure borrow).
//!
//! Per playbook §3 pop pattern, String results are returned as
//! `Arc::into_raw(Arc::new(...)) as u64`, transferring one fresh share to
//! the caller. Inline scalars are returned as raw bits (`i as u64`,
//! `b as u64`, `f.to_bits()`).
//!
//! ## What was deleted
//!
//! - `raw_helpers::extract_str` / `raw_helpers::type_error` / `raw_helpers::
//!   extract_number_coerce` / `raw_helpers::extract_any_array` (the
//!   deleted tag_bits dispatch family — playbook §2.7.7 #4 / #7).
//! - `ValueWord` / `ValueWordExt` (deleted runtime carrier — playbook
//!   §2.7.7 #1).
//! - `ArgVec` / `vmarray_from_vec` / `IteratorState` (deleted dynamic
//!   array / iterator carriers).
//!
//! ## What surfaces (`NotImplemented(SURFACE: ...)`)
//!
//! Per playbook §7.4 (REVISED) DoD, sites that cannot be migrated cleanly
//! within `M-string` territory surface to the supervisor rather than
//! paper over with a forbidden pattern. The surfaced shapes are:
//!
//! 1. **Array results** (`split`, `graphemes`, `join` receiver-as-array)
//!    — need a kinded `Arc<TypedArrayData>` constructor + element-kind
//!    dispatch (ADR-006 §2.3 / §2.4 typed-Arc constructor migration).
//!    Out of `M-string` territory; depends on the typed-array cluster.
//! 2. **Iterator results** (`iter`) — depends on the post-§2.7.4 iterator
//!    state representation; the legacy `IteratorState` is deleted.
//! 3. **Char results** (`charAt` returning `Option<char>`) — `char` lives
//!    on `NativeKind::Ptr(HeapKind::Char)` per ADR-006 §2.7 inline-scalar
//!    payload, but the `MethodFnV2` ABI's kind-blind result channel
//!    cannot communicate the result kind to the caller. Phase-2c follow-
//!    up: extend `MethodFnV2` to return `(u64, NativeKind)` (or surface
//!    via a kinded result slot).
//! 4. **`v2_string_join` (array `.join(sep)`)** — receiver is an array,
//!    not a string; the kind-blind ABI cannot dispatch on element kind
//!    (same gap as `array_joins.rs` D-array-joins surfacing).
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10
//! Wave-β `M-string` row.

use crate::executor::VirtualMachine;
use shape_value::VMError;
use std::sync::Arc;

/// Borrow `args[0]` as `&String` under the dispatcher contract that says
/// the slot is `Arc::into_raw(Arc<String>) as u64`. The borrow does NOT
/// bump or release the strong count — the dispatcher owns the share for
/// the duration of the handler. Pairs `Arc::from_raw` with
/// `Arc::into_raw` immediately so no `Drop` runs.
///
/// SAFETY: caller must guarantee that the `STRING_METHODS` dispatcher
/// routed here, which means `args[0]`'s kind is statically
/// `NativeKind::String` and the bits are a valid `Arc::into_raw::<String>`
/// pointer.
#[inline]
unsafe fn borrow_string_arg(bits: u64) -> Option<String> {
    if bits == 0 {
        return None;
    }
    let arc: Arc<String> = unsafe { Arc::from_raw(bits as *const String) };
    let s = (*arc).clone();
    let _ = Arc::into_raw(arc); // restore the share count balance
    Some(s)
}

/// Borrow `args[0]` as a fresh owned `String` (cloning the inner contents).
/// Errors if `bits == 0`.
#[inline]
fn read_receiver_string(bits: u64) -> Result<String, VMError> {
    // SAFETY: dispatcher contract — `args[0]` is statically
    // `NativeKind::String` for every method routed through STRING_METHODS.
    unsafe { borrow_string_arg(bits) }.ok_or(VMError::TypeError {
        expected: "string",
        got: "null string pointer",
    })
}

/// Push an `Arc<String>` result by converting it to raw bits. Caller is
/// responsible for ensuring the dispatcher pushes the result with kind
/// `NativeKind::String`.
#[inline]
fn string_result(s: String) -> u64 {
    Arc::into_raw(Arc::new(s)) as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string method handlers
// ═══════════════════════════════════════════════════════════════════════════

/// len / length
pub fn v2_string_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(s.len() as u64)
}

/// toUpperCase / to_upper_case
pub fn v2_string_to_upper(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(string_result(s.to_uppercase()))
}

/// toLowerCase / to_lower_case
pub fn v2_string_to_lower(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(string_result(s.to_lowercase()))
}

/// trim
pub fn v2_string_trim(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(string_result(s.trim().to_string()))
}

/// trimStart / trim_start
pub fn v2_string_trim_start(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(string_result(s.trim_start().to_string()))
}

/// trimEnd / trim_end
pub fn v2_string_trim_end(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(string_result(s.trim_end().to_string()))
}

/// toString / to_string
///
/// Identity on the receiver bits. The dispatcher owns the share; we hand
/// the same `Arc::into_raw::<String>` pointer back as the result. A fresh
/// share is implied by the v2 dispatcher's "result is a freshly-owned
/// slot" contract — we re-bump the refcount so the result and the
/// dispatcher's drop don't double-free.
pub fn v2_string_to_string(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let bits = args[0];
    if bits != 0 {
        // SAFETY: dispatcher contract — `args[0]` is `Arc::into_raw::<
        // String>`. Bumping the count gives the result a fresh share so
        // both the result-side drop and the dispatcher's drop balance.
        unsafe { Arc::increment_strong_count(bits as *const String) };
    }
    Ok(bits)
}

/// startsWith / starts_with
pub fn v2_string_starts_with(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    // Type system has proven `args[1]` is `string` for this method.
    let prefix = unsafe { borrow_string_arg(args[1]) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "startsWith".to_string(),
            message: "requires a string argument".to_string(),
        }
    })?;
    Ok(s.starts_with(&prefix) as u64)
}

/// endsWith / ends_with
pub fn v2_string_ends_with(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let suffix = unsafe { borrow_string_arg(args[1]) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "endsWith".to_string(),
            message: "requires a string argument".to_string(),
        }
    })?;
    Ok(s.ends_with(&suffix) as u64)
}

/// contains
pub fn v2_string_contains(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let needle = unsafe { borrow_string_arg(args[1]) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "contains".to_string(),
            message: "requires a string argument".to_string(),
        }
    })?;
    Ok(s.contains(&needle) as u64)
}

/// indexOf / index_of
pub fn v2_string_index_of(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let needle = unsafe { borrow_string_arg(args[1]) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "indexOf".to_string(),
            message: "requires a string argument".to_string(),
        }
    })?;
    let result: i64 = match s.find(&needle) {
        Some(pos) => s[..pos].chars().count() as i64,
        None => -1,
    };
    Ok(result as u64)
}

/// repeat
///
/// Type system has proven `args[1]` is `int` for this method, so the
/// raw bits are a two's-complement `i64` count.
pub fn v2_string_repeat(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let count = args[1] as i64;
    if count < 0 {
        return Err(VMError::InvalidArgument {
            function: "repeat".to_string(),
            message: "count must be non-negative".to_string(),
        });
    }
    Ok(string_result(s.repeat(count as usize)))
}

/// charAt / char_at
///
/// SURFACE: `char` results live on `NativeKind::Ptr(HeapKind::Char)`
/// per ADR-006 §2.7 inline-scalar payload. The `MethodFnV2` ABI's
/// kind-blind result channel cannot communicate the result kind to the
/// caller — the dispatcher would push the bits with the receiver's kind
/// (`NativeKind::String`), which would mis-Drop the codepoint as if it
/// were an `Arc<String>` pointer. Phase-2c follow-up: extend
/// `MethodFnV2` to return `(u64, NativeKind)` or migrate to a kinded
/// result-slot signature.
pub fn v2_string_char_at(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(VMError::NotImplemented(
        "charAt — SURFACE: MethodFnV2 ABI lacks kinded result channel for \
         char results (NativeKind::Ptr(HeapKind::Char) inline-scalar \
         payload). Pushing the codepoint with the dispatcher's String kind \
         would mis-Drop as Arc<String>. Phase-2c follow-up: extend \
         MethodFnV2 to return (u64, NativeKind) or kinded result slot \
         (ADR-006 §2.7 / playbook §10 M-string)."
            .to_string(),
    ))
}

/// reverse
pub fn v2_string_reverse(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(string_result(s.chars().rev().collect::<String>()))
}

/// isDigit / is_digit
pub fn v2_string_is_digit(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok((!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())) as u64)
}

/// isAlpha / is_alpha
pub fn v2_string_is_alpha(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok((!s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())) as u64)
}

/// isAscii / is_ascii
pub fn v2_string_is_ascii(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    Ok(s.is_ascii() as u64)
}

/// toInt / to_int
pub fn v2_string_to_int(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let parsed: i64 = s
        .trim()
        .parse()
        .map_err(|_| VMError::RuntimeError(format!("Cannot convert '{}' to int", s)))?;
    Ok(parsed as u64)
}

/// toNumber / to_number / toFloat / to_float
pub fn v2_string_to_number(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let parsed: f64 = s
        .trim()
        .parse()
        .map_err(|_| VMError::RuntimeError(format!("Cannot convert '{}' to number", s)))?;
    Ok(parsed.to_bits())
}

/// codePointAt / code_point_at
///
/// Returns `i64` (the codepoint as a 32-bit value, or `-1` for out of
/// range), so unlike `charAt` this fits the kind-blind result channel
/// (`NativeKind::Int64`, raw two's-complement bits).
pub fn v2_string_code_point_at(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let index = args[1] as i64;
    if index < 0 {
        return Ok((-1i64) as u64);
    }
    let result: i64 = match s.chars().nth(index as usize) {
        Some(c) => c as u32 as i64,
        None => -1,
    };
    Ok(result as u64)
}

/// graphemeLen / grapheme_len
pub fn v2_string_grapheme_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_segmentation::UnicodeSegmentation;
    let s = read_receiver_string(args[0])?;
    Ok(s.graphemes(true).count() as u64)
}

/// padStart / pad_start
///
/// `args[1]` is `int` (target length); `args[2]` (optional) is `string`
/// (fill). Both kinds are type-system proven for this method.
pub fn v2_string_pad_start(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let target_len = args[1] as i64;
    if target_len < 0 {
        return Err(VMError::InvalidArgument {
            function: "padStart".to_string(),
            message: "length must be non-negative".to_string(),
        });
    }
    let target_len = target_len as usize;
    let fill = if args.len() > 2 {
        // SAFETY: type system has proven `args[2]` is `string` for the
        // padStart(target_len, fill: string) signature.
        unsafe { borrow_string_arg(args[2]) }.unwrap_or_else(|| " ".to_string())
    } else {
        " ".to_string()
    };
    let char_count = s.chars().count();
    if char_count >= target_len {
        Ok(string_result(s))
    } else {
        let pad_needed = target_len - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        if fill_chars.is_empty() {
            return Ok(string_result(s));
        }
        let mut padding = String::with_capacity(pad_needed + s.len());
        for i in 0..pad_needed {
            padding.push(fill_chars[i % fill_chars.len()]);
        }
        padding.push_str(&s);
        Ok(string_result(padding))
    }
}

/// padEnd / pad_end
pub fn v2_string_pad_end(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let target_len = args[1] as i64;
    if target_len < 0 {
        return Err(VMError::InvalidArgument {
            function: "padEnd".to_string(),
            message: "length must be non-negative".to_string(),
        });
    }
    let target_len = target_len as usize;
    let fill = if args.len() > 2 {
        unsafe { borrow_string_arg(args[2]) }.unwrap_or_else(|| " ".to_string())
    } else {
        " ".to_string()
    };
    let char_count = s.chars().count();
    if char_count >= target_len {
        Ok(string_result(s))
    } else {
        let pad_needed = target_len - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        if fill_chars.is_empty() {
            return Ok(string_result(s));
        }
        let mut result = s;
        for i in 0..pad_needed {
            result.push(fill_chars[i % fill_chars.len()]);
        }
        Ok(string_result(result))
    }
}

/// split
///
/// SURFACE: result is `Array<string>`. Constructing the kinded
/// `Arc<TypedArrayData>` with `NativeKind::String` element kind requires
/// the typed-array constructor surface (ADR-006 §2.3 / §2.4); out of
/// `M-string` territory.
pub fn v2_string_split(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(VMError::NotImplemented(
        "split — SURFACE: result is Array<string>; needs kinded \
         Arc<TypedArrayData> constructor with NativeKind::String element \
         kind (ADR-006 §2.3 / §2.4 typed-Arc). Out of M-string territory; \
         depends on typed-array cluster (ARRAY_METHODS migration)."
            .to_string(),
    ))
}

/// replace
pub fn v2_string_replace(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let old = unsafe { borrow_string_arg(args[1]) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "replace".to_string(),
            message: "requires an old argument".to_string(),
        }
    })?;
    let new = unsafe { borrow_string_arg(args[2]) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "replace".to_string(),
            message: "requires a new argument".to_string(),
        }
    })?;
    Ok(string_result(s.replace(&old, &new)))
}

/// substring
pub fn v2_string_substring(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let s = read_receiver_string(args[0])?;
    let start = args[1] as i64;
    if start < 0 {
        return Err(VMError::InvalidArgument {
            function: "substring".to_string(),
            message: "start must be non-negative".to_string(),
        });
    }
    let start = start as usize;
    let result = if args.len() > 2 {
        let end = args[2] as i64;
        if end < 0 {
            return Err(VMError::InvalidArgument {
                function: "substring".to_string(),
                message: "end must be non-negative".to_string(),
            });
        }
        let end = end as usize;
        let chars: Vec<char> = s.chars().collect();
        let end = end.min(chars.len());
        let start = start.min(end);
        chars[start..end].iter().collect::<String>()
    } else {
        let chars: Vec<char> = s.chars().collect();
        let start = start.min(chars.len());
        chars[start..].iter().collect::<String>()
    };
    Ok(string_result(result))
}

/// join
///
/// SURFACE: this handler's contract has the **array** as receiver
/// (`args[0]`) and a `string` separator (`args[1]`). The receiver kind
/// is `NativeKind::Ptr(HeapKind::TypedArray)`, not `String` — but it is
/// registered in `STRING_METHODS` for legacy reasons. The `MethodFnV2`
/// ABI's kind-blind args slice cannot dispatch on the array element
/// kind without a parallel `NativeKind` track (the same gap as
/// `array_joins.rs` D-array-joins surface). Stringifying mixed-kind
/// elements requires per-element kind inspection, which is forbidden to
/// fabricate locally (playbook §2.7.7 #4 / #7 / #9).
pub fn v2_string_join(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(VMError::NotImplemented(
        "join — SURFACE: receiver is Array<T>, not string; MethodFnV2 \
         ABI lacks parallel NativeKind track for element-kind dispatch \
         (same gap as array_joins.rs D-array-joins). Per-element kind \
         interpretation is forbidden without a kinded args slice \
         (playbook §2.7.7 #4 / #7 / #9). Phase-2c follow-up: extend \
         MethodFnV2 with parallel kind track, then re-implement via \
         Arc<TypedArrayData> walk + element-kind dispatch."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string methods: graphemes, normalize, iter
// ═══════════════════════════════════════════════════════════════════════════

/// graphemes
///
/// SURFACE: same shape as `split` — result is `Array<string>` whose
/// kinded `Arc<TypedArrayData>` constructor lives in the typed-array
/// cluster.
pub fn v2_string_graphemes(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(VMError::NotImplemented(
        "graphemes — SURFACE: result is Array<string>; needs kinded \
         Arc<TypedArrayData> constructor with NativeKind::String element \
         kind (ADR-006 §2.3 / §2.4 typed-Arc). Out of M-string territory."
            .to_string(),
    ))
}

/// normalize
pub fn v2_string_normalize(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    use unicode_normalization::UnicodeNormalization;
    let s = read_receiver_string(args[0])?;
    let form_bits = args.get(1).copied().ok_or_else(|| VMError::InvalidArgument {
        function: "normalize".to_string(),
        message: "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")"
            .to_string(),
    })?;
    let form = unsafe { borrow_string_arg(form_bits) }.ok_or_else(|| {
        VMError::InvalidArgument {
            function: "normalize".to_string(),
            message: "requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")"
                .to_string(),
        }
    })?;
    let normalized: String = match form.as_str() {
        "NFC" => s.nfc().collect(),
        "NFD" => s.nfd().collect(),
        "NFKC" => s.nfkc().collect(),
        "NFKD" => s.nfkd().collect(),
        other => {
            return Err(VMError::InvalidArgument {
                function: "normalize".to_string(),
                message: format!(
                    "unknown normalization form '{}', expected NFC/NFD/NFKC/NFKD",
                    other
                ),
            });
        }
    };
    Ok(string_result(normalized))
}

/// iter
///
/// SURFACE: the legacy `IteratorState` carrier is deleted (playbook
/// §2.7.7 / ADR-006 §2.7). The post-§2.7.4 iterator representation lives
/// in the iterator cluster (Phase-2c surface); `M-string` cannot
/// reconstruct it.
pub fn v2_string_iter(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(VMError::NotImplemented(
        "iter — SURFACE: legacy IteratorState carrier deleted (ADR-006 \
         §2.7). Post-§2.7.4 iterator representation owned by iterator \
         cluster; out of M-string territory. Phase-2c follow-up: kinded \
         iterator state + element-kind track."
            .to_string(),
    ))
}
