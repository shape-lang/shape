//! Method handlers for Channel type (MPSC sender/receiver endpoints).

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};

// ═══════════════════════════════════════════════════════════════════════════
// Channel sender methods
// ═══════════════════════════════════════════════════════════════════════════

/// `sender.send(value)` — send a value through the channel.
/// Returns true if sent successfully, false if the channel is closed.
pub fn handle_channel_send(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let value = args.get(1).cloned().unwrap_or_else(ValueWord::none);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("send() called on non-channel value".to_string()))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Sender { tx, closed, .. } => {
                if closed.load(std::sync::atomic::Ordering::Relaxed) {
                    vm.push_vw(ValueWord::from_bool(false))?;
                } else {
                    match tx.send(value) {
                        Ok(()) => vm.push_vw(ValueWord::from_bool(true))?,
                        Err(_) => vm.push_vw(ValueWord::from_bool(false))?,
                    }
                }
                Ok(())
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
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("recv() called on non-channel value".to_string()))?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Receiver { rx, .. } => {
                let guard = rx.lock().map_err(|e| {
                    VMError::RuntimeError(format!("Channel receiver poisoned: {}", e))
                })?;
                match guard.recv() {
                    Ok(val) => vm.push_vw(val)?,
                    Err(_) => vm.push_vw(ValueWord::none())?,
                }
                Ok(())
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
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("try_recv() called on non-channel value".to_string())
    })?;
    match heap {
        HeapValue::Channel(data) => match data.as_ref() {
            shape_value::heap_value::ChannelData::Receiver { rx, .. } => {
                let guard = rx.lock().map_err(|e| {
                    VMError::RuntimeError(format!("Channel receiver poisoned: {}", e))
                })?;
                match guard.try_recv() {
                    Ok(val) => vm.push_vw(val)?,
                    Err(_) => vm.push_vw(ValueWord::none())?,
                }
                Ok(())
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
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("close() called on non-channel value".to_string()))?;
    match heap {
        HeapValue::Channel(data) => {
            data.close();
            vm.push_vw(ValueWord::none())?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "close() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.is_closed()` — check if the channel has been closed.
pub fn handle_channel_is_closed(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("is_closed() called on non-channel value".to_string())
    })?;
    match heap {
        HeapValue::Channel(data) => {
            vm.push_vw(ValueWord::from_bool(data.is_closed()))?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "is_closed() called on non-channel value".to_string(),
        )),
    }
}

/// `channel.is_sender()` — returns true if this is the sender end.
pub fn handle_channel_is_sender(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("is_sender() called on non-channel value".to_string())
    })?;
    match heap {
        HeapValue::Channel(data) => {
            vm.push_vw(ValueWord::from_bool(data.is_sender()))?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "is_sender() called on non-channel value".to_string(),
        )),
    }
}
