//! Wave 5d (phase-1b-vm): math intrinsic bodies deferred.
//!
//! Body fns (`vm_intrinsic_sum`, `vm_intrinsic_mean`, etc.) were the
//! VM-side wrappers around `shape_runtime::intrinsics::math`. The
//! previous bodies used deleted `ValueWord` machinery; they are
//! unreachable today (the dispatch in `vm_impl/builtins.rs`'s
//! `Intrinsic*` arms is `todo!`-stubbed for Wave 5d).
//!
//! This file is intentionally empty post-Wave 5b. Wave 5d migrates
//! the bodies to `&[KindedSlot] -> Result<KindedSlot, VMError>`.
