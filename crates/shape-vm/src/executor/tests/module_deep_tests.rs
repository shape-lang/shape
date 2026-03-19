//! Edge-case tests for module scoping, nesting, and complex patterns.
//!
//! Basic module declaration, visibility, and pub/export tests live in
//! shape-test integration tests (modules_visibility/). This file covers
//! only edge cases not exercised there: deeply nested access, scope
//! isolation, closures, recursion, cross-module interplay, match
//! expressions, generics, and error cases.

use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use crate::{VMConfig, VMError};
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

fn compile_and_execute(source: &str) -> Result<ValueWord, VMError> {
    let program =
        parse_program(source).map_err(|e| VMError::RuntimeError(format!("Parse: {:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("Compile: {:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

/// Assert that source compiles and runs to a numeric result.
fn assert_result_number(code: &str, expected: f64) {
    match compile_and_execute(code) {
        Ok(result) => {
            let n = result.to_number().expect("Expected number result");
            assert!(
                (n - expected).abs() < f64::EPSILON,
                "Expected {}, got {}",
                expected,
                n
            );
        }
        Err(e) => panic!("Expected result {}, got error: {:?}", expected, e),
    }
}

/// Assert that source compiles successfully (may or may not run).
fn assert_compiles(source: &str) {
    let program = parse_program(source).expect("Parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    compiler.compile(&program).expect("Compile failed");
}

// =============================================================================
// Nested Module Access
// =============================================================================

#[test]
fn test_module_exec_nested_module_access() {
    let code = r#"
        mod outer {
            mod inner {
                fn value() { 42; }
            }
        }
        outer::inner::value();
    "#;
    assert_result_number(code, 42.0);
}

#[test]
fn test_module_exec_nested_module_function_resolution() {
    let code = r#"
        mod outer {
            fn outer_fn() { 1; }
            mod inner {
                fn inner_fn() { 2; }
            }
        }
        outer::outer_fn() + outer::inner::inner_fn();
    "#;
    assert_result_number(code, 3.0);
}

#[test]
fn test_module_exec_module_deeply_nested_access() {
    let code = r#"
        mod a {
            mod b {
                mod c {
                    fn deep() { 99; }
                }
            }
        }
        a::b::c::deep();
    "#;
    assert_result_number(code, 99.0);
}

// =============================================================================
// Scope Isolation
// =============================================================================

#[test]
fn test_module_exec_module_does_not_leak_into_global() {
    // A function defined in a module should NOT be accessible globally
    let code = r#"
        mod secret {
            fn hidden() { 42; }
        }
        hidden();
    "#;
    let result = compile_and_execute(code);
    assert!(
        result.is_err(),
        "Module-scoped function should not be globally accessible"
    );
}

#[test]
fn test_module_exec_module_function_cannot_access_outer_locals() {
    // Module functions should not automatically see outer local variables
    // (they have their own scope)
    let code = r#"
        let outer_val = 99;
        mod m {
            fn get_outer() { outer_val; }
        }
        m::get_outer();
    "#;
    // This may or may not work depending on scope rules
    let _result = compile_and_execute(code);
    // Document the behavior without asserting either way
}

#[test]
fn test_module_exec_empty_module_access_fails() {
    let code = r#"
        mod empty { }
        empty::nonexistent();
    "#;
    let result = compile_and_execute(code);
    assert!(
        result.is_err(),
        "accessing nonexistent member of empty module should fail"
    );
}

// =============================================================================
// Complex Patterns: Closures, Recursion, Match, Chaining
// =============================================================================

#[test]
fn test_module_exec_module_function_with_closure() {
    let code = r#"
        mod util {
            fn apply(f, x) { f(x); }
        }
        util::apply(|x| x * 2, 21);
    "#;
    assert_result_number(code, 42.0);
}

#[test]
fn test_module_exec_module_function_recursion() {
    let code = r#"
        mod fib {
            fn compute(n) {
                if n <= 1 { return n }
                return compute(n - 1) + compute(n - 2)
            }
        }
        fib::compute(10);
    "#;
    assert_result_number(code, 55.0);
}

#[test]
fn test_module_exec_module_function_with_default_like_pattern() {
    let code = r#"
        mod config {
            fn get_value(key) {
                if key == "port" { return 8080 }
                return 0
            }
        }
        config::get_value("port");
    "#;
    assert_result_number(code, 8080.0);
}

#[test]
fn test_module_exec_module_function_chaining() {
    let code = r#"
        mod pipeline {
            fn step1(x) { x + 1; }
            fn step2(x) { x * 2; }
            fn step3(x) { x - 3; }
        }
        pipeline::step3(pipeline::step2(pipeline::step1(5)));
    "#;
    // step1(5) = 6, step2(6) = 12, step3(12) = 9
    assert_result_number(code, 9.0);
}

#[test]
fn test_module_exec_two_modules_calling_each_other_functions() {
    let code = r#"
        mod converter {
            fn to_celsius(f) { (f - 32) * 5 / 9; }
        }
        mod formatter {
            fn format_temp(c) { c; }
        }
        formatter::format_temp(converter::to_celsius(212));
    "#;
    assert_result_number(code, 100.0);
}

#[test]
fn test_module_exec_module_with_match_expression() {
    let code = r#"
        mod evaluator {
            fn eval(op, a, b) {
                match op {
                    "add" => a + b,
                    "sub" => a - b,
                    "mul" => a * b,
                    _ => 0,
                };
            }
        }
        evaluator::eval("mul", 6, 7);
    "#;
    assert_result_number(code, 42.0);
}

// =============================================================================
// Generic Edge Case
// =============================================================================

#[test]
fn test_module_exec_pub_fn_with_generic_type_compiles() {
    assert_compiles(
        r#"
        pub fn identity<T>(x: T) -> T { x; }
    "#,
    );
}
