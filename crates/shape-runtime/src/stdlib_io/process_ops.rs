//! Process operation implementations for the io module.
//!
//! Phase 2c partial: registrations DEFERRED. The 12 process functions
//! (spawn, exec, shell, process_wait, process_kill, process_status,
//! process_write, process_read, process_read_err, process_read_line,
//! process_close, process_pid) span cluster #2 (IoHandle) and the
//! `Array<string>` FromSlot sub-cluster (the args param needs
//! `Vec<Arc<String>>`-typed input which has no FromSlot impl yet —
//! TypedArrayData has no String variant). Held until the
//! Array<string>-marshal sub-cluster lands.
//!
//! Surfaced for next session as part of the cluster #2 group 1 +
//! Array<string>-marshal sub-cluster work. The original ValueWord-based
//! bodies have been deleted.

use crate::module_exports::ModuleExports;

/// Register process-IO functions on the io module. Currently empty
/// pending the Array<string>-marshal sub-cluster + cluster #2 follow-up.
pub fn register_process_io(_module: &mut ModuleExports) {
    // Deferred: spawn, exec, shell, process_wait, process_kill,
    // process_status, process_write, process_read, process_read_err,
    // process_read_line, process_close, process_pid.
}
