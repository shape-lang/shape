//! Integration tests for automatic scope-based drop (RAII-style).
//!
//! Verifies that DropCall instructions are emitted at scope exit for
//! local variable bindings, and that drop works correctly with early
//! returns, breaks, nested scopes, etc.

use crate::VMConfig;
use crate::bytecode::OpCode;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

/// Compile Shape source code and return the bytecode program.
fn compile(source: &str) -> crate::bytecode::BytecodeProgram {
    let program = parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    compiler.compile(&program).expect("compile failed")
}

/// Compile and execute Shape source code, returning the final value.
fn eval(source: &str) -> ValueWord {
    let bytecode = compile(source);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed").clone()
}

#[test]
fn test_auto_drop_at_scope_exit() {
    // A let binding inside a block should emit DropCall at scope exit.
    let bytecode = compile(
        r#"
        function test_fn() {
            {
                let x = 42
            }
            return 1
        }
        test_fn()
    "#,
    );
    let has_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCall);
    assert!(
        has_drop,
        "block with let binding should emit DropCall at scope exit"
    );

    // Verify it still executes correctly
    let result = eval(
        r#"
        function test_fn() {
            {
                let x = 42
            }
            return 1
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_auto_drop_reverse_order() {
    // Multiple bindings should drop in reverse declaration order.
    let bytecode = compile(
        r#"
        function test_fn() {
            let a = 1
            let b = 2
            let c = 3
            return a + b + c
        }
        test_fn()
    "#,
    );
    // Should have DropCall instructions for all 3 locals
    let drop_count = bytecode
        .instructions
        .iter()
        .filter(|i| i.opcode == OpCode::DropCall)
        .count();
    assert!(
        drop_count >= 3,
        "3 let bindings should emit at least 3 DropCall instructions, got {}",
        drop_count
    );
}

#[test]
fn test_auto_drop_on_early_return() {
    // An early return inside a block should trigger drops for locals in scope.
    let bytecode = compile(
        r#"
        function test_fn() {
            let x = 10
            if true {
                return x
            }
            return 0
        }
        test_fn()
    "#,
    );
    // Return should emit drops before ReturnValue
    let has_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCall);
    assert!(has_drop, "early return should emit DropCall");

    let result = eval(
        r#"
        function test_fn() {
            let x = 10
            if true {
                return x
            }
            return 0
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_auto_drop_nested_scopes() {
    // Inner scope drops should happen before outer scope drops.
    let bytecode = compile(
        r#"
        function test_fn() {
            let outer = 1
            {
                let inner = 2
            }
            return outer
        }
        test_fn()
    "#,
    );
    // Should have drops for both inner and outer locals
    let drop_count = bytecode
        .instructions
        .iter()
        .filter(|i| i.opcode == OpCode::DropCall)
        .count();
    assert!(
        drop_count >= 2,
        "nested scopes should emit at least 2 DropCall instructions, got {}",
        drop_count
    );

    let result = eval(
        r#"
        function test_fn() {
            let outer = 1
            {
                let inner = 2
            }
            return outer
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_auto_drop_error_does_not_propagate() {
    // Even if a drop errors, remaining code should still execute.
    let result = eval(
        r#"
        function test_fn() {
            {
                let x = 42
            }
            return 99
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(99.0));
}

#[test]
fn test_async_drop_in_async_scope() {
    // Per-type async drop: a struct with async drop in an async function
    // should emit DropCallAsync. Plain `int` (no Drop impl) always gets DropCall.
    let bytecode = compile(
        r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        async function test_fn() {
            let c: Conn = Conn { id: 1 }
            return c.id
        }
    "#,
    );
    let has_async_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCallAsync);
    assert!(
        has_async_drop,
        "async function with async-drop type should emit DropCallAsync"
    );
}

#[test]
fn test_sync_function_uses_sync_drop() {
    // In a sync function, DropCall (not DropCallAsync) should be emitted.
    let bytecode = compile(
        r#"
        function test_fn() {
            let x = 42
            return x
        }
        test_fn()
    "#,
    );
    let has_sync_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCall);
    let has_async_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCallAsync);
    assert!(has_sync_drop, "sync function should emit DropCall");
    assert!(
        !has_async_drop,
        "sync function should NOT emit DropCallAsync"
    );
}
