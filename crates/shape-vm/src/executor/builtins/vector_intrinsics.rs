//! Wave 5d (phase-1b-vm): vector SIMD intrinsic dispatch deferred.
//!
//! `handle_vector_intrinsic` was the entry point for `IntrinsicVec*`
//! variants. The previous body relied on
//! `pop_builtin_args -> Vec<ValueWord>` plus runtime-side intrinsics
//! taking `&[ValueWord]` — both deleted.
//!
//! Wave 5a flipped the dispatch shape; the `IntrinsicVec*` arms in
//! `vm_impl/builtins.rs` `todo!` for Wave 5d. This file is intentionally
//! empty post-Wave 5b.
