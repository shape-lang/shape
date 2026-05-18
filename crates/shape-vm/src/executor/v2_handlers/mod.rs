//! v2 opcode handlers — typed struct field, typed array, and sized integer operations.

pub(crate) mod array;
pub(crate) mod field;
pub(crate) mod int;
pub(crate) mod typed_array_elem;
pub(crate) mod typed_map;
// W11-fup-C (Phase 3d, 2026-05-18): exposed `pub` so the JIT-side
// `crates/shape-jit/src/ffi/v2/mod.rs` allocators can call
// `stamp_elem_type` + the `ELEM_TYPE_*` constants at allocation time.
// See the matching `pub mod v2_handlers` annotation at
// `crates/shape-vm/src/executor/mod.rs:25` for the full §2.7.5
// stamp-at-compile-time rationale.
pub mod v2_array_detect;

#[cfg(test)]
mod integration_tests;
