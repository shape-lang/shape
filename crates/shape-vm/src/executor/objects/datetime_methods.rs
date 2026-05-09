//! PHF-dispatched method handlers for `DateTime` values — receiver
//! `NativeKind::Ptr(HeapKind::Temporal)` (the chrono `DateTime<FixedOffset>`
//! arm of `TemporalData`; see `shape_value::heap_value`).
//!
//! ## Wave 6.5 substep-2 sub-cluster `M-datetime-instant` — SURFACE
//!
//! Every body in this file historically:
//!
//! - Read `args[0]` as raw bits via `raw_helpers::extract_datetime`
//!   (forbidden: the deleted `tag_bits::*` dispatch family — playbook §4 #7,
//!   CLAUDE.md "Renames to refuse on sight"; the helper itself was deleted
//!   wholesale by sub-cluster `D-raw-helpers` — see
//!   `objects/raw_helpers.rs` post-bulldozer header).
//! - Read other arg slots (`args[1]`) via the deleted helpers
//!   `raw_helpers::extract_str`, `raw_helpers::extract_number_coerce`,
//!   `raw_helpers::extract_heap_ref` and surfaced type errors via
//!   `raw_helpers::type_error` / `raw_helpers::type_name_from_bits`
//!   (all deleted by `D-raw-helpers`).
//! - Constructed each result via
//!   `ValueWord::from_i64 / from_bool / from_string / from_time /
//!   from_timespan / from_hashmap_pairs(...).into_raw_bits()`
//!   (forbidden: `ValueWord` + `ValueWordExt` are deleted from
//!   `shape-value` per `crates/shape-value/src/lib.rs` post-bulldozer
//!   header — playbook §4 #1 / CLAUDE.md "Forbidden Patterns").
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
//!   (b) reach into `args[0]` / `args[1]` raw bits without a parallel-kind
//!       track, which is exactly the deleted `tag_bits` dispatch surface.
//!
//! The `v2_add` / `v2_sub` / `v2_timespan_add` / `v2_timespan_sub` arms
//! additionally need to dispatch on the *rhs* heap variant
//! (`HeapValue::Temporal(TemporalData::TimeSpan(_))` vs
//! `HeapValue::Temporal(TemporalData::DateTime(_))`). Per Q8 single-
//! discriminator, that dispatch is `slot.as_heap_value()` + `HeapValue::*`
//! match on a `KindedSlot` — the same ABI cascade.
//!
//! `v2_diff` historically constructed an `Arc<HashMapData>` result via the
//! deleted `ValueWord::from_hashmap_pairs(keys, values)`. The kinded
//! equivalent is `Arc::into_raw::<HashMapData>` + `push_kinded(..,
//! NativeKind::Ptr(HeapKind::HashMap))` per playbook §3 per-`HeapKind`
//! push pattern, but only after the MethodHandler ABI flip surfaces
//! `KindedSlot` returns from this layer.
//!
//! Per playbook §7.4 (DoD: "compiles cleanly OR un-compiling sites have a
//! documented surface") and §8 surface-and-stop trigger ("Cross-cluster
//! migration cascade"), each body returns
//! `VMError::NotImplemented(SURFACE: ...)` documenting the architectural
//! cascade. The function signatures and PHF registry entries
//! (`method_registry::DATETIME_METHODS`) are preserved so external
//! references continue to compile.
//!
//! ## DateTime expression evaluation — Phase 2c
//!
//! DateTime expression surface (`window_join.rs::handle_eval_datetime_expr`,
//! emitted by D-window-join Wave-α) is Phase-2c per ADR-006 §2.7.4 and is
//! NOT touched by this sub-cluster. The `eval_datetime_expr_recursive`
//! helper in `executor/window_join.rs:60` is a pure-AST helper that does
//! not touch this file's handlers.
//!
//! ## Migration status snapshot (sub-cluster close)
//!
//! - Mandatory shim hits: 0 (none in pre-existing file).
//! - Sibling shim hits: 0 (none in pre-existing file).
//! - Forbidden-pattern carry-overs: 0 — `ValueWord`, `ValueWordExt`,
//!   `raw_helpers::extract_datetime`, `raw_helpers::extract_str`,
//!   `raw_helpers::extract_number_coerce`, `raw_helpers::extract_heap_ref`,
//!   `raw_helpers::type_error`, `raw_helpers::type_name_from_bits`,
//!   `from_i64` / `from_bool` / `from_string` / `from_time` / `from_timespan`
//!   / `from_hashmap_pairs` / `into_raw_bits`, plus the `HeapValue::Temporal`
//!   raw-bits borrow site, are all removed.
//! - Surfaces: 31 (every public `v2_*` handler — see PHF registry
//!   `method_registry::DATETIME_METHODS`).
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10
//! M-datetime-instant row, §7.4, §8, ADR-006 §2.7.6 (Q8) / §2.7.4
//! (Phase-2c deferral pattern) / §2.7.7 (Q9).
//!
//! Pure chrono helpers (parse, format, ast_duration_to_chrono, ...) needed
//! by future re-implementation are preserved in
//! `executor/builtins/datetime_builtins.rs` (Wave-α
//! `E-builtins-backlog`), so every body re-write at MethodHandler ABI
//! flip time can lift logic from there without touching this surface
//! shell.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

