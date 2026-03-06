//! Integration tests for native C interop (FFI).
//!
//! Shape supports `extern "C" fn` declarations for calling native libraries.
//! These tests verify the syntax parsing and marshalling semantics. Most
//! are TDD since native interop requires .so fixtures not available in
//! the test environment.

mod ffi_syntax;
mod marshalling;
