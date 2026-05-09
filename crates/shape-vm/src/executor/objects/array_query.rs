//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every,
//! any, all, single, take_while, skip_while, for_each
//!
//! ## Wave-δ `MR-array-basic-query` body migration (playbook §10 / §7 REVISED)
//!
//! Wave-γ `G-method-fn-v2-abi` (merge `5091cba`) flipped `MethodFnV2` to
//! the kinded ABI per ADR-006 §2.7.10 / Q11. This file's handlers split
//! into two classes:
//!
//! 1. **Value-search methods** (`indexOf`, `includes`) — take a receiver
//!    array + a search value; no closure dispatch. **Migrated** to
//!    real bodies below: receiver via `Ptr(HeapKind::TypedArray)` +
//!    `Arc::into_raw(Arc<TypedArrayData>)`; element comparison
//!    dispatches on the receiver's `TypedArrayData` variant cross-checked
//!    against the search value's `NativeKind`. CLAUDE.md "No runtime
//!    coercion" — kind mismatch implies "not present" without coercion.
//!
//! 2. **Higher-order closure-callback methods** (`where`, `select`,
//!    `find`, `findIndex`, `some`, `every`, `any`, `all`, `single`,
//!    `takeWhile`, `skipWhile`, `forEach`) — invoke a user closure per
//!    element. The architectural prerequisite is the kinded
//!    closure-call dispatch (`call_value_immediate_*` family in
//!    `call_convention.rs`). Today every kinded variant of those is
//!    `todo!("phase-2c — ADR-006 §2.7.8 cluster B-round-2: ... rebuild
//!    pending")`; the kind-threaded callee/args/return ABI is Wave-10
//!    JIT-trampoline + B-round-2 territory. **Surface** with a
//!    closure-call-gap reason per playbook §7 REVISED rather than
//!    paper over with a forbidden-pattern call into the deleted ABI.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers — value-search support (no shim usage; pure dispatch on
// TypedArrayData variant + NativeKind cross-check)
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow the `&TypedArrayData` referenced by a `Ptr(HeapKind::TypedArray)`-
/// kinded receiver. Mirrors `array_basic::typed_array_ref`.
#[inline]
fn typed_array_ref<'a>(slot: &'a KindedSlot) -> Result<&'a TypedArrayData, VMError> {
    match slot.kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            let bits = slot.slot.raw();
            // SAFETY: per the kinded-ABI contract, `Ptr(HeapKind::TypedArray)`
            // bits are `Arc::into_raw::<TypedArrayData>` and the dispatch
            // shell owns one strong-count share for the call duration.
            Ok(unsafe { &*(bits as *const TypedArrayData) })
        }
        _ => Err(VMError::TypeError {
            expected: "Array",
            got: "non-array",
        }),
    }
}

