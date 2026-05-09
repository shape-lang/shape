//! Array sort operations
//!
//! Handles: order_by, then_by, join_str
//!
//! ## Wave-δ `MR-array-sort-sets-joins` body migration (ADR-006 §2.7.10 / Q11)
//!
//! Receiver enters as
//! `args[0]: KindedSlot { kind: NativeKind::Ptr(HeapKind::TypedArray) }`;
//! payload recovery via ADR-005 §1 single-discriminator dispatch through
//! `args[0].slot.as_heap_value()` + `HeapValue::TypedArray(arc)` match
//! (Wave 5b precedent in `executor/builtins/array_ops.rs`).
//!
//! `joinStr` does not require a closure callback: it iterates a typed
//! array element-by-element, stringifies per-arm, and concatenates with a
//! separator pulled from `args[1]: KindedSlot { kind: NativeKind::String }`.
//! Body migrated.
//!
//! `orderBy` / `thenBy` take a comparator-closure that returns `int`
//! (negative / zero / positive). The closure-callback path
//! (`executor/call_convention.rs::call_value_immediate_*` and
//! `executor/control_flow/mod.rs::op_call_value`) is itself
//! `NotImplemented(SURFACE)` post-§2.7.10 (Phase-2c rebuild — kinded
//! callee dispatch + `&[KindedSlot]` arg-slice on the runtime side).
//! Surface per playbook §7.4 REVISED.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{
    HeapKind, HeapValue, KindedSlot, NativeKind, TypedArrayData, VMError,
};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

#[inline]
fn as_typed_array(slot: &KindedSlot) -> Option<&Arc<TypedArrayData>> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::TypedArray)) {
        return None;
    }
    match slot.slot.as_heap_value() {
        HeapValue::TypedArray(arc) => Some(arc),
        _ => None,
    }
}

