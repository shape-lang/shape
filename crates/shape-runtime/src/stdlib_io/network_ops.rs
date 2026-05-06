//! Network operation implementations for the io module.
//!
//! Phase 2c partial: registrations DEFERRED. The 9 network functions
//! (tcp_connect, tcp_listen, tcp_accept, tcp_read, tcp_write, tcp_close,
//! udp_bind, udp_send, udp_recv) are cluster #2 (IoHandle) consumers
//! and migrate using `Arc<IoHandleData>` parameters per the option γ
//! shape now in `marshal.rs`. Held with the cluster so they land
//! alongside `process_ops` in one coherent network/process commit.
//!
//! Surfaced for next session as part of the cluster #2 group 1 follow-up.
//! file_ops.rs (the largest cluster #2 sub-cluster) is already migrated;
//! these 9 functions follow the same pattern but were deferred from this
//! session to keep the commit scope manageable. The original
//! ValueWord-based bodies have been deleted to make the absence visible.

use crate::module_exports::ModuleExports;

/// Register network-IO functions on the io module. Currently empty
/// pending the next-session cluster #2 follow-up migration.
pub fn register_network_io(_module: &mut ModuleExports) {
    // Deferred: tcp_connect, tcp_listen, tcp_accept, tcp_read, tcp_write,
    // tcp_close, udp_bind, udp_send, udp_recv.
}
