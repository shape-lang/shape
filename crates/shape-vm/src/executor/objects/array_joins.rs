//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15) — drive-by
//!
//! This file's pickup is a drive-by from ckpt-2's `array_transform.rs`
//! cross-module helper deletion: the imports at original lines 51-55
//! (`bump_closure_share / collect_homogeneous_results / element_kinded /
//! typed_array_arc_from_kinded / typed_array_len`) reference helpers that
//! were deleted in ckpt-2 (every one took `&TypedArrayData` or produced
//! `Arc<TypedArrayData>`). Per dispatch enumeration this file has 0
//! TypedArrayData:: refs in its own body, but the imports are E0432
//! cascade-broken.
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
//!
//! Public handler bodies (`handle_inner_join_v2 / handle_left_join_v2 /
//! handle_cross_join_v2`) are replaced with structured surface-and-stop
//! returning `VMError::NotImplemented`. Local helpers (`typed_array_arc /
//! closure_arg / key_eq`) and the deleted cross-module imports are
//! DELETED.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, the join body's element-read (left/right
//! TypedArrayData walks) migrates to the v2-raw `TypedArray<T>` flat-
//! struct carrier — per-T direct `*buf.data.add(i)` reads. The
//! closure-callback ABI (ADR-006 §2.7.11 / Q12 `vm.call_value_immediate_nb`)
//! and the join-shape algorithm (linear key-cache + cross-array match
//! emit + homogeneous-result collect) are unchanged and re-instate
//! once the receiver-shape migration lands across all element kinds.
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-3 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public handler in this file.
#[cold]
#[inline(never)]
fn ckpt3_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface \
         (drive-by from ckpt-2 cross-module helper deletion). \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The 5-import E0432 cluster at this file's deleted \
         `use` block referenced helpers (`bump_closure_share / \
         collect_homogeneous_results / element_kinded / \
         typed_array_arc_from_kinded / typed_array_len`) that were \
         deleted in ckpt-2 at array_transform.rs (every one took \
         `&TypedArrayData` or produced `Arc<TypedArrayData>`). The 3 \
         public handlers in this file dispatch the join shape (linear \
         right-key cache + cross-array match emit + homogeneous-result \
         collect) entirely through those deleted helpers. Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe; per-T monomorphization \
         landing across ckpt-3 (this file plus array_ops/typed_array_methods/\
         iterator_methods/array_sort/concat/property_access/array_query) + \
         ckpt-4 (Buf<T> / HeapValue::TypedArray arm / \
         HeapKind::TypedArray ordinal) + ckpt-5 (wire/json/marshal + \
         4-table lockstep) + ckpt-6 (JIT FFI). Closure-callback ABI \
         (ADR-006 §2.7.11 / Q12 `vm.call_value_immediate_nb`) is \
         unaffected and re-instates once receiver-shape migration lands. \
         Receiver kind: {kind}. UNREACHABLE until ckpt-6 STRICT close. \
         REFUSED ON SIGHT: TypedArrayData resurrection under any rename \
         (Refusal #1, W12 audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

/// Validate a closure callee kind. Accepts Closure / function-ref;
/// rejects anything else with a `RuntimeError`.
#[inline]
fn validate_closure_kind(op: &str, idx: usize, args: &[KindedSlot]) -> Option<VMError> {
    let Some(slot) = args.get(idx) else {
        return Some(VMError::RuntimeError(format!(
            "{}: missing closure argument at index {}",
            op, idx
        )));
    };
    match slot.kind {
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => None,
        other => Some(VMError::RuntimeError(format!(
            "{}: argument {} must be a closure or function ref, got kind {:?}",
            op, idx, other
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — surface-and-stop stubs
// Signatures preserved for method_registry.rs PHF integrity.
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `innerJoin` — inner join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_inner_join_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 5 {
        return Err(VMError::RuntimeError(
            "innerJoin: expected (left, right, leftKey, rightKey, selector)".to_string(),
        ));
    }
    for idx in [2, 3, 4] {
        if let Some(err) = validate_closure_kind("innerJoin", idx, args) {
            return Err(err);
        }
    }
    Err(ckpt3_surface("innerJoin", args))
}

/// v2 `leftJoin` — left join two arrays with key functions.
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_left_join_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 5 {
        return Err(VMError::RuntimeError(
            "leftJoin: expected (left, right, leftKey, rightKey, selector)".to_string(),
        ));
    }
    for idx in [2, 3, 4] {
        if let Some(err) = validate_closure_kind("leftJoin", idx, args) {
            return Err(err);
        }
    }
    Err(ckpt3_surface("leftJoin", args))
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product).
///
/// args: [left_array, right_array, result_selector_fn]
pub(crate) fn handle_cross_join_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "crossJoin: expected (left, right, selector)".to_string(),
        ));
    }
    if let Some(err) = validate_closure_kind("crossJoin", 2, args) {
        return Err(err);
    }
    Err(ckpt3_surface("crossJoin", args))
}
