//! MethodFnV2 handlers for numeric (f64/i48) methods, plus the bool / char
//! delegation entry points the method registry routes to (`bool_to_string_v2`
//! and the `char_*_v2` family).
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! Number / Int / Char / Bool are inline-scalar `NativeKind` variants
//! (`Float64`, `Int64`, `Ptr(HeapKind::Char)`, `Bool`) per ADR-006 §2.3 +
//! `crates/shape-value/src/heap_variants.rs`, so a kind-correct rewrite
//! of the bodies is possible **once the MethodHandler ABI lands the kinded
//! `&mut [KindedSlot] -> Result<KindedSlot>` shape** (cluster
//! E-builtins-backlog, Wave 5b template, commit `fa2bafc`). The
//! Decimal-coercion path (`number_*_v2` accepting an `Arc<Decimal>`
//! receiver) dispatches via `slot.as_heap_value()` + `HeapValue::Decimal`
//! match per Q8.
//!
//! The pre-Wave-6 implementation imported `shape_value::tag_bits::*` (the
//! deleted ValueWord tag dispatch — forbidden #7), the deleted
//! `shape_value::value_word::*` constructor surface, `ValueWordExt`
//! (forbidden #1), and `objects::raw_helpers::{extract_bool, extract_char,
//! extract_number_coerce, extract_heap_ref, type_error}` (the entire
//! `extract_*` family was deleted in cluster D-raw-helpers; only the
//! FilterExpr extractor remains). Per playbook §4 #9 / ADR-006 §2.7.7 a
//! Bool-default kinded shim preserving the call-pattern shape is
//! forbidden; per §7.4 the correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

#[inline]
fn surface(method: &str, receiver: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — {}.{}(): MethodHandler ABI needs kinded migration \
         (cluster E-builtins-backlog, Wave 5b template); receiver kind \
         resolved per ADR-006 §2.3 / §2.7.6 (inline scalar or \
         Ptr(HeapKind::Decimal) for cross-domain numeric).",
        receiver, method
    ))
}

// ---------------------------------------------------------------------------
// number / int methods
// ---------------------------------------------------------------------------

pub fn number_floor_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("floor", "number"))
}

pub fn number_ceil_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("ceil", "number"))
}

pub fn number_round_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("round", "number"))
}

pub fn number_abs_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("abs", "number"))
}

pub fn number_sign_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("sign", "number"))
}

pub fn number_to_int_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toInt", "number"))
}

pub fn number_to_number_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toNumber", "number"))
}

pub fn number_is_nan_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isNaN", "number"))
}

pub fn number_is_finite_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isFinite", "number"))
}

pub fn number_to_fixed_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toFixed", "number"))
}

pub fn number_to_string_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toString", "number"))
}

pub fn number_clamp_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("clamp", "number"))
}

// ---------------------------------------------------------------------------
// bool methods
// ---------------------------------------------------------------------------

pub fn bool_to_string_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toString", "bool"))
}

// ---------------------------------------------------------------------------
// char methods
// ---------------------------------------------------------------------------

pub fn char_is_alphabetic_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isAlphabetic", "char"))
}

pub fn char_is_numeric_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isNumeric", "char"))
}

pub fn char_is_alphanumeric_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isAlphanumeric", "char"))
}

pub fn char_is_whitespace_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isWhitespace", "char"))
}

pub fn char_is_uppercase_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isUppercase", "char"))
}

pub fn char_is_lowercase_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isLowercase", "char"))
}

pub fn char_is_ascii_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("isAscii", "char"))
}

pub fn char_to_uppercase_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toUppercase", "char"))
}

pub fn char_to_lowercase_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toLowercase", "char"))
}

pub fn char_to_string_v2(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("toString", "char"))
}
