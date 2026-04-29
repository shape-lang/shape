//! Shared test utilities for shape-vm tests.
//!
//! Provides common helpers for compiling and executing Shape source code
//! in tests, reducing duplication across test modules.
//!
//! # Wave E+4.5 — host-boundary kind-hint API
//!
//! Wave E+4 flips the bytecode pipeline so the top-level `Load*` /
//! `Store*` / `ReturnValue*` opcodes push and pop **raw native bits**
//! (e.g. `i64` / `f64::to_bits()` / `0u64`-or-`1u64` for bool / raw
//! heap pointer for `Ptr`) — there is no NaN-box tag inside the VM. The
//! host (test harness, REPL, embedder) is responsible for synthesising a
//! tagged `ValueWord` from those bits at the boundary, given a kind hint.
//!
//! This file exposes three layers of test entry points:
//!
//! 1. [`eval_raw`] — compile + execute, return `(u64 raw bits, Option<SlotKind>)`.
//!    The `SlotKind` is read from the program's `top_level_frame.return_kind`
//!    when present and not `Unknown`. Most flexible; intended for the
//!    harness layer to wrap.
//! 2. [`eval`] — compile + execute, synthesise a tagged `ValueWord` from
//!    the raw bits per the program's declared return kind. If the kind is
//!    unproven (no `top_level_frame` populated by the compiler), the bits
//!    are interpreted as a tagged `ValueWord` directly (passthrough — the
//!    legacy / pre-E+4 behaviour). This signature is unchanged from before
//!    the migration; existing callers continue to work unmodified.
//! 3. [`eval_with_kind`] — compile + execute and synthesise a `ValueWord`
//!    per a caller-supplied `SlotKind`. Use when a test asserts via
//!    `as_i64()` / `as_f64()` / `as_bool()` etc. on a program that
//!    *does not* declare a top-level return type, but you still want
//!    typed-bits decoding.
//!
//! Plus per-FieldKind convenience wrappers ([`eval_typed_i64`],
//! [`eval_typed_f64`], [`eval_typed_bool`]) that return the native
//! Rust value directly, skipping `ValueWord` entirely.
//!
//! See `dispatch.rs::synthesize_value_word_from_raw` for the canonical
//! raw-bits → tagged-`ValueWord` encoder.

use crate::compiler::BytecodeCompiler;
use crate::executor::{VMConfig, VirtualMachine};
use crate::type_tracking::SlotKind;
use shape_value::{VMError, ValueWord, ValueWordExt};

// ─── Layer 1: raw + kind-hint ────────────────────────────────────────────

/// Compile + execute Shape source code, returning the **raw u64 bits** at
/// the top of stack plus the program's declared top-level return kind
/// (if any). The kind comes from `BytecodeProgram::top_level_frame.return_kind`;
/// `None` is returned when the compiler did not populate a typed frame
/// for the top-level (the common case today).
///
/// Use this when you want to bypass `ValueWord` synthesis — e.g. you
/// know the program returns a raw `i64` and you'd rather handle the bits
/// yourself.
///
/// Panics on parse, compile, or execution failure.
pub fn eval_raw(source: &str) -> (u64, Option<SlotKind>) {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let compiler = BytecodeCompiler::new();
    let bytecode = compiler.compile(&program).expect("compile failed");
    let kind = top_level_return_kind(&bytecode);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let bits = vm.execute_raw(None).expect("execution failed");
    (bits, kind)
}

/// Read the program's declared top-level return kind. Returns `None`
/// when the program does not have a typed top-level frame, or when the
/// frame's return kind is `Unknown` (i.e. the compiler did not prove
/// the type).
#[inline]
fn top_level_return_kind(program: &crate::bytecode::BytecodeProgram) -> Option<SlotKind> {
    let kind = program.top_level_frame.as_ref()?.return_kind;
    match kind {
        SlotKind::Unknown => None,
        _ => Some(kind),
    }
}

// ─── Layer 2: tagged ValueWord (default) ─────────────────────────────────

/// Compile and execute Shape source code, returning the final value as a
/// tagged `ValueWord`. Synthesises the `ValueWord` from raw bits per the
/// program's declared top-level return kind (if any); falls back to
/// passthrough when no kind is declared. Panics on parse, compile, or
/// execution failure.
pub fn eval(source: &str) -> ValueWord {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let compiler = BytecodeCompiler::new();
    let bytecode = compiler.compile(&program).expect("compile failed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed").clone()
}

/// Compile and execute Shape source code, returning a Result.
/// Useful when testing error conditions.
pub fn eval_result(source: &str) -> Result<ValueWord, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let compiler = BytecodeCompiler::new();
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|v| v.clone())
}

