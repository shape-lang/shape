//! Library half of the workspace automation crate.
//!
//! Logic that carries its own unit tests lives here rather than in `main.rs`,
//! so `cargo test --workspace --lib` (the `just test-fast` / `just test` tier)
//! runs it. `main.rs` stays a thin command-line surface over it.

pub mod perf_suite;
