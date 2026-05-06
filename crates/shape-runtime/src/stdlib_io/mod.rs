//! Native `io` module for file system, network, and process operations.
//!
//! Exports (post-cluster-#2 group 1 file_ops migration):
//!
//! ## Migrated (cluster #2 group 1 file_ops + path-mass file_ops)
//! - File handle: io.open, io.read_to_string, io.read, io.read_bytes,
//!   io.write, io.close, io.flush
//! - File path: io.exists, io.stat, io.is_file, io.is_dir, io.mkdir,
//!   io.remove, io.rename, io.read_dir, io.read_gzip, io.write_gzip
//!
//! ## Deferred (next session)
//! - Path utilities (path_ops): io.join (varargs), io.dirname, io.basename,
//!   io.extension, io.resolve. Blocked on the varargs-marshal sub-cluster
//!   for io.join; the other 4 are mechanical and held with the cluster.
//! - Async file ops (async_file_ops): io.read_file_async, io.write_file_async,
//!   io.append_file_async, io.read_bytes_async, io.exists_async. Path-only,
//!   mechanical migrations using `register_typed_async_fn_N`.
//! - Network ops (network_ops): tcp_connect/listen/accept/read/write/close,
//!   udp_bind/send/recv. Cluster #2 IoHandle consumers — same option γ
//!   shape as file_ops; deferred to keep this commit's scope manageable.
//! - Process ops (process_ops): spawn, exec, shell, process_*. Blocked on
//!   the `Array<string>`-marshal sub-cluster for `args` params plus
//!   cluster #2 for IoHandle returns.
//!
//! See `docs/defections.md` 2026-05-06 cluster #2 entry +
//! `marshal-optional-args` entry for the architectural decisions
//! underlying the migrated functions.

pub mod async_file_ops;
pub mod file_ops;
pub mod network_ops;
pub mod path_ops;
pub mod process_ops;

use crate::module_exports::ModuleExports;

/// Create the `io` module with file system, network, and process operations.
pub fn create_io_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::io");
    module.description = "File system, network, and process operations".to_string();

    // Migrated this session (cluster #2 group 1 file_ops + path-mass file_ops).
    file_ops::register_file_io_handle_ops(&mut module);
    file_ops::register_file_path_ops(&mut module);

    // Deferred to next session (each subfile's register_X is a no-op stub).
    path_ops::register_path_io(&mut module);
    async_file_ops::register_async_file_io(&mut module);
    network_ops::register_network_io(&mut module);
    process_ops::register_process_io(&mut module);

    module
}
