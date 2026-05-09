//! Method handlers for the Channel type (MPSC sender/receiver endpoints).
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7), matching the
//! `concurrency_methods.rs` close-out (Mutex/Atomic/Lazy).
//!
//! `Channel` payloads lived inside the deleted
//! `HeapValue::Concurrency(ConcurrencyData::Channel(_))` arm — the
//! `Concurrency` variant was removed from `HeapValue` per ADR-006 §2.3
//! trim (`crates/shape-value/src/heap_variants.rs` lists no
//! `Concurrency` variant; `ConcurrencyData` itself was deleted with the
//! `ValueWord`-shaped per-element payloads it depended on). Re-modelling
//! Channel + Mutex + Atomic + Lazy on top of typed-Arc HeapValue is a
//! Phase 2c Stage C item; the `concurrency_methods` precedent calls out
//! the exact same cascade.
//!
//! The pre-Wave-6 implementation used the deleted
//! `shape_value::{ValueWord, ValueWordExt, ConcurrencyData}` surface,
//! the deleted `ValueWord::from_bool` / `none` / `clone_from_bits`
//! constructors, the `objects::raw_helpers::extract_heap_ref` (deleted
//! in cluster D-raw-helpers — only the FilterExpr extractor remains),
//! and the kindless MethodHandler ABI. Per playbook §4 #1 / #9 a
//! Bool-default kinded shim is forbidden; per §7.4 the correct response
//! is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Channel.{}(): Concurrency variant needs typed-Arc redesign \
         per ADR-006 §2.3 (matches concurrency_methods.rs precedent for \
         Mutex/Atomic/Lazy). MethodHandler ABI also needs kinded migration \
         (cluster E-builtins-backlog, Wave 5b template).",
        method
    ))
}

pub fn v2_channel_send(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("send"))
}

pub fn v2_channel_recv(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("recv"))
}

pub fn v2_channel_try_recv(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("try_recv"))
}

pub fn v2_channel_close(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("close"))
}

pub fn v2_channel_is_closed(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("is_closed"))
}

pub fn v2_channel_is_sender(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("is_sender"))
}
