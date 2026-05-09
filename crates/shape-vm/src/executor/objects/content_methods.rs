//! Content method dispatch for ContentNode values.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Content` *is* a surviving `HeapKind` variant
//! (`Content(Arc<ContentNode>)` per ADR-006 §2.3 +
//! `crates/shape-value/src/heap_variants.rs`), so a kind-correct rewrite
//! of these handlers is mechanical: receiver is
//! `NativeKind::Ptr(HeapKind::Content)`, dispatch via
//! `slot.as_heap_value()` + `HeapValue::Content(arc)` match per Q8, push
//! the result as `Arc::into_raw(Arc<ContentNode>) as u64` with kind
//! `NativeKind::Ptr(HeapKind::Content)` (string return arms push
//! `NativeKind::String`).
//!
//! Migration is blocked on the MethodHandler ABI rewrite to
//! `&mut [KindedSlot] -> Result<KindedSlot>` (cluster
//! E-builtins-backlog, Wave 5b template, commit `fa2bafc`). The
//! pre-Wave-6 implementation imported the deleted
//! `shape_value::{ValueWord, ValueWordExt, ValueWordDisplay}` surface,
//! the deleted `ValueWord::from_content` / `from_string` /
//! `from_raw_bits` / `clone_from_bits` constructors, and the
//! `objects::raw_helpers::{extract_content, extract_number_coerce,
//! extract_str}` helpers (deleted in cluster D-raw-helpers — only the
//! FilterExpr extractor remains). The macro-generated runtime delegators
//! (`v2_content_border`, `v2_content_series`, etc.) call into
//! `shape_runtime::content_methods::call_content_method` which itself
//! takes `ValueWord` arguments — that crate-boundary signature is also
//! awaiting the kinded redesign per playbook §8 cross-cluster cascade.
//! Per playbook §4 #1 / #9 a Bool-default kinded shim is forbidden; per
//! §7.4 the correct response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Content.{}(): MethodHandler ABI needs kinded migration \
         (cluster E-builtins-backlog, Wave 5b template); receiver kind \
         NativeKind::Ptr(HeapKind::Content), dispatch via \
         slot.as_heap_value() + HeapValue::Content match per ADR-006 \
         §2.7.6 / Q8. Runtime delegators (border/series/title/etc.) also \
         depend on the shape-runtime crate-boundary kinded redesign per \
         playbook §8 cross-cluster cascade.",
        method
    ))
}

pub fn v2_content_bold(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("bold"))
}

pub fn v2_content_italic(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("italic"))
}

pub fn v2_content_underline(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("underline"))
}

pub fn v2_content_dim(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("dim"))
}

pub fn v2_content_fg(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("fg"))
}

pub fn v2_content_bg(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("bg"))
}

pub fn v2_content_to_string(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toString"))
}

pub fn v2_content_border(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("border"))
}

pub fn v2_content_max_rows(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("max_rows"))
}

pub fn v2_content_max_rows_camel(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("maxRows"))
}

pub fn v2_content_series(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("series"))
}

pub fn v2_content_title(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("title"))
}

pub fn v2_content_x_label(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("x_label"))
}

pub fn v2_content_x_label_camel(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("xLabel"))
}

pub fn v2_content_y_label(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("y_label"))
}

pub fn v2_content_y_label_camel(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("yLabel"))
}
