//! Array set operations
//!
//! Handles: union, intersect, except, unique, distinct, distinct_by
//!
//! Wave-β `A-array-sort-sets` sub-cluster (ADR-006 §2.7.6 / §2.7.7 /
//! §2.7.8 / Q8-Q10): the `MethodFnV2` native ABI takes `args: &mut [u64]`
//! raw bits with **no parallel `NativeKind` track**. Every interaction
//! these handlers need — interpreting `args[i]` as an array vs.
//! closure, iterating `TypedArrayData` element-by-element with a
//! per-element kind, equality-comparing values across heterogeneous
//! element kinds, retaining heap shares into a result buffer, calling
//! back into `op_call_value` with kinded callee + arg + count slots —
//! is impossible without a kind-aware extension to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted
//!     tag_bits dispatch, §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op"
//!     (forbidden §2.7.7 #9 — the W-series rationalization the
//!     playbook names verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7), or
//!   - reaching for `vw_clone(bits)` / `vw_drop(bits)` (forbidden
//!     §2.7.7 #8 — replaced by `clone_with_kind` / `drop_with_kind`
//!     which require a kinded slot).
//!
//! This is the same architectural ABI gap that `D-array-joins` surfaced
//! at close (`2fe4a6b`); the Phase-2c follow-up there — extend
//! `MethodFnV2` to a kind-aware `args: &mut [(u64, NativeKind)]` (or
//! parallel-track equivalent) — also unblocks this file. Per playbook
//! §7.4 (REVISED) the correct refusal shape is `NotImplemented(SURFACE:
//! ...)` with the kind-source gap named explicitly, never a forbidden-
//! pattern workaround.

use crate::executor::VirtualMachine;
use shape_value::VMError;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `union` — set union of two arrays (deduplicated, order-preserving)
///
/// args: [array, other_array]
pub(crate) fn handle_union_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: the MethodFnV2 ABI's `args: &mut [u64]` is kind-blind.
    // This handler needs:
    //
    //   1. To interpret args[0] and args[1] as arrays — requires
    //      `kind == NativeKind::Ptr(HeapKind::TypedArray)` proof per
    //      §2.7.7, then `slot.as_heap_value() + HeapValue::TypedArray
    //      (arc)` match per ADR-005 §1 single-discriminator.
    //   2. To iterate `TypedArrayData::*` arms with per-element
    //      `NativeKind` for equality comparison and result-buffer
    //      retention. Per-arm kinds are `I64 => Int64`, `F64 =>
    //      Float64`, `String => NativeKind::String`, …; `HeapValue(_)`
    //      and `Matrix` cannot be uniformly kinded — see §8 surface
    //      trigger.
    //   3. To equality-compare elements across heterogeneous kinds.
    //      The legacy body used `ValueWordExt::vw_equals` (a deleted
    //      ValueWord method §2.7.7 #7); the kinded replacement
    //      requires per-`NativeKind` equality dispatch (numeric ==,
    //      string-arc deep-eq, heap-pointer identity) which itself
    //      depends on kinded args.
    //   4. To retain elements into the result buffer. The legacy body
    //      used `shape_value::vw_clone(bits)` (forbidden §2.7.7 #8 —
    //      replaced by `clone_with_kind(bits, kind)` which needs a
    //      kinded slot). And `vmarray_from_vec` materialized the
    //      result as a generic `HeapValue::Array(Arc<Vec<ValueWord>>)`
    //      — that "generic VW array" arm is itself a Phase-2c reentry
    //      surface (see `array_operations.rs` close).
    //
    // Bool-defaulting any of the above is forbidden (§2.7.7 #9). The
    // correct refusal shape is the surface below.
    Err(VMError::NotImplemented(
        "union — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up; same gap as D-array-joins \
         close 2fe4a6b). Cannot interpret arg kinds, iterate element \
         kinds, equality-compare across kinds, or retain into a kinded \
         result buffer (replaces `vw_clone` / `vmarray_from_vec` — both \
         forbidden post-§2.7.7). Phase-2c follow-up: extend MethodFnV2 \
         to `args: &mut [(u64, NativeKind)]` (or equivalent parallel \
         track) analogous to the stack/cell-store §2.7.7/§2.7.8 \
         invariants."
            .to_string(),
    ))
}

