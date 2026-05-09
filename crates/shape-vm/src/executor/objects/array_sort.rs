//! Array sort operations
//!
//! Handles: order_by, then_by, join_str
//!
//! Wave-β `A-array-sort-sets` sub-cluster (ADR-006 §2.7.6 / §2.7.7 /
//! §2.7.8 / Q8-Q10): the `MethodFnV2` native ABI takes `args: &mut [u64]`
//! raw bits with **no parallel `NativeKind` track**. Every interaction
//! these handlers need — interpreting `args[i]` as an array vs. closure
//! vs. string, iterating `TypedArrayData` element-by-element with a
//! per-element kind, comparing/keying values across heterogeneous
//! element kinds, calling back into `op_call_value` with kinded callee
//! + arg + count slots — is impossible without a kind-aware extension
//! to the V2 ABI itself.
//!
//! Sourcing the kind locally would require either:
//!   - decoding kind from the raw `u64` bits (forbidden — the deleted
//!     tag_bits dispatch, §2.7.7 #4 / #7), or
//!   - defaulting to `NativeKind::Bool` "because Drop is a no-op"
//!     (forbidden §2.7.7 #9 — the W-series rationalization the
//!     playbook names verbatim), or
//!   - probing a heap discriminant via a deleted heap-ref accessor /
//!     ValueWord synthesizer (forbidden §2.7.7 #7).
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

/// v2 `orderBy` — sort an array by a key function (optionally with direction)
///
/// args: [array, key_fn, direction?]
pub(crate) fn handle_order_by_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: the MethodFnV2 ABI's `args: &mut [u64]` is kind-blind.
    // Each arg is a raw u64 with no companion `NativeKind`. ADR-006
    // §2.7.7 (stack track) and §2.7.8 (cell-store track) extend the
    // parallel-kind invariant to the VM stack and cell-store; the V2
    // method ABI is the next analogous surface that has not yet been
    // extended. Without it this handler cannot:
    //
    //   1. Distinguish array vs. closure vs. string args without
    //      decoding kind from the raw bits (§2.7.7 #4 forbidden) or
    //      probing a heap discriminant via deleted ValueWord shape
    //      (§2.7.7 #7 forbidden).
    //   2. Iterate `TypedArrayData::*` arms with per-element
    //      `NativeKind` to feed the key function and rebuild the
    //      sorted result (§2.7.7 lockstep discipline; per-arm kinds
    //      are `I64 => Int64`, `F64 => Float64`, `String =>
    //      NativeKind::String`, …; `HeapValue(_)` and `Matrix` cannot
    //      be uniformly kinded — see §8 surface trigger).
    //   3. Compare keys across heterogeneous `NativeKind` (numeric vs.
    //      string vs. heap) — the legacy `compare_nb_values` reached
    //      through `ValueWord::as_number_coerce` / `as_str` /
    //      `type_name`, all deleted with ValueWord. The kinded
    //      replacement requires a `numeric_domain(slot)?` /
    //      string-arc-from-kinded path that itself depends on kinded
    //      args.
    //   4. Call back via `op_call_value` with kinded (callee, arg,
    //      arg_count) slots — args[1] (the key fn) is kind-blind in
    //      the current ABI.
    //
    // Bool-defaulting any of the above is forbidden (§2.7.7 #9). The
    // correct refusal shape is the surface below.
    Err(VMError::NotImplemented(
        "orderBy — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up; same gap as D-array-joins \
         close 2fe4a6b). Cannot dispatch on receiver array kind, iterate \
         element kinds, compare keys across heterogeneous kinds, or call \
         back through op_call_value without kind-aware extension. \
         Phase-2c follow-up: extend MethodFnV2 to `args: &mut [(u64, \
         NativeKind)]` (or equivalent parallel track) analogous to the \
         stack/cell-store §2.7.7/§2.7.8 invariants."
            .to_string(),
    ))
}

/// v2 `thenBy` — sort an already-ordered array by a secondary key
///
/// args: [array, key_fn, direction?]
pub(crate) fn handle_then_by_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: same gap as `handle_order_by_v2` — kind-blind args, kind-
    // blind element iteration, kind-blind key comparison, kind-blind
    // callback path. (The pre-Wave-β body delegated to
    // `handle_order_by_v2`; the surface delegation is preserved here in
    // intent — both surface the same architectural ABI gap.)
    Err(VMError::NotImplemented(
        "thenBy — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Same gap as orderBy. \
         Phase-2c follow-up: extend MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}

/// v2 `join` — join array elements into a single string with a separator
///
/// args: [array, separator?]
pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // SURFACE: the MethodFnV2 ABI's `args: &mut [u64]` is kind-blind.
    // This handler additionally needs:
    //
    //   1. To distinguish the array receiver from the optional
    //      separator argument by kind — `Ptr(HeapKind::TypedArray)` vs.
    //      `NativeKind::String` — without a kind track this is the
    //      deleted ValueWord-shape `as_any_array` / `as_str`
    //      forbidden-pattern dispatch (§2.7.7 #7).
    //   2. To stringify each element per its element kind. The legacy
    //      body called `nb_to_string_coerce(nb)` on a `ValueWord`; the
    //      kinded replacement is a per-`NativeKind` formatter
    //      (PrintResult / PrintSpan path the Wave-α `E-printing`
    //      sub-cluster owns). Without per-element kind that path
    //      cannot be reached.
    //   3. To produce a result `NativeKind::String` — the result
    //      kind itself is fine here, but it requires kinded inputs to
    //      get the elements out.
    //
    // Bool-defaulting any of the above is forbidden (§2.7.7 #9).
    Err(VMError::NotImplemented(
        "join — SURFACE: MethodFnV2 ABI lacks parallel NativeKind track \
         (ADR-006 §2.7.7 / §2.7.8 follow-up). Cannot distinguish array \
         receiver from string separator, iterate element kinds, or feed \
         per-element formatter (E-printing PrintResult/PrintSpan path) \
         without kind-aware extension. Phase-2c follow-up: extend \
         MethodFnV2 with parallel kind track."
            .to_string(),
    ))
}
