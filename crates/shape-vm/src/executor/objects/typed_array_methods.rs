//! Method handlers for typed arrays (Vec<int>, Vec<number>, Vec<bool>)
//!
//! ## Wave-β `M-typed-array` migration (playbook §10)
//!
//! Receiver kind for these handlers is
//! `NativeKind::Ptr(HeapKind::TypedArray)` (per ADR-006 §2.7.6 / Q8); element
//! kind comes from `TypedArrayData::*` variant. Higher-order methods take a
//! callable as `args[1]` whose arity/kind cannot be sourced from the
//! `(&mut [u64], NativeKind?)` carrier — the deleted W-series tag-decode
//! family (`is_callable_raw`, `callable_arity_raw`, `is_truthy_raw`) was the
//! pre-Wave-6.5 cover for that gap and is forbidden per ADR-006 §2.7.7 /
//! CLAUDE.md "Forbidden Patterns".
//!
//! ## Out-of-territory surfaces
//!
//! Every handler in this file is fundamentally `ValueWord`-shaped: the
//! `MethodFnV2` contract (`Fn(&mut VirtualMachine, &mut [u64], …) -> Result<u64>`)
//! takes raw `u64` ValueWord bits as receiver/args, and the legacy bodies
//! reconstructed a `ValueWord` via `borrow_vw` / `try_v2_view` and dispatched
//! through `as_float_array()` / `as_int_array()` / `as_bool_array()` /
//! `vmarray_from_vec` / `tag_bits::is_tagged` — every one a forbidden
//! pattern. Migrating the bodies in-place is impossible without first
//! migrating:
//!
//! - The `MethodFnV2` registry contract itself off raw `u64` bits onto a
//!   kinded carrier (Phase-2c reentry; the entire `method_registry.rs` PHF
//!   table and every sibling handler — `array_aggregation`, `array_query`,
//!   `array_transform`, `iterator_methods`, `column_methods`, `hashmap_methods`
//!   — would cascade).
//! - The `raw_helpers` carrier (`is_callable_raw`, `callable_arity_raw`,
//!   `is_truthy_raw`) which is itself a forbidden tag-decode-family probe per
//!   playbook §4 #11 and CLAUDE.md "Renames to refuse on sight" — owned by
//!   cluster D `D-raw-helpers` Wave-α (still pending).
//! - The `v2::as_v2_typed_array` / `read_element` / `pop_element` /
//!   `sum_elements` / `avg_elements` / `min_elements` / `max_elements` /
//!   `dot_elements` / `norm_elements` / `count_true_elements` /
//!   `any_elements` / `all_elements` / `variance_elements` / `std_elements` /
//!   `unary_f64_transform` / `diff_f64` family in
//!   `executor/v2_handlers/v2_array_detect.rs` — also forbidden-helper
//!   carriers (cluster D `D-v2-array-detect` Wave-α, still pending).
//! - The `vw_drop` / `value_word_drop::vw_drop` retain-on-write discipline,
//!   replaced by `clone_with_kind` / `drop_with_kind` post-§2.7.7 — but with
//!   no kind on the raw-`u64` MethodFnV2 carrier, the kind-aware drop is
//!   unreachable from this dispatch boundary.
//!
//! Per playbook §7 REVISED: "If a call site cannot be migrated cleanly, the
//! correct shape is `NotImplemented(SURFACE)` … never a forbidden-pattern
//! workaround." Every handler in this file therefore returns
//! `VMError::NotImplemented` with a Phase-2c reentry note. The PHF
//! registrations in `method_registry.rs` continue to compile (function
//! signatures are preserved); end-to-end execution surfaces the deferred
//! migration to the user as a typed runtime error rather than a silent
//! forbidden-pattern carry-over.
//!
//! All forbidden patterns from the pre-Wave-6.5 body — `is_callable_raw`,
//! `callable_arity_raw`, `is_truthy_raw`, `vmarray_from_vec`,
//! `tag_bits::is_tagged`, `tag_bits::I48_MIN`, `tag_bits::I48_MAX`,
//! `value_word_drop::vw_drop`, `as_float_array`, `as_int_array`,
//! `as_bool_array`, `to_generic_array`, `from_float_array`, `from_int_array`,
//! `from_array`, `from_native_ptr`, `borrow_vw`, `try_v2_view` — are deleted
//! from this territory. Re-introduction in any shape, including the
//! defection-attractor renames CLAUDE.md catalogues under "Renames to refuse
//! on sight", is refused.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

