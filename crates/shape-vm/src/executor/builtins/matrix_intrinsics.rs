//! Wave 5d (phase-1b-vm): matrix intrinsic dispatch deferred.
//!
//! `handle_matrix_intrinsic` was the entry point for `IntrinsicMatMul*`,
//! `IntrinsicMatAdd`, and `IntrinsicMatSub`. The previous body relied
//! on `pop_builtin_args -> Vec<ValueWord>` and a runtime-side intrinsic
//! taking `&[ValueWord]` — both deleted.
//!
//! Wave 5a flipped the dispatch shape; the matrix-intrinsic arms in
//! `vm_impl/builtins.rs` `todo!` for Wave 5d. This file is intentionally
//! empty post-Wave 5b.
