//! Shared test utilities for shape-vm tests.
//!
//! Provides common helpers for compiling and executing Shape source code
//! in tests, reducing duplication across test modules.
//!
//! # Post-W11 kinded API
//!
//! After the W7-W10 strict-typing waves the VM's stack ABI is kinded
//! (ADR-006 §2.7) and the deleted `ValueWord` / `ValueWordExt` /
//! `synthesize_value_word_from_raw` carriers no longer exist. The
//! test-helper layer is rebuilt around `KindedSlot`:
//!
//! 1. [`eval_raw`] — compile + execute, return the **raw u64 bits** at the
//!    top of stack plus the program's declared top-level return kind
//!    (if any). The kind comes from
//!    `BytecodeProgram::top_level_frame.return_kind` (now
//!    `Option<NativeKind>` per §2.7.8/Q10); `None` only when no frame is
//!    attached or the kind isn't stamped.
//! 2. [`eval`] — compile + execute, return the result as a [`KindedSlot`].
//!    Use the per-kind accessors (`.as_i64()`, `.as_f64()`, `.as_bool()`,
//!    `.as_str()`) for assertions.
//! 3. [`eval_typed_i64`] / [`eval_typed_f64`] / [`eval_typed_bool`] —
//!    decode the result via the kinded accessors and return the native
//!    Rust value directly.

use crate::compiler::BytecodeCompiler;
use crate::executor::{VMConfig, VirtualMachine};
use crate::type_tracking::NativeKind;
use shape_value::{KindedSlot, VMError};

// ─── Layer 1: raw + kind-hint ────────────────────────────────────────────

/// Compile + execute Shape source code, returning the **raw u64 bits** at
/// the top of stack plus the program's declared top-level return kind
/// (if any). The kind comes from
/// `BytecodeProgram::top_level_frame.return_kind` (post-§2.7.8/Q10
/// `Option<NativeKind>`); `None` is returned when no frame is attached or
/// the compiler did not stamp a kind.
///
/// Use this when you want to bypass the `KindedSlot` carrier — e.g. you
/// know the program returns raw `i64` bits and want to handle them
/// yourself.
///
/// Panics on parse, compile, or execution failure.
pub fn eval_raw(source: &str) -> (u64, Option<NativeKind>) {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let compiler = BytecodeCompiler::new();
    let bytecode = compiler.compile(&program).expect("compile failed");
    let kind = top_level_return_kind(&bytecode);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let bits = vm.execute_raw(None).expect("execution failed");
    (bits, kind)
}

/// Read the program's declared top-level return kind. Returns `None` when
/// the program does not have a typed top-level frame, or when the frame's
/// `return_kind` field is `None` (i.e. the compiler did not stamp a
/// proven kind). Post-ADR-006 §2.7.5.1, the deleted `NativeKind::Unknown`
/// sentinel is no longer observable here — `Option<NativeKind>` is the
/// new "kind not stamped" carrier.
#[inline]
fn top_level_return_kind(program: &crate::bytecode::BytecodeProgram) -> Option<NativeKind> {
    program.top_level_frame.as_ref()?.return_kind
}

// ─── Layer 2: KindedSlot (default) ──────────────────────────────────────

/// Compile and execute Shape source code, returning the final value as a
/// [`KindedSlot`] carrier (ADR-006 §2.7). Panics on parse, compile, or
/// execution failure.
pub fn eval(source: &str) -> KindedSlot {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let compiler = BytecodeCompiler::new();
    let bytecode = compiler.compile(&program).expect("compile failed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed")
}

/// Compile and execute Shape source code, returning a Result.
/// Useful when testing error conditions.
pub fn eval_result(source: &str) -> Result<KindedSlot, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let compiler = BytecodeCompiler::new();
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None)
}

/// Compile + execute Shape source code and build a [`KindedSlot`] from
/// the raw bits per the supplied `NativeKind`. Use this when a test
/// asserts via `as_i64()` / `as_f64()` / `as_bool()` etc. on a program
/// whose top-level return kind isn't declared but you still want typed
/// decoding under a specific kind.
///
/// For convenience-wrapper pendants that return native Rust types
/// directly, see [`eval_typed_i64`], [`eval_typed_f64`], etc.
pub fn eval_with_kind(source: &str, expected: NativeKind) -> KindedSlot {
    let (bits, _) = eval_raw(source);
    KindedSlot::new(shape_value::ValueSlot::from_raw(bits), expected)
}

/// Compile Shape source code and return the bytecode program.
/// Panics on parse or compile failure.
pub fn compile(source: &str) -> crate::bytecode::BytecodeProgram {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let compiler = BytecodeCompiler::new();
    compiler.compile(&program).expect("compile failed")
}

/// Compile Shape source code with prelude items prepended.
/// This is needed for tests that use stdlib features like comptime builtins.
/// Panics on parse or compile failure.
pub fn eval_with_prelude(source: &str) -> KindedSlot {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    let (graph, stdlib_names, prelude_imports) =
        crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
            .expect("graph build failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;
    let bytecode = compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .expect("compile failed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed")
}

/// Compile Shape source code with prelude, returning a Result.
/// Useful for testing expected compile/runtime errors with stdlib.
pub fn compile_with_prelude(
    source: &str,
) -> Result<crate::bytecode::BytecodeProgram, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    let (graph, stdlib_names, prelude_imports) =
        crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
            .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;
    compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))
}

// ─── Layer 3: native-type convenience helpers ────────────────────────────
//
// These return the native Rust value directly, skipping the `KindedSlot`
// indirection. Useful in tests that follow the pattern
//
//     assert_eq!(eval(src).as_i64().unwrap(), 42);
//
// which becomes the cleaner
//
//     assert_eq!(eval_typed_i64(src), 42);

/// Evaluate Shape source and return the result as a native `i64`. Panics
/// if the value cannot be decoded as an integer.
pub fn eval_typed_i64(source: &str) -> i64 {
    eval_with_kind(source, NativeKind::Int64)
        .as_i64()
        .expect("eval_typed_i64: result is not an integer")
}

/// Evaluate Shape source and return the result as a native `f64`. Panics
/// if the value cannot be decoded as a float.
pub fn eval_typed_f64(source: &str) -> f64 {
    eval_with_kind(source, NativeKind::Float64)
        .as_f64()
        .expect("eval_typed_f64: result is not a float")
}

/// Evaluate Shape source and return the result as a native `bool`.
/// Panics if the value cannot be decoded as a boolean.
pub fn eval_typed_bool(source: &str) -> bool {
    eval_with_kind(source, NativeKind::Bool)
        .as_bool()
        .expect("eval_typed_bool: result is not a boolean")
}

// Tests for this module are gated `deep-tests` post-W11: the original
// kind-hint API tests asserted on the deleted ValueWord synthesizer
// (`synthesize_value_word_from_raw`). Restoring them requires the
// kinded-carrier round-trip surface (Phase-2c reentry per ADR-006 §2.7.4).
#[cfg(all(test, feature = "deep-tests"))]
mod kind_hint_api_tests {}
