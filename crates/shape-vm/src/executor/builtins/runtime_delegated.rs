//! Wave 5d (phase-1b-vm): intrinsic dispatch handler deferred.
//!
//! `handle_intrinsic_builtin` was the entry point routing `Intrinsic*`
//! `BuiltinFunction` variants to their `vm_intrinsic_*` body wrappers.
//! Both sides of that handshake used deleted `ValueWord` machinery.
//!
//! Wave 5a flipped the dispatch in `vm_impl/builtins.rs` to consume
//! `&[KindedSlot]` directly — the handler indirection through this file
//! is now dead code (every `Intrinsic*` arm of `op_builtin_call`
//! `todo!`'s for Wave 5d).
//!
//! This file is intentionally empty post-Wave 5b. Wave 5d either
//! re-introduces a kind-aware dispatch helper or inlines the intrinsic
//! bodies directly into the dispatch arms.