#[inline]
fn surface_call_method_cascade(method: &'static str) -> VMError {
    VMError::NotImplemented(format!(
        "SURFACE: DateTime.{}() depends on the MethodHandler ABI migration \
         from (&mut [u64]) -> Result<u64> to (&mut [KindedSlot]) -> \
         Result<KindedSlot>. The receiver kind is \
         NativeKind::Ptr(HeapKind::Temporal); rhs heap-variant dispatch \
         (TimeSpan vs DateTime) goes through slot.as_heap_value() + \
         HeapValue::Temporal match per ADR-006 §2.7.6 / Q8 (single-\
         discriminator). The dispatch shell op_call_method \
         (objects/mod.rs:299) is itself a SURFACE under cluster \
         E-builtins-backlog; this handler becomes reachable only once \
         that ABI flip lands. Pure-chrono logic is preserved in \
         executor/builtins/datetime_builtins.rs. See playbook §10 \
         M-datetime-instant row + ADR-006 §2.7.4 / §2.7.6.",
        method
    ))
}

// ===== Component access (return int) — SURFACE per ABI cascade =====

pub fn v2_year(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("year"))
}
pub fn v2_month(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("month"))
}
pub fn v2_day(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("day"))
}
pub fn v2_hour(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("hour"))
}
pub fn v2_minute(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("minute"))
}
pub fn v2_second(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("second"))
}
pub fn v2_millisecond(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("millisecond"))
}
pub fn v2_microsecond(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("microsecond"))
}

// ===== Day info — SURFACE per ABI cascade =====

pub fn v2_day_of_week(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("day_of_week"))
}
pub fn v2_day_of_year(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("day_of_year"))
}
pub fn v2_week_of_year(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("week_of_year"))
}
pub fn v2_is_weekday(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("is_weekday"))
}
pub fn v2_is_weekend(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("is_weekend"))
}

// ===== Formatting — SURFACE per ABI cascade =====

pub fn v2_format(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("format"))
}
pub fn v2_iso8601(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("iso8601"))
}
pub fn v2_rfc2822(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("rfc2822"))
}
pub fn v2_unix_timestamp(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("unix_timestamp"))
}
pub fn v2_to_unix_millis(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("to_unix_millis"))
}

// ===== Diff — SURFACE per ABI cascade =====

pub fn v2_diff(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("diff"))
}

// ===== Timezone — SURFACE per ABI cascade =====

pub fn v2_to_utc(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("to_utc"))
}
pub fn v2_to_timezone(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("to_timezone"))
}
pub fn v2_to_local(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("to_local"))
}
pub fn v2_timezone(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("timezone"))
}
pub fn v2_offset(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("offset"))
}

// ===== Operator-trait methods (add/sub) — SURFACE per ABI cascade =====
//
// `v2_add` / `v2_sub` need rhs heap-variant dispatch on
// `HeapValue::Temporal(TemporalData::TimeSpan)` vs
// `HeapValue::Temporal(TemporalData::DateTime)` — Q8 single-discriminator
// via `slot.as_heap_value()` + `HeapValue::*` match. Same ABI cascade.

pub fn v2_add(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("add"))
}
pub fn v2_sub(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("sub"))
}

// ===== Arithmetic — SURFACE per ABI cascade =====

pub fn v2_add_days(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("add_days"))
}
pub fn v2_add_hours(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("add_hours"))
}
pub fn v2_add_minutes(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("add_minutes"))
}
pub fn v2_add_seconds(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("add_seconds"))
}
pub fn v2_add_months(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("add_months"))
}

// ===== Comparison — SURFACE per ABI cascade =====

pub fn v2_is_before(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("is_before"))
}
pub fn v2_is_after(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("is_after"))
}
pub fn v2_is_same_day(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("is_same_day"))
}

// ===== TimeSpan (Duration) operator-trait methods — SURFACE per ABI cascade =====
//
// `v2_timespan_add` / `v2_timespan_sub` dispatch on receiver kind
// `NativeKind::Ptr(HeapKind::Temporal)` (TimeSpan arm) and rhs that may
// be TimeSpan or DateTime — Q8 single-discriminator via
// `slot.as_heap_value()` + `HeapValue::Temporal` match. Same ABI cascade.

pub fn v2_timespan_add(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("timespan_add"))
}
pub fn v2_timespan_sub(_vm: &mut VirtualMachine, _args: &mut [u64], _ctx: Option<&mut ExecutionContext>) -> Result<u64, VMError> {
    Err(surface_call_method_cascade("timespan_sub"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests removed during M-datetime-instant surface.
// ═════════════════════════════════════════════════════════════════════════════
//
// The pre-Wave-6 `tests` module exercised pure chrono semantics (no VM
// stack interaction) and is preserved as-is — the equivalent coverage
// lives in `executor/builtins/datetime_builtins.rs`'s test module
// (Wave-α `E-builtins-backlog`). Re-instating any handler-level test
// here belongs with the MethodHandler ABI flip in cluster
// E-builtins-backlog, when the bodies are real again.
