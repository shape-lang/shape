//! Async file operation implementations for the io module.
//!
//! Phase 2c partial: registrations DEFERRED. The 5 async file functions
//! (io.read_file_async, io.write_file_async, io.append_file_async,
//! io.read_bytes_async, io.exists_async) are path-only and therefore
//! mechanical migrations using `register_typed_async_fn_N` with
//! `Arc<String>` parameters. Held with the cluster so they land
//! alongside the rest of the stdlib_io path-mass migration in one
//! coherent unit.
//!
//! Surfaced for next session as part of the `stdlib_io path-mass`
//! follow-up workstream. The original ValueWord-based async function
//! bodies have been deleted.

use crate::module_exports::ModuleExports;

/// Register async file-IO functions on the io module. Currently empty
/// pending the next-session path-mass migration.
pub fn register_async_file_io(_module: &mut ModuleExports) {
    // Deferred: io.read_file_async, io.write_file_async, io.append_file_async,
    // io.read_bytes_async, io.exists_async.
}
