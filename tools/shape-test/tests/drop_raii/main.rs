//! Integration tests for automatic scope-based drop (RAII).
//!
//! Shape uses `trait Drop { fn drop(self) }` for deterministic cleanup.
//! The compiler emits DropCall opcodes at scope exit in reverse declaration
//! order (LIFO). These tests verify drop behavior across blocks, functions,
//! and control flow constructs.

mod control_flow;
mod ordering;
mod scope_drop;
