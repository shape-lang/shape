//! Native method handlers for char values.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! The pre-Wave-6 implementation imported the deleted
//! `shape_value::value_word::*` constructor surface (`vw_from_bool`,
//! `vw_from_char`, `vw_from_string`) plus `ValueWordExt` (forbidden #1)
//! and `objects::raw_helpers::{extract_char, type_error}` (deleted in
//! cluster D-raw-helpers — only the FilterExpr extractor remains). Char
//! payloads are still reachable via `HeapKind::Char` (codepoint inline,
//! no Arc; see `stack_ops::op_push_const` Char arm + ADR-006 §2.3
//! Char-as-inline-scalar shape), so a kind-correct rewrite is mechanical
//! once the MethodHandler ABI lands the kinded `&mut [KindedSlot] ->
//! Result<KindedSlot>` shape (cluster E-builtins-backlog, Wave 5b
//! template, commit `fa2bafc`). Per playbook §4 #9 / ADR-006 §2.7.7 a
//! Bool-default kinded shim preserving the call-pattern shape is
//! forbidden.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — char.{}(): MethodHandler ABI needs kinded migration \
         (cluster E-builtins-backlog, Wave 5b template); receiver kind \
         NativeKind::Ptr(HeapKind::Char), codepoint inline per ADR-006 §2.3 / §2.7.6",
        method
    ))
}

pub fn char_is_alphabetic(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isAlphabetic"))
}

pub fn char_is_numeric(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isNumeric"))
}

pub fn char_is_alphanumeric(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isAlphanumeric"))
}

pub fn char_is_whitespace(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isWhitespace"))
}

pub fn char_is_uppercase(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isUppercase"))
}

pub fn char_is_lowercase(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isLowercase"))
}

pub fn char_is_ascii(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("isAscii"))
}

pub fn char_to_uppercase(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toUppercase"))
}

pub fn char_to_lowercase(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toLowercase"))
}

pub fn char_to_string(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toString"))
}