/// v2 `intersect` — set intersection of two arrays (deduplicated, order-preserving)
///
/// args: [array, other_array]
pub(crate) fn handle_intersect_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_union_v2` — kind-blind args, kind-
    // blind element iteration, kind-blind equality compare, kind-blind
    // retain into result. (The legacy body used `vw_equals` for cross-
    // array membership testing and `vw_clone` to retain shares.)
    Err(VMError::NotImplemented(
        "intersect — SURFACE: MethodFnV2 ABI lacks parallel NativeKind \
         track (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as union \
         (kind-blind args, vw_equals/vw_clone replacements need kinded \
         slots). Phase-2c follow-up: extend MethodFnV2 with parallel \
         kind track."
            .to_string(),
    ))
}

/// v2 `except` — set difference of two arrays (deduplicated, order-preserving)
///
/// args: [array, other_array]
pub(crate) fn handle_except_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_union_v2` — kind-blind args, kind-
    // blind element iteration, kind-blind equality compare, kind-blind
    // retain into result.
    Err(VMError::NotImplemented(
        "except — SURFACE: MethodFnV2 ABI lacks parallel NativeKind \
         track (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as union \
         (kind-blind args, vw_equals/vw_clone replacements need kinded \
         slots). Phase-2c follow-up: extend MethodFnV2 with parallel \
         kind track."
            .to_string(),
    ))
}

/// v2 `unique` — deduplicate array elements (order-preserving)
///
/// args: [array]
pub(crate) fn handle_unique_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_union_v2` — kind-blind receiver,
    // kind-blind element iteration, kind-blind equality compare, kind-
    // blind retain into result. (Single-array variant; no `other`.)
    Err(VMError::NotImplemented(
        "unique — SURFACE: MethodFnV2 ABI lacks parallel NativeKind \
         track (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot dispatch on \
         receiver array kind, iterate element kinds, equality-compare \
         across kinds, or retain into a kinded result buffer. Phase-2c \
         follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

/// v2 `distinct` — alias for `unique`
///
/// args: [array]
pub(crate) fn handle_distinct_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_unique_v2` (the legacy body
    // delegated to it). Surface the same ABI gap directly here — the
    // kind track Phase-2c follow-up unblocks both.
    Err(VMError::NotImplemented(
        "distinct — SURFACE: MethodFnV2 ABI lacks parallel NativeKind \
         track (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as unique. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

/// v2 `distinctBy` — deduplicate by a key function (order-preserving)
///
/// args: [array, key_fn]
pub(crate) fn handle_distinct_by_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: the MethodFnV2 ABI's `args: &mut [u64]` is kind-blind,
    // and this handler additionally needs:
    //
    //   1. To distinguish args[0] (array receiver) from args[1]
    //      (closure / fn-id) by kind — `Ptr(HeapKind::TypedArray)` vs.
    //      a callable kind. The legacy body relied on `ValueWord`
    //      probing (forbidden §2.7.7 #7).
    //   2. To call back into `op_call_value` with kinded (callee, arg,
    //      arg_count) slots for the key function — args[1] is kind-
    //      blind in the current ABI.
    //   3. To equality-compare returned keys across heterogeneous
    //      kinds (the key function may return any type). Legacy body
    //      used `vw_equals` (forbidden); kinded replacement needs a
    //      per-`NativeKind` equality dispatch.
    //   4. To either retain the source element into the result buffer
    //      (`vw_clone` — forbidden §2.7.7 #8) or release the
    //      duplicate-key call-return ref (`vw_drop` — forbidden
    //      §2.7.7 #8). Both replaced by `clone_with_kind` /
    //      `drop_with_kind` which require kinded slots.
    //
    // Bool-defaulting any of the above is forbidden (§2.7.7 #9).
    Err(VMError::NotImplemented(
        "distinctBy — SURFACE: MethodFnV2 ABI lacks parallel NativeKind \
         track (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot distinguish \
         array receiver from key-fn arg, call back through op_call_value \
         with kinded slots, equality-compare returned keys, or run the \
         retain (`vw_clone`) / release (`vw_drop`) discipline (both \
         replaced by `clone_with_kind` / `drop_with_kind` which require \
         kinded slots). Phase-2c follow-up: extend MethodFnV2 with \
         parallel kind track."
            .to_string(),
    ))
}