/// Linear search for `needle` in `arr`. Returns `Some(idx)` on match or
/// `None` if absent / kind mismatch (no runtime coercion per
/// CLAUDE.md). Element kind on the receiver is captured per playbook §2
/// (typed-array element-source rule).
fn find_first_index(arr: &TypedArrayData, needle: &KindedSlot) -> Result<Option<usize>, VMError> {
    match arr {
        TypedArrayData::I64(buf) => {
            let Some(target) = needle.as_i64() else {
                return Ok(None);
            };
            Ok(buf.data.iter().position(|&v| v == target))
        }
        TypedArrayData::F64(buf) => {
            let Some(target) = needle.as_f64() else {
                return Ok(None);
            };
            // Float equality follows the IEEE-754 == relation as in the
            // pre-Wave-6.5 body — NaN never compares equal to itself.
            Ok(buf.data.iter().position(|&v| v == target))
        }
        TypedArrayData::Bool(buf) => {
            let Some(target) = needle.as_bool() else {
                return Ok(None);
            };
            let target_bits: u8 = if target { 1 } else { 0 };
            Ok(buf.data.iter().position(|&v| v == target_bits))
        }
        TypedArrayData::String(buf) => {
            let Some(target) = needle.as_str() else {
                return Ok(None);
            };
            Ok(buf.data.iter().position(|s| s.as_str() == target))
        }
        // Narrow-int / matrix / float-slice / heap-value-backed receivers
        // need element-kind-aware comparison the §2.7.6 / Q8 per-variant
        // constructor matrix completes in Wave-γ-followup. Surface
        // explicitly per playbook §7 REVISED.
        other => Err(VMError::NotImplemented(format!(
            "Array.indexOf/includes: TypedArrayData variant {} — Wave-γ-\
             followup. Element-kind-aware comparison needs narrow-int \
             width, heap-value-backed equality, matrix shape, and \
             float-slice view comparison.",
            other.type_name()
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (kinded ABI per ADR-006 §2.7.10 / Q11) handlers
// ═══════════════════════════════════════════════════════════════════════════
//
// Two classes:
//
// 1. Value-search methods (`indexOf`, `includes`) — real bodies below.
// 2. Higher-order closure-callback methods — surface with closure-call
//    gap reason. The kinded `call_value_immediate_*` family in
//    `call_convention.rs` is `todo!("phase-2c — ADR-006 §2.7.8 cluster
//    B-round-2: ... rebuild pending")`.

const CLOSURE_DISPATCH_SURFACE: &str =
    "SURFACE: closure-callback dispatch is Wave-10 JIT-trampoline + \
     B-round-2 territory. The kinded `call_value_immediate_nb` / \
     `call_value_immediate_raw` / `call_function_with_raw_args` / \
     `jit_trampoline_call_closure` paths in `call_convention.rs` are all \
     `todo!(\"phase-2c — ADR-006 §2.7.8 cluster B-round-2\")` pending the \
     kind-threaded callee/upvalues/args/return ABI. Per playbook §7 \
     REVISED + §8 cross-cluster cascade, this handler surfaces back \
     rather than paper over with a forbidden-pattern call into the \
     deleted kind-blind ABI (CLAUDE.md \"Forbidden Patterns\" #1).";

/// `arr.where(predicate)` — filter via closure callback.
pub(crate) fn handle_where_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.where — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.select(projector)` — project via closure callback.
pub(crate) fn handle_select_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.select — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.find(predicate)` — first matching element via closure callback.
pub(crate) fn handle_find_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.find — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.findIndex(predicate)` — index of first matching element via
/// closure callback.
pub(crate) fn handle_find_index_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.findIndex — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.indexOf(value)` — first index of `value`, or `-1` if absent. No
/// closure dispatch; pure value-search.
pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::argument_count_error("indexOf", 1, 0));
    }
    let arr = typed_array_ref(&args[0])?;
    let idx = find_first_index(arr, &args[1])?;
    Ok(KindedSlot::from_int(idx.map(|i| i as i64).unwrap_or(-1)))
}

/// `arr.includes(value)` — true if `value` is present. No closure
/// dispatch; pure value-search.
pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::argument_count_error("includes", 1, 0));
    }
    let arr = typed_array_ref(&args[0])?;
    let idx = find_first_index(arr, &args[1])?;
    Ok(KindedSlot::from_bool(idx.is_some()))
}

/// `arr.some(predicate)` — true if any element satisfies the closure
/// predicate.
pub(crate) fn handle_some_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.some — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.every(predicate)` — true if all elements satisfy the closure
/// predicate.
pub(crate) fn handle_every_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.every — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.any(predicate)` — alias for `some` in the SQL-like query DSL.
pub(crate) fn handle_any_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.any — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.all(predicate)` — alias for `every` in the SQL-like query DSL.
pub(crate) fn handle_all_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.all — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.single(predicate)` — exactly-one-match via closure predicate;
/// errors on zero or multiple matches.
pub(crate) fn handle_single_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.single — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.takeWhile(predicate)` — prefix where the closure predicate
/// holds.
pub(crate) fn handle_take_while_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.takeWhile — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.skipWhile(predicate)` — suffix from the first element where the
/// closure predicate fails.
pub(crate) fn handle_skip_while_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.skipWhile — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}

/// `arr.forEach(action)` — invoke the closure on every element.
pub(crate) fn handle_for_each_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "Array.forEach — {}",
        CLOSURE_DISPATCH_SURFACE
    )))
}
