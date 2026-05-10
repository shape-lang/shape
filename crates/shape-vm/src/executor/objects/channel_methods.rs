//! Method handlers for the Channel type (MPSC sender/receiver endpoints).
//!
//! ## W15-channel-rebuild migration (2026-05-10)
//!
//! Per ADR-006 §2.7.20 / Q21 amendment (Wave 15 W15-channel-rebuild),
//! the Channel carrier is a typed-`Arc<ChannelData>`-backed `HeapValue`
//! arm — full HeapValue arm, not pure-discriminator like FilterExpr /
//! SharedCell. Channel is the first concurrency primitive to land
//! kinded; same shape Mutex / Atomic / Lazy will use when they rebuild.
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::Channel)`, then `args[0].slot.as_heap_value()`
//! pattern-matched against `HeapValue::Channel(arc)` (single-discriminator
//! per ADR-005 §1 — no per-heap-variant `KindedSlot` accessor).
//!
//! ## Sync-only path at landing
//!
//! The smoke target (`let c = Channel(); c.send(1); c.recv()` returns
//! `1`) exercises the same-thread sync path:
//!
//! - `send(slot)` clones the kinded slot (bumps refcount for heap
//!   payloads) and queues it via `ChannelData::send`.
//! - `recv()` calls `try_recv()` first; on `Some(slot)` the queued
//!   `KindedSlot` transfers to the caller verbatim (its share moves).
//!   On `None` the receiver is empty: per playbook §0 surface-and-stop
//!   discipline, blocking `recv()` requires the §2.7.4 task-scheduler
//!   boundary; we surface a clean `NotImplemented` rather than fabricate
//!   a Bool-default null fallback.
//! - `try_recv()` returns the queued slot or `null` (`KindedSlot::none`)
//!   when the queue is empty — sync-friendly poll API.
//! - `close()` flips the closed flag; further `send()` calls error.
//! - `is_closed()` reads the flag.
//! - `is_sender()` SURFACE — pre-bulldozer Channel had separate
//!   sender/receiver endpoints; the rebuild collapses both into a
//!   single `Arc<ChannelData>` carrier (any share is both producer
//!   and consumer). The is_sender method is preserved at the PHF
//!   surface for source-compatibility with pre-bulldozer Shape code
//!   but always errors with a SURFACE message — the typed
//!   sender/receiver split is a phase-2c follow-up.
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.20 + Wave 14-15-16 playbook
//! §2.W15-channel.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{ChannelData, HeapKind, HeapValue};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to the inner `Arc<ChannelData>`
/// via the §2.7.6 / Q8 single-discriminator path: kind gate on
/// `Ptr(HeapKind::Channel)`, then `slot.as_heap_value()` matched
/// against `HeapValue::Channel(arc)`. The receiver retains its share
/// — the caller borrows through the `&Arc<ChannelData>` and never
/// decrements.
#[inline]
fn as_channel(slot: &KindedSlot) -> Result<Arc<ChannelData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Channel)) {
        return Err(type_error(format!(
            "Channel method receiver must be a Channel (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Channel method receiver slot bits null"));
    }
    // SAFETY: see `set_methods::as_hashset` for the canonical form.
    // `KindedSlot::from_channel` stores `Arc::into_raw(Arc<ChannelData>)`
    // directly per §2.7.20; recovery uses the same typed-Arc shape.
    let arc = unsafe { Arc::<ChannelData>::from_raw(bits as *const ChannelData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

// ═══════════════════════════════════════════════════════════════════════════
// Sender-side handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Channel.send(value) -> Channel
///
/// Queues `value` on the channel. The kinded slot is `clone()`'d (bumps
/// any heap-bearing refcount share) before being queued — both the
/// caller-passed argument carrier and the queued copy own independent
/// shares. Returns the channel itself (chained `c.send(1).send(2)` flow
/// works).
///
/// Errors with a closed-channel message if the channel has been
/// `close()`'d.
pub fn v2_channel_send(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Channel.send() requires exactly 1 argument (value)",
        ));
    }
    let ch: Arc<ChannelData> = as_channel(&args[0])?;
    // Clone the value's KindedSlot — bumps refcount for heap shares,
    // no-op for inline scalars (Drop discipline preserved through
    // KindedSlot::Clone per §2.7.7).
    let queued = args[1].clone();
    if ch.send(queued).is_err() {
        return Err(type_error("Channel.send() on a closed channel"));
    }
    // Return the receiver share — fresh KindedSlot with one strong-
    // count bump on the same `Arc<ChannelData>`.
    let result = ch;
    Ok(KindedSlot::from_channel(result))
}

// ═══════════════════════════════════════════════════════════════════════════
// Receiver-side handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Channel.recv() -> T
///
/// Synchronous same-thread receive: pops the front element if the
/// queue is non-empty.
///
/// **Empty-queue surface.** The cross-task blocking `recv()` (the
/// canonical async-channel use case) requires integration with the
/// §2.7.4 task-scheduler boundary — the receiver suspends until a
/// producer `send()`s. Per the W15 playbook surface-and-stop
/// discipline that integration is a separate phase-2c cluster; this
/// handler returns `NotImplemented(SURFACE)` on an empty queue rather
/// than fabricate a Bool-default null. Sync code should call
/// `try_recv()` for the non-blocking poll variant.
pub fn v2_channel_recv(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Channel.recv() takes no arguments"));
    }
    let ch: Arc<ChannelData> = as_channel(&args[0])?;
    if let Some(slot) = ch.try_recv() {
        return Ok(slot);
    }
    if ch.is_closed() {
        return Err(type_error(
            "Channel.recv() on a closed and empty channel",
        ));
    }
    Err(VMError::NotImplemented(
        "Channel.recv() blocking-on-empty path requires the \
         §2.7.4 task-scheduler boundary integration (cross-task \
         await-style suspend/resume) — phase-2c follow-up. Use \
         try_recv() for the non-blocking poll variant. \
         ADR-006 §2.7.20 / §2.7.4."
            .to_string(),
    ))
}

/// Channel.try_recv() -> T?
///
/// Non-blocking receive: returns the front element if available,
/// `null` (`KindedSlot::none`) otherwise. Sync-friendly poll API
/// — the canonical surface for same-thread channel use.
pub fn v2_channel_try_recv(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "Channel.try_recv() takes no arguments",
        ));
    }
    let ch: Arc<ChannelData> = as_channel(&args[0])?;
    Ok(ch.try_recv().unwrap_or_else(KindedSlot::none))
}

// ═══════════════════════════════════════════════════════════════════════════
// Lifecycle handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Channel.close() -> Channel
///
/// Mark the channel closed. Idempotent — calling close on an already-
/// closed channel is a no-op. Returns the channel itself.
pub fn v2_channel_close(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Channel.close() takes no arguments"));
    }
    let ch: Arc<ChannelData> = as_channel(&args[0])?;
    ch.close();
    Ok(KindedSlot::from_channel(ch))
}

/// Channel.is_closed() -> bool
pub fn v2_channel_is_closed(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "Channel.is_closed() takes no arguments",
        ));
    }
    let ch: Arc<ChannelData> = as_channel(&args[0])?;
    Ok(KindedSlot::from_bool(ch.is_closed()))
}

/// Channel.is_sender() -> bool — SURFACE
///
/// The pre-bulldozer Channel design had separate sender/receiver
/// endpoint types so callers could classify a Channel handle's role.
/// The W15 rebuild collapses both endpoints into a single
/// `Arc<ChannelData>` carrier (any share can both `send` and `recv`),
/// so the role-check method has no semantic answer at the kinded
/// surface. Surface as a phase-2c follow-up rather than fabricate a
/// Bool-default answer (forbidden #9).
///
/// The method is preserved at the PHF surface for source-
/// compatibility with pre-bulldozer Shape code that may still reach
/// this entry point during incremental migration.
pub fn v2_channel_is_sender(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "Channel.is_sender() takes no arguments",
        ));
    }
    let _ = as_channel(&args[0])?;
    Err(VMError::NotImplemented(
        "Channel.is_sender(): the W15 rebuild (ADR-006 §2.7.20 / \
         Q21, 2026-05-10) collapses sender/receiver endpoints into \
         a single Arc<ChannelData> carrier — the role-check has no \
         semantic answer. Re-introducing typed sender/receiver \
         endpoints is a phase-2c follow-up."
            .to_string(),
    ))
}