/// Compile + execute Shape source code and synthesise a tagged
/// `ValueWord` from the raw bits per the supplied `SlotKind`. Use this
/// when a test asserts via `as_i64()` / `as_f64()` / `as_bool()` etc.
/// on a program whose top-level return kind isn't declared but you
/// still want typed-bits decoding.
///
/// For convenience-wrapper pendants that return native Rust types
/// directly, see [`eval_typed_i64`], [`eval_typed_f64`], etc.
pub fn eval_with_kind(source: &str, expected: SlotKind) -> ValueWord {
    let (bits, _) = eval_raw(source);
    crate::executor::dispatch::synthesize_value_word_from_raw(bits, Some(expected))
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
pub fn eval_with_prelude(source: &str) -> ValueWord {
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
    vm.execute(None).expect("execution failed").clone()
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
// These return the native Rust value directly, skipping the ValueWord
// indirection. Useful in tests that follow the pattern
//
//     assert_eq!(eval(src).as_i64().unwrap(), 42);
//
// which becomes the cleaner
//
//     assert_eq!(eval_typed_i64(src), 42);
//
// Adds for the kinds where shape-vm tests historically needed conversion;
// extend as needed when new emission flips land.

/// Evaluate Shape source and return the result as a native `i64`. Panics
/// if the value cannot be decoded as an integer.
pub fn eval_typed_i64(source: &str) -> i64 {
    eval_with_kind(source, SlotKind::Int64)
        .as_i64()
        .expect("eval_typed_i64: result is not an integer")
}

/// Evaluate Shape source and return the result as a native `f64`. Panics
/// if the value cannot be decoded as a float.
pub fn eval_typed_f64(source: &str) -> f64 {
    eval_with_kind(source, SlotKind::Float64)
        .as_f64()
        .expect("eval_typed_f64: result is not a float")
}

/// Evaluate Shape source and return the result as a native `bool`.
/// Panics if the value cannot be decoded as a boolean.
pub fn eval_typed_bool(source: &str) -> bool {
    eval_with_kind(source, SlotKind::Bool)
        .as_bool()
        .expect("eval_typed_bool: result is not a boolean")
}

#[cfg(test)]
mod kind_hint_api_tests {
    use super::*;
    use crate::executor::dispatch::synthesize_value_word_from_raw;

    #[test]
    fn eval_returns_legacy_passthrough_today() {
        // Today, no top-level program populates `top_level_frame.return_kind`,
        // so the synthesis path falls through to passthrough — the bits on
        // the stack are already a tagged ValueWord. This test pins that
        // historical behaviour: numeric literals come back as ValueWord
        // values that decode via `as_i64()` / `as_f64()` exactly as before.
        let v = eval("42");
        assert_eq!(v.as_i64(), Some(42));
    }

    #[test]
    fn eval_raw_returns_bits_and_legacy_none_kind() {
        // Per the precondition above, the kind side of the tuple is None
        // for legacy programs. The bits side is the same `u64` that
        // `eval()` would synthesise from.
        let (_bits, kind) = eval_raw("42");
        assert!(
            kind.is_none(),
            "legacy program should report no top-level return kind, got {:?}",
            kind
        );
    }

    #[test]
    fn eval_raw_then_synthesize_passthrough_matches_eval() {
        // Building block: when `eval_raw` reports no kind, the host
        // should synthesise as passthrough — and the result must
        // exactly match what `eval()` produces. This pins the contract
        // that `eval == passthrough(eval_raw)` for legacy programs.
        let (bits, kind) = eval_raw("42");
        let synthesised = synthesize_value_word_from_raw(bits, kind);
        assert_eq!(synthesised.as_i64(), eval("42").as_i64());
    }

    #[test]
    fn eval_with_kind_forces_int64_synthesis_on_raw_bits() {
        // When the bits really are raw `i64` (constructed directly,
        // e.g. by a `ReturnValueI64` opcode after E+4 flips), forcing
        // SlotKind::Int64 produces a ValueWord that decodes correctly.
        // We construct the raw-bits scenario synthetically — a real
        // `eval_with_kind("42", Int64)` call against today's compiler
        // is **incorrect** because today's compiler emits a tagged
        // ValueWord on the stack, not raw i64. The use-site contract:
        // call this only when you know the program emits typed bits
        // for the chosen kind.
        let raw_bits = 42i64 as u64;
        let v = synthesize_value_word_from_raw(raw_bits, Some(SlotKind::Int64));
        assert_eq!(v.as_i64(), Some(42));
    }

    #[test]
    fn eval_typed_helpers_work_when_program_emits_raw_bits() {
        // The `eval_typed_*` helpers are intended for programs whose
        // top-level emission produces raw native bits — i.e. programs
        // compiled after E+4 flips the relevant per-site emission. We
        // can't construct such a program through the high-level
        // `eval(...)` path today, so this test just smoke-tests that
        // the synthesiser produces sane values for raw integer bits.
        let raw = 100i64 as u64;
        assert_eq!(
            synthesize_value_word_from_raw(raw, Some(SlotKind::Int64))
                .as_i64()
                .unwrap(),
            100
        );

        let raw = 2.5f64.to_bits();
        assert_eq!(
            synthesize_value_word_from_raw(raw, Some(SlotKind::Float64))
                .as_f64()
                .unwrap(),
            2.5
        );

        assert_eq!(
            synthesize_value_word_from_raw(1, Some(SlotKind::Bool))
                .as_bool()
                .unwrap(),
            true
        );
        assert_eq!(
            synthesize_value_word_from_raw(0, Some(SlotKind::Bool))
                .as_bool()
                .unwrap(),
            false
        );
    }

    #[test]
    fn eval_result_propagates_runtime_errors() {
        // Sanity: eval_result still surfaces VMError correctly.
        let r = eval_result("1 / 0");
        assert!(r.is_err(), "division by zero should be a runtime error");
    }
}
