//! Method handlers for Channel type (MPSC sender/receiver endpoints).
//!
//! Both legacy (`handle_*`) and v2 (`v2_*`) handlers are provided. The v2 handlers
//! use the `MethodFnV2` ABI: `fn(&mut VM, &[u64], ctx) -> Result<u64, VMError>`.

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::type_mismatch_error;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;

/// Borrow a `ValueWord` from raw u64 bits without taking ownership.
///
/// The returned `ManuallyDrop<ValueWord>` prevents the destructor from running,
/// so the caller's raw bits remain valid. This is safe because `ValueWord` is
/// `#[repr(transparent)]` over `u64`.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    // SAFETY: ValueWord is repr(transparent) over u64.
    ManuallyDrop::new(unsafe { std::mem::transmute::<u64, ValueWord>(raw) })
}

/// Transfer ownership of a `ValueWord` into raw u64 bits.
///
/// The `ValueWord` destructor is suppressed so the refcount is NOT decremented.
/// The caller (dispatcher) takes ownership of the returned bits via `transmute`.
#[inline]
fn into_raw(vw: ValueWord) -> u64 {
    let bits = vw.raw_bits();
    std::mem::forget(vw);
    bits
}

// ═══════════════════════════════════════════════════════════════════════════
// Channel sender methods
// ═══════════════════════════════════════════════════════════════════════════

