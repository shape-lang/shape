//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every,
//! any, all, single, take_while, skip_while, for_each
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15) — drive-by
//!
//! This file's pickup is a drive-by from ckpt-2's `array_transform.rs`
//! cross-module helper deletion: the imports at original lines 49-52
//! (`typed_array_arc_from_kinded / element_kinded / project_indices /
//! collect_homogeneous_results / bump_closure_share`) reference helpers
//! that were deleted in ckpt-2 — every one of the first four took
//! `&TypedArrayData` or produced `Arc<TypedArrayData>`. Plus the file's
//! own 24 in-file `TypedArrayData::` references (value-search handlers
//! `handle_index_of_v2` / `handle_includes_v2` dispatching on
//! `TypedArrayData` arms via `typed_array_ref` and `read_element_at`)
//! cascade-broke at ckpt-1 enum deletion.
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
//!
//! Public handler bodies (`handle_where_v2 / handle_select_v2 /
//! handle_find_v2 / handle_find_index_v2 / handle_index_of_v2 /
//! handle_includes_v2 / handle_some_v2 / handle_every_v2 / handle_any_v2 /
//! handle_all_v2 / handle_single_v2 / handle_take_while_v2 /
//! handle_skip_while_v2 / handle_for_each_v2`) are replaced with
//! structured surface-and-stop returning `VMError::NotImplemented`. Local
//! helpers (`typed_array_arc / typed_array_ref / read_element_at` and
//! every per-`TypedArrayData::X` value-comparison body) and the deleted
//! cross-module imports are DELETED.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X(buf)`
//! match arm in `handle_index_of_v2` / `handle_includes_v2` /
//! `read_element_at` migrates to the v2-raw `TypedArray<T>` flat-struct
//! carrier — per-T direct comparison via `*buf.data.add(idx)`. The
//! closure-callback ABI (ADR-006 §2.7.11 / Q12 `vm.call_value_immediate_nb`)
//! is unaffected; predicate handlers re-instate against the v2-raw
//! receiver-shape once the per-T element-read path lands. The
//! result-builder handlers (`handle_where_v2` / `handle_select_v2` /
//! `handle_take_while_v2` / `handle_skip_while_v2`) re-route through the
//! sibling W9-array-transform cluster's per-T builder once the
//! `array_transform.rs` v2-raw rewrite lands.
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
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
         SUPERSEDED. The previous `Arc<TypedArrayData>` receiver-recovery \
         + per-variant value-search / closure-callback dispatch path (~24 \
         references across 14 public handlers in this file plus the \
         5-import E0432 cluster from ckpt-2 cross-module helper deletion \
         at array_transform.rs) cascade-broke at the enum deletion site \
         (`crates/shape-value/src/heap_value.rs:3944`). Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe; per-T monomorphization \
         landing across ckpt-3 (this file plus array_ops/typed_array_methods/\
         iterator_methods/array_sort/concat/property_access) + ckpt-4 \
         (Buf<T> / HeapValue::TypedArray arm / HeapKind::TypedArray \
         ordinal) + ckpt-5 (wire/json/marshal + 4-table lockstep) + \
         ckpt-6 (JIT FFI). Closure-callback ABI (ADR-006 §2.7.11 / Q12 \
         `vm.call_value_immediate_nb`) is unaffected and re-instates \
         once receiver-shape migration lands. Receiver kind: {kind}. \
         UNREACHABLE until ckpt-6 STRICT close. REFUSED ON SIGHT: \
         TypedArrayData resurrection under any rename (Refusal #1, W12 \
         audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

/// Closure-arg validation for closure-callback handlers.
#[inline]
fn validate_closure_arg(op: &str, args: &[KindedSlot]) -> Option<VMError> {
    if args.len() >= 2 && args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        Some(VMError::RuntimeError(format!(
            "{}: second argument must be a closure, got kind {:?}",
            op, args[1].kind
        )))
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — surface-and-stop stubs
// Signatures preserved for method_registry.rs PHF integrity.
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.where(|x| ...)` — predicate-filter projection.
pub(crate) fn handle_where_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("where", args) {
        return Err(err);
    }
    Err(ckpt3_surface("where", args))
}

/// `arr.select(|x| ...)` — per-element transform.
pub(crate) fn handle_select_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("select", args) {
        return Err(err);
    }
    Err(ckpt3_surface("select", args))
}

/// `arr.find(|x| ...)`.
pub(crate) fn handle_find_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("find", args) {
        return Err(err);
    }
    Err(ckpt3_surface("find", args))
}

/// `arr.findIndex(|x| ...)`.
pub(crate) fn handle_find_index_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("findIndex", args) {
        return Err(err);
    }
    Err(ckpt3_surface("findIndex", args))
}

/// `arr.indexOf(value)` — value-search.
pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("indexOf", args))
}

/// `arr.includes(value)` — value-search.
pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("includes", args))
}

/// `arr.some(|x| ...)`.
pub(crate) fn handle_some_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("some", args) {
        return Err(err);
    }
    Err(ckpt3_surface("some", args))
}

/// `arr.every(|x| ...)`.
pub(crate) fn handle_every_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("every", args) {
        return Err(err);
    }
    Err(ckpt3_surface("every", args))
}

/// `arr.any(|x| ...)`.
pub(crate) fn handle_any_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("any", args) {
        return Err(err);
    }
    Err(ckpt3_surface("any", args))
}

/// `arr.all(|x| ...)`.
pub(crate) fn handle_all_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("all", args) {
        return Err(err);
    }
    Err(ckpt3_surface("all", args))
}

/// `arr.single(|x| ...)` — find the unique element matching the predicate.
pub(crate) fn handle_single_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("single", args) {
        return Err(err);
    }
    Err(ckpt3_surface("single", args))
}

/// `arr.takeWhile(|x| ...)`.
pub(crate) fn handle_take_while_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("takeWhile", args) {
        return Err(err);
    }
    Err(ckpt3_surface("takeWhile", args))
}

/// `arr.skipWhile(|x| ...)`.
pub(crate) fn handle_skip_while_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("skipWhile", args) {
        return Err(err);
    }
    Err(ckpt3_surface("skipWhile", args))
}

/// `arr.forEach(|x| ...)`.
pub(crate) fn handle_for_each_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("forEach", args) {
        return Err(err);
    }
    Err(ckpt3_surface("forEach", args))
}
