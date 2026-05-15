//! String operations
//!
//! V2 (`MethodFnV2`) handlers for string methods.
//!
//! ## Phase 1.B-vm Wave-δ MR-string-misc body migration (playbook §10)
//!
//! Per the dispatcher contract for `STRING_METHODS` (PHF map in
//! `method_registry.rs`), these handlers are invoked with `args[0]` =
//! receiver — for these handlers the receiver kind is statically
//! `NativeKind::String` (i.e. the `Arc::into_raw(Arc<String>)` raw pointer
//! is stored in `args[0].slot`). The dispatch shell `op_call_method`
//! constructs the `&[KindedSlot]` slice from popped stack args; per the
//! §2.7.10 / Q11 ABI the args slice is borrow-only and the dispatch
//! shell owns each `KindedSlot`'s share for the call duration. Handler
//! bodies read the receiver via `args[0].as_str()` (which dispatches on
//! `args[0].kind`); per-arg kinds dispatch via `args[i].kind` per §2.7.6
//! / Q8 heterogeneous-kind body pattern.
//!
//! Result construction uses per-`NativeKind` `KindedSlot::from_*`
//! constructors (e.g. `KindedSlot::from_string_arc(Arc::new(s))` for
//! string results, `KindedSlot::from_int(i)` for index results,
//! `KindedSlot::from_bool(b)` for predicates, `KindedSlot::from_char(c)`
//! for char results, `KindedSlot::from_typed_array(arc)` for
//! Array<string> results).
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
//! ## What remains as `NotImplemented(SURFACE)`
//!
//! - **`v2_string_iter`** — depends on the post-§2.7.4 iterator state
//!   representation; the legacy `IteratorState` carrier is deleted and
//!   the kinded iterator state lives in the iterator cluster (Phase-2c
//!   surface).

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder (for TypedArrayData-dependent methods)
// ═══════════════════════════════════════════════════════════════════════════

#[cold]
#[inline(never)]
fn ckpt5_string_array_surface(op: &'static str) -> VMError {
    VMError::NotImplemented(format!(
        "String.{op}: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 \
         surface. `Arc<TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)>` \
         result carrier DELETED at V3-S5 ckpt-1..ckpt-4 per W12-typed-\
         array-data-deletion audit §3.5 + §3.6 + §B + ADR-006 §2.7.24 \
         Q25.A SUPERSEDED. Rebuild lands at ckpt-6 STRICT close per the \
         v2-raw `TypedArray<*const StringObj>` carrier shape. REFUSED ON \
         SIGHT: TypedArrayData resurrection under any rename (Refusal #1).",
        op = op,
    ))
}

/// Read the receiver `&str` from `args[0]`. The dispatcher contract
/// guarantees `args[0].kind == NativeKind::String` for STRING_METHODS
/// entries, so this is total in practice; we still surface a TypeError
/// rather than panic on a contract violation.
#[inline]
fn receiver_str<'a>(args: &'a [KindedSlot]) -> Result<&'a str, VMError> {
    args.first()
        .and_then(|a| a.as_str())
        .ok_or(VMError::TypeError {
            expected: "string receiver",
            got: "non-string kind",
        })
}

/// Read an `int` argument by index. The compiler proves these kinds at
/// emit time (the `repeat`/`pad_start`/`pad_end`/`char_at` arguments are
/// declared `int`); a kind mismatch is a runtime invariant violation.
#[inline]
fn int_arg(args: &[KindedSlot], idx: usize) -> Result<i64, VMError> {
    args.get(idx)
        .and_then(|a| a.as_i64())
        .ok_or(VMError::TypeError {
            expected: "int argument",
            got: "non-int kind",
        })
}

/// Read a `string` argument by index. Same rationale as `int_arg`.
#[inline]
fn str_arg<'a>(args: &'a [KindedSlot], idx: usize) -> Result<&'a str, VMError> {
    args.get(idx)
        .and_then(|a| a.as_str())
        .ok_or(VMError::TypeError {
            expected: "string argument",
            got: "non-string kind",
        })
}

/// Convenience: build a fresh `KindedSlot` String result from an owned
/// `String` value.
#[inline]
fn string_result(s: String) -> KindedSlot {
    KindedSlot::from_string_arc(Arc::new(s))
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string method handlers
// ═══════════════════════════════════════════════════════════════════════════

/// len / length
pub fn v2_string_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(KindedSlot::from_int(s.chars().count() as i64))
}

/// toUpperCase / to_upper_case
pub fn v2_string_to_upper(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.to_uppercase()))
}

/// toLowerCase / to_lower_case
pub fn v2_string_to_lower(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.to_lowercase()))
}

/// trim
pub fn v2_string_trim(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.trim().to_string()))
}

