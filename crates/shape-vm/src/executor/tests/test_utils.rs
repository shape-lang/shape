//! Shared test utilities for executor tests.
//!
//! Provides common helpers for compiling and executing Shape source code
//! in tests, reducing duplication across test modules. Mirrors the
//! kind-hint API documented in `crate::test_utils` (see that module for
//! the design rationale).

use crate::VMConfig;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use crate::type_tracking::NativeKind;
use shape_value::{VMError, ValueWord, ValueWordExt};

/// Compile + execute Shape source code, returning the **raw u64 bits**
/// at the top of stack plus the program's declared top-level return
/// kind (if any). Mirrors `crate::test_utils::eval_raw`.
pub fn eval_raw(source: &str) -> (u64, Option<NativeKind>) {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler.compile(&program).expect("compile failed");
    let kind = top_level_return_kind(&bytecode);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let bits = vm.execute_raw(None).expect("execution failed");
    (bits, kind)
}

#[inline]
fn top_level_return_kind(program: &crate::bytecode::BytecodeProgram) -> Option<NativeKind> {
    let kind = program.top_level_frame.as_ref()?.return_kind;
    match kind {
        NativeKind::Unknown => None,
        _ => Some(kind),
    }
}

/// Compile and execute Shape source code, returning the final value as
/// a tagged `ValueWord`. Synthesises from raw bits per the program's
/// declared top-level return kind, or passthrough when unknown.
/// Panics on parse, compile, or execution failure.
pub fn eval(source: &str) -> ValueWord {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
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
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|v| v.clone())
}

/// Compile + execute and synthesise a tagged `ValueWord` from the raw
/// bits per the supplied `NativeKind`. Use when a test needs typed-bits
/// decoding for a program that doesn't declare a top-level return type.
pub fn eval_with_kind(source: &str, expected: NativeKind) -> ValueWord {
    let (bits, _) = eval_raw(source);
    crate::executor::dispatch::synthesize_value_word_from_raw(bits, Some(expected))
}

/// Compile Shape source code and return the bytecode program.
/// Panics on parse or compile failure.
pub fn compile(source: &str) -> crate::bytecode::BytecodeProgram {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
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
    compiler.set_source(source);
    let bytecode = compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .expect("compile failed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed").clone()
}

/// Compile Shape source code with prelude, returning a Result.
/// Useful for testing expected compile/runtime errors with stdlib.
pub fn compile_with_prelude(source: &str) -> Result<crate::bytecode::BytecodeProgram, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    let (graph, stdlib_names, prelude_imports) =
        crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
            .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;
    compiler.set_source(source);
    compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))
}

/// Evaluate Shape source and return the result as a native `i64`.
/// Panics if the value cannot be decoded as an integer.
pub fn eval_typed_i64(source: &str) -> i64 {
    eval_with_kind(source, NativeKind::Int64)
        .as_i64()
        .expect("eval_typed_i64: result is not an integer")
}

/// Evaluate Shape source and return the result as a native `f64`.
/// Panics if the value cannot be decoded as a float.
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
