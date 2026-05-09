//! Wave 5d (phase-1b-vm): signal intrinsic bodies deferred.
//!
//! Body fns (`vm_intrinsic_diff`, `vm_intrinsic_pct_change`,
//! `vm_intrinsic_rolling_*`, etc.) were VM-side wrappers around
//! `shape_runtime::intrinsics::signal`. The previous bodies used
//! deleted `ValueWord` machinery; they are unreachable today (the
//! dispatch in `vm_impl/builtins.rs`'s `Intrinsic*` arms is `todo!`-
//! stubbed for Wave 5d).
//!
//! This file is intentionally empty post-Wave 5b. Wave 5d migrates
//! the bodies to `&[KindedSlot] -> Result<KindedSlot, VMError>`.
