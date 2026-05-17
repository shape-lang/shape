//! Wave 5b (phase-1b-vm): `builtin_range` and `builtin_slice` migrated
//! to `crates/shape-vm/src/executor/builtins/array_ops.rs` as free fns
//! taking `&[KindedSlot]`. This file is intentionally empty post-migration;
//! kept as a placeholder so `mod generators;` in `builtins/mod.rs` keeps
//! resolving until follow-up cleanup retires the empty module.
