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
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

// Pre-§2.7.9 helpers `borrow_string_arg`, `read_receiver_string`, and
// `string_result` deleted with their callers' SURFACE migration.
// They operated on raw `u64` bits per the kind-blind ABI; the post-§2.7.9
// kinded form will read the receiver `Arc<String>` from `args[0].slot`
// directly (kind statically `NativeKind::String` per dispatcher contract)
// and return result `KindedSlot` via `KindedSlot::from_string_arc(Arc::new(s))`
// per playbook §3 per-`HeapKind` push pattern. Wave-γ-followup body
// migration territory.

// ═══════════════════════════════════════════════════════════════════════════
// V2 string method handlers
// ═══════════════════════════════════════════════════════════════════════════

/// len / length
pub fn v2_string_len(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_len — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// toUpperCase / to_upper_case
pub fn v2_string_to_upper(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_to_upper — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// toLowerCase / to_lower_case
pub fn v2_string_to_lower(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_to_lower — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// trim
pub fn v2_string_trim(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_trim — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// trimStart / trim_start
pub fn v2_string_trim_start(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_trim_start — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// trimEnd / trim_end
pub fn v2_string_trim_end(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_trim_end — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_to_string — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// startsWith / starts_with
pub fn v2_string_starts_with(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_starts_with — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// endsWith / ends_with
pub fn v2_string_ends_with(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_ends_with — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// contains
pub fn v2_string_contains(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_contains — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// indexOf / index_of
pub fn v2_string_index_of(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_index_of — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// repeat
///
/// Type system has proven `args[1]` is `int` for this method, so the
/// raw bits are a two's-complement `i64` count.
pub fn v2_string_repeat(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_repeat — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_reverse — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// isDigit / is_digit
pub fn v2_string_is_digit(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_is_digit — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// isAlpha / is_alpha
pub fn v2_string_is_alpha(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_is_alpha — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// isAscii / is_ascii
pub fn v2_string_is_ascii(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_is_ascii — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// toInt / to_int
pub fn v2_string_to_int(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_to_int — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// toNumber / to_number / toFloat / to_float
pub fn v2_string_to_number(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_to_number — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// codePointAt / code_point_at
///
/// Returns `i64` (the codepoint as a 32-bit value, or `-1` for out of
/// range), so unlike `charAt` this fits the kind-blind result channel
/// (`NativeKind::Int64`, raw two's-complement bits).
pub fn v2_string_code_point_at(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_code_point_at — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// graphemeLen / grapheme_len
pub fn v2_string_grapheme_len(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_grapheme_len — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// padStart / pad_start
///
/// `args[1]` is `int` (target length); `args[2]` (optional) is `string`
/// (fill). Both kinds are type-system proven for this method.
pub fn v2_string_pad_start(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_pad_start — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// padEnd / pad_end
pub fn v2_string_pad_end(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_pad_end — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// split
///
/// SURFACE: result is `Array<string>`. Constructing the kinded
/// `Arc<TypedArrayData>` with `NativeKind::String` element kind requires
/// the typed-array constructor surface (ADR-006 §2.3 / §2.4); out of
/// `M-string` territory.
pub fn v2_string_split(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_replace — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// substring
pub fn v2_string_substring(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_substring — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
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
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_string_normalize — SURFACE: ADR-006 §2.7.9 / Q11 — kinded MethodFnV2 ABI landed (Wave-γ G-method-fn-v2-abi); body migration is Wave-γ-followup territory. Receiver kind dispatch via `args[0].kind` + `args[0].slot.as_heap_value()` (HeapValue match per ADR-005 §1) replaces the deleted ValueWord-shape probes. Per-arg kinds come from the §2.7.7 stack parallel-Vec<NativeKind> track at the dispatch boundary; result is constructed via per-NativeKind `KindedSlot::from_*` (or `KindedSlot::new(ValueSlot::from_..., NativeKind::*)` for heap arms) per playbook §3."
            .to_string(),
    ))
}

/// iter
///
/// SURFACE: the legacy `IteratorState` carrier is deleted (playbook
/// §2.7.7 / ADR-006 §2.7). The post-§2.7.4 iterator representation lives
/// in the iterator cluster (Phase-2c surface); `M-string` cannot
/// reconstruct it.
pub fn v2_string_iter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "iter — SURFACE: legacy IteratorState carrier deleted (ADR-006 \
         §2.7). Post-§2.7.4 iterator representation owned by iterator \
         cluster; out of M-string territory. Phase-2c follow-up: kinded \
         iterator state + element-kind track."
            .to_string(),
    ))
}