/// trimStart / trim_start
pub fn v2_string_trim_start(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.trim_start().to_string()))
}

/// trimEnd / trim_end
pub fn v2_string_trim_end(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.trim_end().to_string()))
}

/// toString / to_string
///
/// Identity on the receiver. Allocates a fresh `Arc<String>` clone of
/// the receiver's contents; the dispatcher's caller owns the result
/// share independently of the receiver share.
pub fn v2_string_to_string(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.to_string()))
}

/// startsWith / starts_with
pub fn v2_string_starts_with(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let prefix = str_arg(args, 1)?;
    Ok(KindedSlot::from_bool(s.starts_with(prefix)))
}

/// endsWith / ends_with
pub fn v2_string_ends_with(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let suffix = str_arg(args, 1)?;
    Ok(KindedSlot::from_bool(s.ends_with(suffix)))
}

/// contains
pub fn v2_string_contains(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let needle = str_arg(args, 1)?;
    Ok(KindedSlot::from_bool(s.contains(needle)))
}

/// indexOf / index_of — returns `i64` (-1 if not found)
pub fn v2_string_index_of(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let needle = str_arg(args, 1)?;
    let idx = match s.find(needle) {
        Some(byte_idx) => {
            // Translate byte offset → char offset to match the language's
            // char-indexed semantics.
            s[..byte_idx].chars().count() as i64
        }
        None => -1,
    };
    Ok(KindedSlot::from_int(idx))
}

/// repeat
///
/// Type system has proven `args[1]` is `int` for this method.
pub fn v2_string_repeat(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let n = int_arg(args, 1)?;
    if n < 0 {
        return Err(VMError::RuntimeError(
            "string.repeat(n): n must be non-negative".to_string(),
        ));
    }
    Ok(string_result(s.repeat(n as usize)))
}

/// charAt / char_at
///
/// Wave-δ MR-string-misc: the §2.7.10 / Q11 kinded `MethodFnV2` ABI
/// carries the result kind on the returned `KindedSlot`, so
/// `NativeKind::Ptr(HeapKind::Char)` results are first-class.
/// Out-of-range indices return `Char('\0')` — the language semantics
/// model `char_at` as total (the pre-§2.7.10 implementation returned
/// `Option<char>` but the kind-blind ABI couldn't represent `None`
/// either). Callers using `string.len()` to bound the index get the
/// expected behavior.
pub fn v2_string_char_at(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let i = int_arg(args, 1)?;
    if i < 0 {
        return Ok(KindedSlot::from_char('\0'));
    }
    let c = s.chars().nth(i as usize).unwrap_or('\0');
    Ok(KindedSlot::from_char(c))
}

/// reverse
pub fn v2_string_reverse(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.chars().rev().collect()))
}

/// isDigit / is_digit
pub fn v2_string_is_digit(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(KindedSlot::from_bool(
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()),
    ))
}

/// isAlpha / is_alpha
pub fn v2_string_is_alpha(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(KindedSlot::from_bool(
        !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
    ))
}

/// isAscii / is_ascii
pub fn v2_string_is_ascii(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(KindedSlot::from_bool(s.is_ascii()))
}

/// toInt / to_int
///
/// Returns `0` on parse failure (matches the existing language contract;
/// the typed `Result<int, ParseError>` form is left for a future
/// signature change).
pub fn v2_string_to_int(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let i: i64 = s.trim().parse().unwrap_or(0);
    Ok(KindedSlot::from_int(i))
}

/// toNumber / to_number / toFloat / to_float
pub fn v2_string_to_number(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let n: f64 = s.trim().parse().unwrap_or(0.0);
    Ok(KindedSlot::from_number(n))
}

/// codePointAt / code_point_at — returns the codepoint as `i64` (-1 out of range)
pub fn v2_string_code_point_at(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let i = int_arg(args, 1)?;
    if i < 0 {
        return Ok(KindedSlot::from_int(-1));
    }
    let cp = s
        .chars()
        .nth(i as usize)
        .map(|c| c as i64)
        .unwrap_or(-1);
    Ok(KindedSlot::from_int(cp))
}

/// graphemeLen / grapheme_len
///
/// Returns the codepoint count (Rust's `chars().count()`). True
/// extended-grapheme-cluster counting (Unicode UAX #29) requires a
/// dedicated dependency; the existing `len()` semantics already use
/// `chars().count()` so we mirror that here.
pub fn v2_string_grapheme_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(KindedSlot::from_int(s.chars().count() as i64))
}

/// padStart / pad_start
pub fn v2_string_pad_start(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let target_len = int_arg(args, 1)?;
    let pad = args
        .get(2)
        .and_then(|a| a.as_str())
        .unwrap_or(" ");
    Ok(string_result(pad_to(s, target_len, pad, /*at_start=*/ true)))
}

