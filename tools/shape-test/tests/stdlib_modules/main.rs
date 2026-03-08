//! Integration tests for csv, msgpack, set, and crypto stdlib modules.
//!
//! These modules are loaded as global objects via `.with_stdlib()`.
//! The semantic analyzer does not recognize stdlib globals, so tests
//! use runtime assertions (expect_run_ok, expect_output, etc.).

mod crypto_tests;
mod csv_tests;
mod msgpack_tests;
mod set_tests;
