//! Wave 5d (phase-1b-vm): VM intrinsic body stubs deferred.
//!
//! The intrinsic dispatch (`handle_intrinsic_builtin` in `runtime_delegated.rs`)
//! is unreachable today — every `Intrinsic*` arm in
//! `vm_impl/builtins.rs` is `todo!`-stubbed for Wave 5d. The previous
//! body wrappers (`vm_intrinsic_sum`, etc.) called into deleted
//! `ValueWord` machinery and are now dead.
//!
//! Submodules `math`, `signal`, `statistical` are kept as empty
//! placeholders so `mod intrinsics;` keeps resolving until the deferred
//! bodies land in Wave 5d.

pub mod math;
pub mod signal;
pub mod statistical;
