//! Method handlers for concurrency primitive types: Mutex<T>, Atomic<T>, Lazy<T>
//!
//! Phase 1.B-vm Wave 6.5 substep-2 cluster D-obj-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §10 D-obj-tail row + §7 REVISED
//! DoD #4. The original implementations relied on the
//! `HeapValue::Concurrency(ConcurrencyData::*)` enum form (deleted by the
//! HeapValue-typed-Arc redesign, ADR-006 §2.3) and on
//! `raw_helpers::{extract_heap_ref, extract_number_coerce}` plus
//! `ValueWord::{from_raw_bits, from_i64, none, clone_from_bits, from_bool}`
//! (forbidden #7 / #1 per playbook §4).
//!
//! Mandatory-shim sites in `v2_lazy_get` (`vm.push_raw_u64` x2 +
//! `vm.pop_raw_u64`) cannot be migrated to the kinded API in isolation:
//! the closure-call pathway through `op_call_value` (B11 territory) is
//! itself still on raw-u64 / `tag_bits::*` and cannot accept kinded
//! operands without that cluster's migration. Per ADR-006 §2.7.7 #9 +
//! CLAUDE.md "Renames to refuse on sight", a Bool-default kinded shim
//! preserving the closure-call shape is forbidden.
//!
//! Resurfacing concurrency primitives is Phase 2c: ConcurrencyData needs
//! re-modelling on top of typed-Arc HeapValue payloads, after which
//! Mutex/Atomic/Lazy method dispatch can route through the kinded API.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V2 (MethodFnV2) handlers — raw u64 ABI placeholder bodies
// ═══════════════════════════════════════════════════════════════════════════

// ── Mutex<T> ─────────────────────────────────────────────────────────────

/// `mutex.lock()` — v2 ABI.
pub fn v2_mutex_lock(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Mutex.lock(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

/// `mutex.try_lock()` — v2 ABI.
pub fn v2_mutex_try_lock(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Mutex.try_lock(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

/// `mutex.set(value)` — v2 ABI.
pub fn v2_mutex_set(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Mutex.set(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

// ── Atomic<T> ────────────────────────────────────────────────────────────

/// `atomic.load()` — v2 ABI.
pub fn v2_atomic_load(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Atomic.load(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

/// `atomic.store(value)` — v2 ABI.
pub fn v2_atomic_store(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Atomic.store(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

/// `atomic.fetch_add(delta)` — v2 ABI.
pub fn v2_atomic_fetch_add(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Atomic.fetch_add(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

/// `atomic.fetch_sub(delta)` — v2 ABI.
pub fn v2_atomic_fetch_sub(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Atomic.fetch_sub(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

/// `atomic.compare_exchange(expected, new)` — v2 ABI.
pub fn v2_atomic_compare_exchange(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Atomic.compare_exchange(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}

// ── Lazy<T> ──────────────────────────────────────────────────────────────

/// `lazy.get()` — v2 ABI.
///
/// Note: `lazy.get()` may invoke an initializer closure, which requires
/// calling into the VM (`op_call_value`). This handler therefore needs `vm`
/// (not `_vm`) once Phase 2c re-enables the body.
pub fn v2_lazy_get(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Lazy.get(): closure-call dispatch + Concurrency typed-Arc redesign per ADR-006 §2.3 / §2.7.7"
            .to_string(),
    ))
}

/// `lazy.is_initialized()` — v2 ABI.
pub fn v2_lazy_is_initialized(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "phase-2c — Lazy.is_initialized(): Concurrency variant needs typed-Arc redesign per ADR-006 §2.3"
            .to_string(),
    ))
}
