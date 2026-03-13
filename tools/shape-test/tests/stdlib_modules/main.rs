//! Integration tests for csv, msgpack, set, and crypto stdlib modules.
//!
//! These modules are imported via `use std::core::<module>` and accessed
//! with `module::function()` syntax. Tests use runtime assertions
//! (expect_run_ok, expect_output, etc.).

mod crypto_tests;
mod csv_tests;
mod msgpack_tests;
mod set_tests;
