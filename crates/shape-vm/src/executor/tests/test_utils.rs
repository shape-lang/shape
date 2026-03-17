//! Shared test utilities for executor tests.
//!
//! Provides common helpers for compiling and executing Shape source code
//! in tests, reducing duplication across test modules.

use crate::VMConfig;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};

/// Compile and execute Shape source code, returning the final value.
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