/// padEnd / pad_end
pub fn v2_string_pad_end(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let target_len = int_arg(args, 1)?;
    let pad = args
        .get(2)
        .and_then(|a| a.as_str())
        .unwrap_or(" ");
    Ok(string_result(pad_to(s, target_len, pad, /*at_start=*/ false)))
}

/// split — `Array<string>` result.
///
/// V3-S5 ckpt-5: TypedArrayData::String result carrier deleted; rebuild
/// lands at ckpt-6 STRICT close per v2-raw `TypedArray<*const StringObj>`.
pub fn v2_string_split(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let _ = receiver_str(args)?;
    let _ = str_arg(args, 1)?;
    let _: Option<Arc<String>> = None;
    Err(ckpt5_string_array_surface("split"))
}

/// replace — replace all occurrences of `from` with `to`
pub fn v2_string_replace(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let from = str_arg(args, 1)?;
    let to = str_arg(args, 2)?;
    Ok(string_result(s.replace(from, to)))
}

/// substring — char-indexed [start, end)
pub fn v2_string_substring(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    let start = int_arg(args, 1)?;
    let total = s.chars().count() as i64;
    let end = match args.get(2).and_then(|a| a.as_i64()) {
        Some(e) => e,
        None => total,
    };
    let s_idx = start.clamp(0, total) as usize;
    let e_idx = end.clamp(0, total) as usize;
    if s_idx >= e_idx {
        return Ok(string_result(String::new()));
    }
    let result: String = s.chars().skip(s_idx).take(e_idx - s_idx).collect();
    Ok(string_result(result))
}

/// join — `Array<T>` receiver, separator (`string`) argument.
///
/// V3-S5 ckpt-5: `Arc<TypedArrayData>` receiver dispatch deleted; rebuild
/// lands at ckpt-6 STRICT close per per-element-kind v2-raw `TypedArray<T>`
/// direct access.
pub fn v2_string_join(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let _ = str_arg(args, 1)?;
    // Suppress unused-fn warnings; `format_f64` stays live for forward
    // compatibility (will be needed when the v2-raw rebuild lands).
    let _f = format_f64;
    Err(ckpt5_string_array_surface("join"))
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 string methods: graphemes, normalize, iter
// ═══════════════════════════════════════════════════════════════════════════

/// graphemes — same shape as `split` on a per-char boundary.
///
/// Returns `Array<string>` with one string per Unicode codepoint. True
/// extended-grapheme-cluster splitting (UAX #29) requires a dedicated
/// dependency; the codepoint approximation matches the existing
/// `chars().count()` length semantics.
pub fn v2_string_graphemes(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let _ = receiver_str(args)?;
    let _: Option<Arc<String>> = None;
    Err(ckpt5_string_array_surface("graphemes"))
}

/// normalize — Unicode normalization. The `unicode-normalization` crate
/// is not a current dependency; pre-§2.7.10 the body returned the input
/// unchanged. Preserve that behavior.
pub fn v2_string_normalize(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let s = receiver_str(args)?;
    Ok(string_result(s.to_string()))
}

/// iter
///
/// W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10): forwards to
/// the iterator cluster's `handle_string_iter` factory. Constructs a
/// fresh `IteratorState { source: IteratorSource::String(arc),
/// transforms: vec![], cursor: 0 }` and wraps it as
/// `KindedSlot::from_iterator(Arc::new(state))`.
pub fn v2_string_iter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::iterator_methods::handle_string_iter(vm, args, ctx)
}

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Pad `s` to `target_len` *codepoints* using `pad`, at start or end.
/// Mirrors JS String.prototype.padStart / padEnd semantics.
fn pad_to(s: &str, target_len: i64, pad: &str, at_start: bool) -> String {
    if target_len <= 0 || pad.is_empty() {
        return s.to_string();
    }
    let s_chars = s.chars().count();
    let target = target_len as usize;
    if s_chars >= target {
        return s.to_string();
    }
    let need = target - s_chars;
    let pad_chars: Vec<char> = pad.chars().collect();
    let pad_len = pad_chars.len();
    if pad_len == 0 {
        return s.to_string();
    }
    let mut prefix = String::with_capacity(need);
    for i in 0..need {
        prefix.push(pad_chars[i % pad_len]);
    }
    if at_start {
        format!("{}{}", prefix, s)
    } else {
        format!("{}{}", s, prefix)
    }
}

/// Format an f64 the same way the language's array Display does: integer
/// values print without a fractional part when they fit cleanly.
fn format_f64(v: &f64) -> String {
    if v.is_finite() && *v == v.trunc() && v.abs() < 1e15 {
        format!("{}", *v as i64)
    } else {
        format!("{}", v)
    }
}