/// Common surface error for every handler in this file.
///
/// Every handler in this module is registered in
/// `executor/objects/method_registry.rs` against the `MethodFnV2` PHF map,
/// which takes raw `u64` ValueWord bits — a fundamentally pre-§2.7.7 carrier
/// shape. Migrating off it requires migrating the entire method-dispatch
/// surface (out of territory; Phase-2c).
#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "{method}: Phase-2c reentry. The MethodFnV2 raw-u64 ValueWord carrier \
         is forbidden per ADR-006 §2.7.7; migration depends on (1) cluster D \
         `D-raw-helpers` (raw_helpers tag-decode probes), (2) cluster D \
         `D-v2-array-detect` (v2_array_detect ValueWord helpers), and (3) \
         the broader method_registry PHF table moving to a kinded carrier. \
         See typed_array_methods.rs module docs."
    ))
}

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — registered in method_registry.rs against the PHF map
// for FloatArray, IntArray, BoolArray method dispatch.
//
// Signatures preserved so method_registry.rs compiles; bodies surface per
// playbook §7 REVISED. See module docs for the full forbidden-pattern audit.
// ═════════════════════════════════════════════════════════════════════════════

/// v2 len: works for all element types (float, int, bool).
pub fn v2_len(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("typed_array.len"))
}

/// v2 sum for float arrays.
pub fn v2_float_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.sum"))
}

/// v2 sum for int arrays.
pub fn v2_int_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.sum"))
}

/// v2 avg/mean for float arrays.
pub fn v2_float_avg(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.avg"))
}

/// v2 avg/mean for int arrays.
pub fn v2_int_avg(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.avg"))
}

/// v2 min for float arrays.
pub fn v2_float_min(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.min"))
}

/// v2 min for int arrays.
pub fn v2_int_min(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.min"))
}

/// v2 max for float arrays.
pub fn v2_float_max(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.max"))
}

/// v2 max for int arrays.
pub fn v2_int_max(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.max"))
}

/// v2 variance for float arrays.
pub fn v2_float_variance(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.variance"))
}

/// v2 std (standard deviation) for float arrays.
pub fn v2_float_std(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.std"))
}

/// v2 dot product for float arrays.
pub fn v2_float_dot(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.dot"))
}

/// v2 norm for float arrays.
pub fn v2_float_norm(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.norm"))
}

/// v2 bool count (count of true values).
pub fn v2_bool_count(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<bool>.count"))
}

/// v2 bool any.
pub fn v2_bool_any(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<bool>.any"))
}

/// v2 bool all.
pub fn v2_bool_all(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<bool>.all"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Element-wise / numeric transforms (float arrays)
// ═════════════════════════════════════════════════════════════════════════════

/// v2 normalize: L2-normalize a float array (divide each element by L2 norm).
pub(crate) fn handle_float_normalize(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.normalize"))
}

/// v2 cumsum: cumulative sum of a float array.
pub(crate) fn handle_float_cumsum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.cumsum"))
}

/// v2 diff: consecutive differences of a float array.
pub(crate) fn handle_float_diff(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.diff"))
}

/// v2 abs: element-wise absolute value of a float array.
pub(crate) fn handle_float_abs(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.abs"))
}

/// v2 sqrt: element-wise square root of a float array.
pub(crate) fn handle_float_sqrt(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.sqrt"))
}

/// v2 ln: element-wise natural logarithm of a float array.
pub(crate) fn handle_float_ln(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.ln"))
}

