#![allow(clippy::result_large_err)]

//! JIT Compiler Module for Shape
//!
//! Compiles Shape bytecode to native x86-64/ARM machine code using Cranelift
//! for high-performance strategy execution in backtesting.
//!
//! # Module Structure
//!
//! - `ffi::jit_kinds` - JIT-specific heap kinds and allocation types
//! - `ffi::value_ffi` - NaN-boxing value encoding/decoding helpers
//! - `context` - JITContext, JITDataFrame, and related structures
//! - `ffi` - FFI functions called from JIT-compiled code
//! - `loop_analysis` - Loop analysis for JIT optimization
//! - `osr_compiler` - OSR (On-Stack Replacement) loop compilation
//! - `ffi_refs` - FFI function references for heap operations
//! - `compiler` - JITCompiler implementation (split into logical modules)
//! - `core` - Legacy re-exports and tests

mod compiler;
pub mod context;
mod core;
pub mod error;
pub mod executor;
// JIT FFI trampolines are deliberate raw-pointer entry points called from
// Cranelift-generated machine code. They take/deref raw pointers by ABI
// necessity; marking them `unsafe` would change the JIT-callable signatures
// the codegen layer relies on. The deref contract is upheld by the codegen
// site, not the Rust type system, so the lint is allowed module-wide here.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub mod ffi;
pub(crate) mod ffi_refs;
// FFI symbol-table entry points for series/vector/time intrinsics. Registered
// by name via the JIT symbol registry; many are staged ahead of their codegen
// call sites, so dead_code is expected and intentional here.
#[allow(dead_code)]
mod ffi_symbols;
// Foreign-function bridge scaffolding staged ahead of its JIT call sites.
#[allow(dead_code)]
mod foreign_bridge;
pub mod jit_array;
pub mod jit_cache;
pub mod jit_matrix;
pub mod loop_analysis;
// v2 JIT codegen scaffolding (typed MIR→IR, struct/string/int lowering, call
// ABI). Staged ahead of full v2 wiring; dead_code is expected WIP here.
#[allow(dead_code)]
pub mod mir_compiler;
pub mod mixed_table;
mod numeric_compiler;
// `mod optimizer` was deleted by #192 (ADR-018 §6, wire-or-delete): all
// thirteen components had zero non-test callers. The per-component
// dispositions, and the evidence a future re-derivation owes for each, are in
// docs/program/workstreams/192-dead-opt-dispositions.md.
pub mod osr_compiler;
pub mod worker;
// #117 / R15: execution-level tripwires for the NativeExecutionWitness. Gated
// behind `deep-tests` like the other whole-prelude JIT execution tests.
#[cfg(all(test, feature = "deep-tests"))]
mod witness_tripwires;
// #191 / ADR-018 §5: runtime negative controls for bounds-check elision —
// an out-of-range access of each widened index shape must still trap. Same
// gating rationale as `witness_tripwires`.
#[cfg(all(test, feature = "deep-tests"))]
mod bounds_elision_traps;

// Re-export commonly used items at module level
pub use context::*;
pub use error::JitError;
pub use executor::JITExecutor;
pub use ffi::jit_kinds::*;
pub use ffi::value_ffi::*;

pub use shape_ast as ast;
pub use shape_runtime as runtime;

// Re-export JITCompiler and related items from compiler module
pub use self::compiler::JITCompiler;
// JITKernelCompiler re-export removed — runtime-side
// `shape_runtime::simulation` module bulldozed in 2601ba7. See
// `compiler/setup.rs` comment block. Re-introduction owned by the
// simulation-kernel-extension-rebuild workstream.
pub use self::compiler::JitParityEntry;
pub use self::compiler::JitParityTarget;
pub use self::compiler::JitPreflightReport;
pub use self::compiler::build_full_builtin_parity_matrix;
pub use self::compiler::build_full_opcode_parity_matrix;
pub use self::compiler::build_program_parity_matrix;
pub use self::compiler::can_jit_compile;
pub use self::compiler::get_incomplete_opcodes;
pub use self::compiler::get_unsupported_opcodes;
pub use self::compiler::preflight_blob_jit_compatibility;
pub use self::compiler::preflight_instructions;
pub use self::compiler::preflight_jit_compatibility;
pub use self::osr_compiler::{OsrCompilationResult, compile_osr_loop};
pub use self::worker::JitCompilationBackend;