/// `sender.send(value)` — send a value through the channel.
/// Returns true if sent successfully, false if the channel is closed.
pub fn handle_channel_send(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let value = args.get(1).cloned().unwrap_or_else(ValueWord::none);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("send()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Sender { tx, closed, .. } => {
                if closed.load(std::sync::atomic::Ordering::Relaxed) {
                    Ok(ValueWord::from_bool(false))
                } else {
                    match tx.send(value) {
                        Ok(()) => Ok(ValueWord::from_bool(true)),
                        Err(_) => Ok(ValueWord::from_bool(false)),
                    }
                }
            }
            _ => Err(VMError::RuntimeError(
                "send() called on channel receiver (use on sender)".to_string(),
            )),
        },
        _ => Err(VMError::RuntimeError(
            "send() called on non-channel value".to_string(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Channel receiver methods
// ═══════════════════════════════════════════════════════════════════════════

/// `receiver.recv()` — receive a value from the channel (blocking).
/// Returns the value, or None if the channel is closed/disconnected.
pub fn handle_channel_recv(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("recv()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Receiver { rx, .. } => {
                // Recover from mutex poisoning: the underlying Receiver is still
                // usable even if a previous holder panicked. Use into_inner() to
                // extract the guard and clear the poison flag.
                let guard = rx.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match guard.recv() {
                    Ok(val) => Ok(val),
                    Err(_) => Ok(ValueWord::none()),
                }
            }
            _ => Err(VMError::RuntimeError(
                "recv() called on channel sender (use on receiver)".to_string(),
            )),
        },
        _ => Err(VMError::RuntimeError(
            "recv() called on non-channel value".to_string(),
        )),
    }
}

/// `receiver.try_recv()` — try to receive without blocking.
/// Returns the value if available, or None otherwise.
pub fn handle_channel_try_recv(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        type_mismatch_error("try_recv()", "channel")
    })?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Receiver { rx, .. } => {
                // Recover from mutex poisoning: the underlying Receiver is still
                // usable even if a previous holder panicked. Use into_inner() to
                // extract the guard and clear the poison flag.
                let guard = rx.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match guard.try_recv() {
                    Ok(val) => Ok(val),
                    Err(_) => Ok(ValueWord::none()),
                }
            }
            _ => Err(VMError::RuntimeError(
                "try_recv() called on channel sender (use on receiver)".to_string(),
            )),
        },
        _ => Err(VMError::RuntimeError(
            "try_recv() called on non-channel value".to_string(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared methods (both sender and receiver)
// ═══════════════════════════════════════════════════════════════════════════

/// `channel.close()` — close the channel.
pub fn handle_channel_close(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("close()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => {
            data.close();
            Ok(ValueWord::none())
        }
        _ => Err(VMError::RuntimeError(
            "close() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.is_closed()` — check if the channel has been closed.
pub fn handle_channel_is_closed(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        type_mismatch_error("is_closed()", "channel")
    })?;
    match heap {
        HeapValue::Channel(data) => Ok(ValueWord::from_bool(data.is_closed())),
        _ => Err(VMError::RuntimeError(
            "is_closed() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.is_sender()` — returns true if this is the sender end.
pub fn handle_channel_is_sender(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        type_mismatch_error("is_sender()", "channel")
    })?;
    match heap {
        HeapValue::Channel(data) => Ok(ValueWord::from_bool(data.is_sender())),
        _ => Err(VMError::RuntimeError(
            "is_sender() called on non-channel value".to_string(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 (MethodFnV2) handlers — raw u64 ABI
// ═══════════════════════════════════════════════════════════════════════════

/// `sender.send(value)` — v2 ABI. args[0]=channel, args[1]=value.
pub fn v2_channel_send(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let value = if args.len() > 1 {
        // SAFETY: clone_from_bits increments refcount for heap values.
        unsafe { ValueWord::clone_from_bits(args[1]) }
    } else {
        ValueWord::none()
    };
    let heap = vw
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("send()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Sender { tx, closed, .. } => {
                if closed.load(std::sync::atomic::Ordering::Relaxed) {
                    Ok(into_raw(ValueWord::from_bool(false)))
                } else {
                    match tx.send(value) {
                        Ok(()) => Ok(into_raw(ValueWord::from_bool(true))),
                        Err(_) => Ok(into_raw(ValueWord::from_bool(false))),
                    }
                }
            }
            _ => Err(VMError::RuntimeError(
                "send() called on channel receiver (use on sender)".to_string(),
            )),
        },
        _ => Err(VMError::RuntimeError(
            "send() called on non-channel value".to_string(),
        )),
    }
}

/// `receiver.recv()` — v2 ABI.
pub fn v2_channel_recv(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let heap = vw
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("recv()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Receiver { rx, .. } => {
                let guard = rx.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match guard.recv() {
                    Ok(val) => Ok(into_raw(val)),
                    Err(_) => Ok(into_raw(ValueWord::none())),
                }
            }
            _ => Err(VMError::RuntimeError(
                "recv() called on channel sender (use on receiver)".to_string(),
            )),
        },
        _ => Err(VMError::RuntimeError(
            "recv() called on non-channel value".to_string(),
        )),
    }
}

/// `receiver.try_recv()` — v2 ABI.
pub fn v2_channel_try_recv(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let heap = vw
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("try_recv()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Receiver { rx, .. } => {
                let guard = rx.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match guard.try_recv() {
                    Ok(val) => Ok(into_raw(val)),
                    Err(_) => Ok(into_raw(ValueWord::none())),
                }
            }
            _ => Err(VMError::RuntimeError(
                "try_recv() called on channel sender (use on receiver)".to_string(),
            )),
        },
        _ => Err(VMError::RuntimeError(
            "try_recv() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.close()` — v2 ABI.
pub fn v2_channel_close(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let heap = vw
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("close()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => {
            data.close();
            Ok(into_raw(ValueWord::none()))
        }
        _ => Err(VMError::RuntimeError(
            "close() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.is_closed()` — v2 ABI.
pub fn v2_channel_is_closed(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let heap = vw
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("is_closed()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => Ok(into_raw(ValueWord::from_bool(data.is_closed()))),
        _ => Err(VMError::RuntimeError(
            "is_closed() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.is_sender()` — v2 ABI.
pub fn v2_channel_is_sender(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let heap = vw
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("is_sender()", "channel"))?;
    match heap {
        HeapValue::Channel(data) => Ok(into_raw(ValueWord::from_bool(data.is_sender()))),
        _ => Err(VMError::RuntimeError(
            "is_sender() called on non-channel value".to_string(),
        )),
    }
}
