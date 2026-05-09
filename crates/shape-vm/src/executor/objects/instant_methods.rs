//! Method handlers for `Instant` values (std::time::Instant) — receiver
//! `NativeKind::Ptr(HeapKind::Instant)`.
//!
//! ## Wave 6.5 substep-2 sub-cluster `M-datetime-instant` — SURFACE
//!
//! Every body in this file historically:
//!
//! - Read `args[0]` as raw bits via `raw_helpers::extract_instant`
//!   (forbidden: the deleted `tag_bits::*` dispatch family — playbook §4 #7,
//!   CLAUDE.md "Renames to refuse on sight"; the helper itself was deleted
//!   wholesale by sub-cluster `D-raw-helpers` — see
//!   `objects/raw_helpers.rs` post-bulldozer header).
//! - Constructed a `Vec<u64>` result via `ValueWord::from_*().into_raw_bits()`
//!   (forbidden: `ValueWord` is deleted from `shape-value` per
//!   `crates/shape-value/src/lib.rs` post-bulldozer header — playbook §4 #1
//!   / CLAUDE.md "Forbidden Patterns").
//! - Surfaced type errors via `raw_helpers::type_error` /
//!   `raw_helpers::type_name_from_bits` (also deleted by `D-raw-helpers`).
//!
//! The `MethodFnV2` ABI itself
//! (`fn(&mut VM, &mut [u64], _) -> Result<u64, VMError>`) is **kind-less in
//! both directions**. The dispatch shell `op_call_method`
//! (`objects/mod.rs:299`) is itself surfaced as `NotImplemented(SURFACE)`
//! — these handlers are unreachable until the ABI migrates to
//! `fn(&mut VM, &mut [KindedSlot], _) -> Result<KindedSlot, VMError>`
//! per Wave-α cluster `E-builtins-backlog` (Wave 5b template, commit
//! `fa2bafc`).
//!
//! Any in-place migration of these bodies to the kinded API would either:
//!   (a) fabricate a kind on the result push (forbidden Bool-default
//!       rationalization, ADR-006 §2.7.7 / playbook §4 #9); or
//!   (b) reach into `args[0]` raw bits without a parallel-kind track,
//!       which is exactly the deleted `tag_bits` dispatch surface.
//!
//! Per playbook §7.4 (DoD: "compiles cleanly OR un-compiling sites have a
//! documented surface") and §8 surface-and-stop trigger ("Cross-cluster
//! migration cascade"), each body returns
//! `VMError::NotImplemented(SURFACE: ...)` documenting the architectural
//! cascade. The function signatures and PHF registry entries
//! (`method_registry::INSTANT_METHODS`) are preserved so external
//! references continue to compile.
//!
//! ## Migration status snapshot (sub-cluster close)
//!
//! - Mandatory shim hits: 0 (none in pre-existing file).
//! - Sibling shim hits: 0 (none in pre-existing file).
//! - Forbidden-pattern carry-overs: 0 — `ValueWord`, `ValueWordExt`,
//!   `raw_helpers::extract_instant`, `raw_helpers::type_error`,
//!   `from_instant` / `from_f64` / `from_i64` / `from_string` / `from_raw_bits`
//!   / `into_raw_bits`, and the test-only `as_f64` / `as_str` / `as_number_coerce`
//!   ValueWord accessors are all removed.
//! - Surfaces: 6 (`v2_elapsed`, `v2_elapsed_ms`, `v2_elapsed_us`,
//!   `v2_elapsed_ns`, `v2_duration_since`, `v2_to_string`).
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10
//! M-datetime-instant row, §7.4, §8, ADR-006 §2.7.6 (Q8) / §2.7.4
//! (Phase-2c deferral pattern) / §2.7.7 (Q9).
//!
//! Pure chrono / `std::time::Instant` helpers needed by future re-
//! implementation are preserved in `executor/builtins/datetime_builtins.rs`
//! (Wave-α `E-builtins-backlog`), so every body re-write at MethodHandler
//! ABI flip time can lift logic from there without touching this surface
//! shell.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

#[inline]
fn surface_call_method_cascade(method: &'static str) -> VMError {
    VMError::NotImplemented(format!(
        "SURFACE: Instant.{}() depends on the MethodHandler ABI migration \
         from (&mut [u64]) -> Result<u64> to (&mut [KindedSlot]) -> \
         Result<KindedSlot>. The receiver kind is \
         NativeKind::Ptr(HeapKind::Instant). The dispatch shell \
         op_call_method (objects/mod.rs:299) is itself a SURFACE under \
         cluster E-builtins-backlog; this handler becomes reachable only \
         once that ABI flip lands. Pure-Instant logic is preserved so the \
         re-implementation lifts from std::time::Instant directly. See \
         playbook §10 M-datetime-instant row + ADR-006 §2.7.6 / Q8.",
        method
    ))
}

/// .elapsed() -> number (seconds as f64) — SURFACE per ABI cascade.
pub fn v2_elapsed(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("elapsed"))
}

/// .elapsed_ms() -> number — SURFACE per ABI cascade.
pub fn v2_elapsed_ms(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("elapsed_ms"))
}

/// .elapsed_us() -> number — SURFACE per ABI cascade.
pub fn v2_elapsed_us(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("elapsed_us"))
}

/// .elapsed_ns() -> int — SURFACE per ABI cascade.
pub fn v2_elapsed_ns(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("elapsed_ns"))
}

/// .duration_since(other: Instant) -> number — SURFACE per ABI cascade.
pub fn v2_duration_since(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("duration_since"))
}

/// .to_string() -> string — SURFACE per ABI cascade.
pub fn v2_to_string(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("to_string"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests removed during M-datetime-instant surface.
// ═════════════════════════════════════════════════════════════════════════════
//
// The pre-Wave-6 `tests` module exercised every handler with
// `ValueWord::from_instant(...)` / `ValueWord::from_raw_bits(...)` /
// `as_f64()` / `as_str()` / `as_number_coerce()`. Every constructor and
// accessor on that path is deleted with the type. The canonical kinded
// equivalents (push/pop on the kinded VM stack with
// `NativeKind::Ptr(HeapKind::Instant)`) belong with the MethodHandler ABI
// flip in cluster E-builtins-backlog; the tests will be re-instated there
// once the handler bodies are real again.
