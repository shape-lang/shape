//! Integration tests for csv, msgpack, set, and crypto stdlib modules.
//!
//! These modules are imported via `use std::core::<module>` and accessed
//! with `module::function()` syntax. Tests use runtime assertions
//! (expect_run_ok, expect_output, etc.).
//!
//! ADR-006 §2.7.4 (W11-tail, 2026-05-10): `csv_tests` was written against
//! the deleted `ValueWord`/`ValueWordExt`/`vmarray_from_vec` API and the
//! pre-§2.7.10 `ModuleExports::invoke_export` shim. Gated until rewritten
//! onto the §2.7.10/Q11 `KindedSlot` dispatch shape.

mod crypto_tests;
#[cfg(any())]
mod csv_tests;
mod msgpack_tests;
mod set_tests;
