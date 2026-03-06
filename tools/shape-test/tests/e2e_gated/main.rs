//! Feature-gated E2E tests for language runtime extensions.
//!
//! These tests require external runtimes (Python, TypeScript) to be installed
//! and the corresponding Cargo features to be enabled:
//!   cargo test -p shape-test --features e2e-python
//!   cargo test -p shape-test --features e2e-typescript

#[cfg(feature = "e2e-python")]
mod python_interop;

#[cfg(feature = "e2e-typescript")]
mod typescript_interop;

// When no features are enabled, this binary still compiles but runs 0 tests.