/// Stringify a single element of `arr` at `idx` in the canonical
/// per-arm format. Mirrors the pre-Wave-6 join semantics: integer/float
/// formatting, bool as "true"/"false", string passthrough.
fn element_to_string(arr: &TypedArrayData, idx: usize, out: &mut String) -> Result<(), VMError> {
    use std::fmt::Write as _;
    match arr {
        TypedArrayData::I64(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::F64(buf) => {
            let v = buf.data[idx];
            // Match the Display contract used elsewhere in the runtime
            // (TypedArrayData::F64 in heap_value.rs:996): integer-valued
            // f64s render as integers when |v| < 1e15.
            if v == v.trunc() && v.abs() < 1e15 {
                write!(out, "{}", v as i64).map_err(|e| type_error(e.to_string()))
            } else {
                write!(out, "{}", v).map_err(|e| type_error(e.to_string()))
            }
        }
        TypedArrayData::Bool(buf) => {
            let b = buf.data[idx] != 0;
            write!(out, "{}", b).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::I8(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::I16(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::I32(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U8(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U16(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U32(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::U64(buf) => {
            write!(out, "{}", buf.data[idx]).map_err(|e| type_error(e.to_string()))
        }
        TypedArrayData::F32(buf) => {
            let v = buf.data[idx];
            if v == v.trunc() && v.abs() < 1e15 {
                write!(out, "{}", v as i64).map_err(|e| type_error(e.to_string()))
            } else {
                write!(out, "{}", v).map_err(|e| type_error(e.to_string()))
            }
        }
        TypedArrayData::String(buf) => {
            out.push_str(buf.data[idx].as_str());
            Ok(())
        }
        TypedArrayData::HeapValue(_)
        | TypedArrayData::Matrix(_)
        | TypedArrayData::FloatSlice { .. } => Err(type_error(format!(
            "joinStr: TypedArrayData variant {} not part of the Wave-δ joinStr migration \
             (Phase-2c reentry — heterogeneous-heap / matrix / float-slice element \
             stringification needs per-NativeKind formatter dispatch via the kinded \
             output-adapter path, not yet wired through the MethodFnV2 handler tier)",
            arr.type_name()
        ))),
    }
}

fn array_len(arr: &TypedArrayData) -> Result<usize, VMError> {
    Ok(match arr {
        TypedArrayData::I64(b) => b.len(),
        TypedArrayData::F64(b) => b.len(),
        TypedArrayData::Bool(b) => b.len(),
        TypedArrayData::I8(b) => b.len(),
        TypedArrayData::I16(b) => b.len(),
        TypedArrayData::I32(b) => b.len(),
        TypedArrayData::U8(b) => b.len(),
        TypedArrayData::U16(b) => b.len(),
        TypedArrayData::U32(b) => b.len(),
        TypedArrayData::U64(b) => b.len(),
        TypedArrayData::F32(b) => b.len(),
        TypedArrayData::String(b) => b.len(),
        TypedArrayData::HeapValue(b) => b.len(),
        TypedArrayData::Matrix(m) => m.data.len(),
        TypedArrayData::FloatSlice { len, .. } => *len as usize,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `orderBy` — sort an array by a key function (optionally with direction).
///
/// args: [array, key_fn, direction?]
///
/// **SURFACE — Wave-δ closure-callback dependency.** The kinded
/// `MethodFnV2` ABI landed (Wave-γ G-method-fn-v2-abi); however the
/// closure-callback path needed to invoke `key_fn(elem)` per element
/// (`call_value_immediate_*` in `executor/call_convention.rs`,
/// `op_call_value` in `executor/control_flow/mod.rs`) is itself
/// `NotImplemented(SURFACE)` post-§2.7.10 pending the kinded callee
/// dispatch + `&[KindedSlot]` arg-slice rebuild (ADR-006 §2.7.4 / §2.7.8
/// Phase-2c). Without it the handler cannot invoke the user's comparator.
pub(crate) fn handle_order_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "orderBy — SURFACE: closure-callback path unmigrated. \
         The kinded MethodFnV2 ABI landed (ADR-006 §2.7.10 / Q11), but \
         `call_value_immediate_*` / `op_call_value` (executor/call_convention.rs, \
         executor/control_flow/mod.rs) still return NotImplemented(SURFACE) \
         pending the kinded callee dispatch + `&[KindedSlot]` arg-slice rebuild \
         per ADR-006 §2.7.4 / §2.7.8. Body shape: `args[0]` = receiver \
         (Ptr(HeapKind::TypedArray)), `args[1]` = comparator closure \
         (Ptr(HeapKind::Closure)), `args[2]?` = direction; iterate elements as \
         indexed pairs and stably sort via `slice::sort_by` driving the closure \
         callback (signum of the int return)."
            .to_string(),
    ))
}

/// v2 `thenBy` — sort an already-ordered array by a secondary key.
///
/// args: [array, key_fn, direction?]
///
/// **SURFACE — Wave-δ closure-callback dependency.** Same blocker as
/// `orderBy`; bodies share the comparator-closure callback shape.
pub(crate) fn handle_then_by_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "thenBy — SURFACE: closure-callback path unmigrated. \
         Same blocker as orderBy: kinded `call_value_immediate_*` / \
         `op_call_value` dispatch is Phase-2c rebuild territory (ADR-006 \
         §2.7.4 / §2.7.8). Body shape mirrors orderBy with the receiver \
         already partially-ordered (the secondary-key sort is stable)."
            .to_string(),
    ))
}

/// v2 `joinStr` — join array elements into a single string with a separator.
///
/// args: [array, separator]
///
/// Receiver kind = `NativeKind::Ptr(HeapKind::TypedArray)`; separator kind
/// = `NativeKind::String`. Element stringification dispatches on the
/// `TypedArrayData::*` arm (no closure callback). The result is a fresh
/// `Arc<String>` carried as `KindedSlot::from_string_arc`.
pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "joinStr() requires 2 arguments (array, separator)",
        ));
    }
    let arc = as_typed_array(&args[0])
        .ok_or_else(|| type_error("joinStr(): receiver must be an Array"))?;
    let arr = arc.as_ref();

    let sep: &str = match args[1].kind {
        NativeKind::String => args[1]
            .as_str()
            .ok_or_else(|| type_error("joinStr(): separator slot kind=String but bits empty"))?,
        _ => {
            return Err(type_error(format!(
                "joinStr(): separator must be a string, got {:?}",
                args[1].kind
            )));
        }
    };

    let len = array_len(arr)?;
    let mut out = String::new();
    for i in 0..len {
        if i > 0 {
            out.push_str(sep);
        }
        element_to_string(arr, i, &mut out)?;
    }
    Ok(KindedSlot::from_string_arc(Arc::new(out)))
}

// Tests intentionally not added in this file: handler tests need a
// minimal `VirtualMachine` instance and the dispatch shell
// (`op_call_method`) is itself a §2.7.10 SURFACE pending the
// receiver-classification cascade. Test coverage for `joinStr` lands
// alongside the dispatch-shell rebuild via the same harness pattern as
// `executor/builtins/array_ops.rs::tests` (Wave 5b body migrations).
