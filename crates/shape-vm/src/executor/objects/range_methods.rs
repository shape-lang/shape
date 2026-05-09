//! Native method handlers for Range values.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod
//! precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Range` is **not** a surviving `HeapKind` variant per ADR-006 §2.3
//! trim (`crates/shape-value/src/heap_variants.rs`); the
//! `HeapValue::Range { start: Option<Box<ValueWord>>, end:
//! Option<Box<ValueWord>>, .. }` payload depended on the deleted
//! `ValueWord` for cross-kind range bounds (a range over `int` vs
//! `Decimal` vs `BigInt` vs an open range). The kinded equivalent
//! requires an ADR-006 follow-up on the `HeapValue::Range` payload shape
//! (the same surface that `objects/mod.rs::op_make_range` documents).
//!
//! The pre-Wave-6 implementation used the deleted `ValueWord::as_range`,
//! `value_word::vw_from_*` constructors, and
//! `raw_helpers::{extract_number_coerce, extract_range, type_error}`
//! (the entire `extract_*` family was deleted in cluster D-raw-helpers).
//! Per playbook §4 #1 / #9 a Bool-default kinded shim is forbidden; per
//! §7.4 the correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Range.{}(): Range is not a surviving HeapKind variant per \
         ADR-006 §2.3 trim; needs cross-kind Range payload redesign \
         (Option<Arc<...>> per element kind, Phase 2c). MethodHandler ABI \
         also needs kinded migration (cluster E-builtins-backlog).",
        method
    ))
}

/// `range.contains(value)` — check if a numeric value is within the range.
pub fn range_contains(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("contains"))
}

/// `range.toArray()` — materialize the range into an array of integers.
pub fn range_to_array(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toArray"))
}