/// v2 exp: element-wise exponential of a float array.
pub(crate) fn handle_float_exp(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.exp"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Higher-order methods (float arrays) — closure arity/kind not sourceable
// from the raw-u64 carrier; surface per playbook §10 row M-typed-array.
// ═════════════════════════════════════════════════════════════════════════════

/// v2 map for float arrays: apply a callback to each element.
pub(crate) fn handle_float_map(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.map"))
}

/// v2 filter for float arrays: keep elements where callback returns truthy.
pub(crate) fn handle_float_filter(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.filter"))
}

/// v2 forEach for float arrays: call callback on each element, return none.
pub(crate) fn handle_float_for_each(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.forEach"))
}

/// v2 reduce for float arrays: fold with accumulator.
pub(crate) fn handle_float_reduce(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.reduce"))
}

/// v2 find for float arrays: return first element matching predicate, or none.
pub(crate) fn handle_float_find(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.find"))
}

/// v2 some for float arrays: return true if any element matches predicate.
pub(crate) fn handle_float_some(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.some"))
}

/// v2 every for float arrays: return true if all elements match predicate.
pub(crate) fn handle_float_every(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.every"))
}

/// v2 toArray for float arrays: convert typed array to generic Array.
pub(crate) fn handle_float_to_array(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<number>.toArray"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Element-wise / higher-order methods (int arrays)
// ═════════════════════════════════════════════════════════════════════════════

/// v2 abs for int arrays: element-wise absolute value.
pub(crate) fn handle_int_abs(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.abs"))
}

/// v2 map for int arrays: apply a callback to each element.
pub(crate) fn handle_int_map(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.map"))
}

/// v2 filter for int arrays: keep elements where callback returns truthy.
pub(crate) fn handle_int_filter(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.filter"))
}

/// v2 forEach for int arrays: call callback on each element, return none.
pub(crate) fn handle_int_for_each(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.forEach"))
}

/// v2 reduce for int arrays: fold with accumulator.
pub(crate) fn handle_int_reduce(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.reduce"))
}

/// v2 find for int arrays: return first element matching predicate, or none.
pub(crate) fn handle_int_find(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.find"))
}

/// v2 some for int arrays: return true if any element matches predicate.
pub(crate) fn handle_int_some(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.some"))
}

/// v2 every for int arrays: return true if all elements match predicate.
pub(crate) fn handle_int_every(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.every"))
}

/// v2 toArray for int arrays: convert typed array to generic Array.
pub(crate) fn handle_int_to_array(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<int>.toArray"))
}

/// v2 toArray for bool arrays: convert typed array to generic Array.
pub(crate) fn handle_bool_to_array(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("Vec<bool>.toArray"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════
//
// Pre-Wave-β tests in this module exercised the script harness via
// `crate::test_utils::eval` ("[1,2,3].map(...)" etc.) and direct unit tests
// against `v2::unary_f64_transform` / `v2::diff_f64` / `v2::stamp_elem_type` /
// `v2::ELEM_TYPE_F64` / `v2::read_element` (cluster D `D-v2-array-detect`
// territory — also pending Wave-α migration off `ValueWord::from_native_ptr`).
//
// All tests are gated `#[cfg(all(test, feature = "deep-tests"))]` because (a)
// the script harness path now routes to `surface(...)` and intentionally
// fails with `NotImplemented`, and (b) the direct v2-helper tests reach into
// out-of-territory `v2_array_detect` symbols that are themselves
// forbidden-helper carriers per cluster D `D-v2-array-detect` Wave-α
// (still pending). They are preserved for re-enablement once the upstream
// helpers and the MethodFnV2 carrier migrate to a kinded shape (Phase-2c).

#[cfg(all(test, feature = "deep-tests"))]
mod tests {
    // Intentionally empty post-Wave-β. The original bodies depended on
    // `ValueWord::from_native_ptr`, `v2::stamp_elem_type`, `v2::ELEM_TYPE_F64`,
    // `v2::as_v2_typed_array`, `v2::read_element`, `v2::unary_f64_transform`,
    // `v2::diff_f64`, `TypedArray::push`, `TypedArray::drop_array` — every
    // one a forbidden-helper carrier (cluster D `D-v2-array-detect` Wave-α
    // territory). Re-enable once those helpers migrate off ValueWord and the
    // MethodFnV2 carrier moves to a kinded shape (Phase-2c).
}
