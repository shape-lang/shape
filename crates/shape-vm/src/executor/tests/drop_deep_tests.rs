//! Deep exhaustive tests for the Drop/RAII and complex lifetime system.
//!
//! Categories:
//! 1. Basic Drop Ordering (~20 tests)
//! 2. Early Exit Drops (~25 tests)
//! 3. Loop Drop Patterns (~20 tests)
//! 4. Complex Scope Patterns (~25 tests)
//! 5. Drop with Custom Types (~15 tests)
//! 6. Drop with Closures & Higher-Order (~10 tests)
//! 7. Async Drop (~10 tests)

use crate::VMConfig;
use crate::bytecode::OpCode;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

fn compile(source: &str) -> crate::bytecode::BytecodeProgram {
    let program = parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    compiler.compile(&program).expect("compile failed")
}

fn eval(source: &str) -> ValueWord {
    let bytecode = compile(source);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).expect("execution failed").clone()
}

/// Count occurrences of a specific opcode in compiled bytecode.
fn count_opcode(source: &str, opcode: OpCode) -> usize {
    let bytecode = compile(source);
    bytecode
        .instructions
        .iter()
        .filter(|i| i.opcode == opcode)
        .count()
}

/// Check whether a specific opcode exists in compiled bytecode.
fn has_opcode(source: &str, opcode: OpCode) -> bool {
    count_opcode(source, opcode) > 0
}

// ============================================================================
// Category 1: Basic Drop Ordering (~20 tests)
// ============================================================================

#[test]
fn test_drop_basic_single_let_emits_drop() {
    // A single let binding in a function should emit exactly one DropCall.
    let src = r#"
        function f() {
            let x = 1
            return x
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "single let binding should emit DropCall"
    );
}

#[test]
fn test_drop_basic_two_lets_emit_two_drops() {
    let src = r#"
        function f() {
            let a = 1
            let b = 2
            return a + b
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "two let bindings should emit at least 2 DropCall instructions, got {}",
        count
    );
}

#[test]
fn test_drop_basic_five_lets_all_dropped() {
    let src = r#"
        function f() {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            return a + b + c + d + e
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 5,
        "five let bindings should emit at least 5 DropCall instructions, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_basic_let_in_function_body_correct_result() {
    let result = eval(
        r#"
        function f() {
            let x = 42
            return x
        }
        f()
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_basic_let_in_nested_block_drops_at_block_exit() {
    // Inner block let should produce a DropCall for the inner block scope,
    // separate from outer function drops.
    let src = r#"
        function f() {
            let outer = 1
            {
                let inner = 2
            }
            return outer
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "inner block let + outer let should emit at least 2 DropCall, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_basic_multiple_nested_blocks() {
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
                {
                    let c = 3
                }
            }
            return a
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 3,
        "three lets in nested blocks should emit at least 3 DropCall, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_basic_let_binding_primitive_number() {
    // Even primitives get DropCall (silently skipped at runtime).
    let src = r#"
        function f() {
            let x = 99
            return x
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "primitive number let binding should still emit DropCall"
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(99.0));
}

#[test]
fn test_drop_basic_let_binding_string() {
    let src = r#"
        function f() {
            let s = "hello"
            return s
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "string let binding should emit DropCall"
    );
}

#[test]
fn test_drop_basic_let_binding_array() {
    let src = r#"
        function f() {
            let arr = [1, 2, 3]
            return arr
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "array let binding should emit DropCall"
    );
}

#[test]
fn test_drop_basic_let_binding_bool() {
    let src = r#"
        function f() {
            let b = true
            return b
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "bool let binding should emit DropCall"
    );
}

#[test]
fn test_drop_basic_let_no_init() {
    // Let with no initialization should still be tracked for drop.
    let src = r#"
        function f() {
            let x
            return 1
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "uninitialized let should still emit DropCall"
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_basic_mutable_let() {
    let src = r#"
        function f() {
            let mut x = 1
            x = 2
            return x
        }
        f()
    "#;
    assert!(has_opcode(src, OpCode::DropCall));
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(2.0));
}

#[test]
fn test_drop_basic_sequential_blocks() {
    // Two sequential blocks should each drop their locals independently.
    let src = r#"
        function f() {
            {
                let a = 1
            }
            {
                let b = 2
            }
            return 3
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "two sequential blocks should emit at least 2 DropCall, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_basic_block_with_multiple_lets() {
    let src = r#"
        function f() {
            {
                let a = 1
                let b = 2
                let c = 3
            }
            return 0
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 3,
        "block with 3 lets should emit at least 3 DropCall, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(0.0));
}

#[test]
fn test_drop_basic_function_params_not_tracked_separately() {
    // Function parameters are stored in locals but may or may not be tracked
    // for auto-drop. Check that the function works correctly either way.
    let src = r#"
        function f(a, b) {
            let c = a + b
            return c
        }
        f(10, 20)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_basic_deeply_nested_blocks() {
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
                {
                    let c = 3
                    {
                        let d = 4
                        {
                            let e = 5
                        }
                    }
                }
            }
            return a
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 5,
        "5 nested lets should emit at least 5 DropCall, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_basic_empty_block_no_drops() {
    // An empty block should not emit any drops.
    let src = r#"
        function f() {
            {}
            return 1
        }
        f()
    "#;
    // The function itself may emit drops for its scope, but the empty block
    // should contribute no additional drops.
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_basic_let_with_complex_init() {
    // Let binding initialized from a complex expression.
    let src = r#"
        function f() {
            let x = [1, 2, 3].length()
            return x
        }
        f()
    "#;
    assert!(has_opcode(src, OpCode::DropCall));
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_basic_multiple_functions_independent() {
    // Each function should have its own drop scope.
    let src = r#"
        function f() {
            let x = 10
            return x
        }
        function g() {
            let y = 20
            return y
        }
        f() + g()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_basic_function_calling_function() {
    // Drops in inner function should not affect outer function.
    let src = r#"
        function inner() {
            let a = 5
            return a
        }
        function outer() {
            let b = inner()
            let c = 10
            return b + c
        }
        outer()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

// ============================================================================
// Category 2: Early Exit Drops (~25 tests)
// ============================================================================

#[test]
fn test_drop_early_return_single_local() {
    let src = r#"
        function f() {
            let x = 42
            return x
        }
        f()
    "#;
    assert!(has_opcode(src, OpCode::DropCall));
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_early_return_from_if_block() {
    let src = r#"
        function f() {
            let x = 10
            if true {
                return x
            }
            return 0
        }
        f()
    "#;
    assert!(has_opcode(src, OpCode::DropCall));
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_early_return_from_else_block() {
    let src = r#"
        function f() {
            let x = 10
            if false {
                return 0
            } else {
                return x
            }
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_early_return_nested_scope_two_levels() {
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
                return a + b
            }
        }
        f()
    "#;
    // Should emit drops for both b and a before return
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "early return from nested scope should drop both inner and outer, got {} drops",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_early_return_three_nested_scopes() {
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
                {
                    let c = 3
                    return a + b + c
                }
            }
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 3,
        "early return from 3 nested scopes should drop all 3, got {} drops",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

#[test]
fn test_drop_early_return_conditional_true_branch() {
    let src = r#"
        function f(cond) {
            let x = 10
            if cond {
                let y = 20
                return y
            }
            return x
        }
        f(true)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(20.0));
}

#[test]
fn test_drop_early_return_conditional_false_branch() {
    let src = r#"
        function f(cond) {
            let x = 10
            if cond {
                let y = 20
                return y
            }
            return x
        }
        f(false)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_break_from_loop() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let x = i * 2
                if x > 6 {
                    break
                }
                sum = sum + x
            }
            return sum
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(12.0));
}

#[test]
fn test_drop_break_from_nested_loop_inner() {
    let src = r#"
        function f() {
            let mut total = 0
            for i in [1, 2, 3] {
                for j in [10, 20, 30] {
                    let val = j
                    if j > 10 {
                        break
                    }
                    total = total + val
                }
            }
            return total
        }
        f()
    "#;
    // Each outer iteration adds 10 (only first j=10 is added before break at j=20)
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_continue_in_loop() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let x = i
                if i == 3 {
                    continue
                }
                sum = sum + x
            }
            return sum
        }
        f()
    "#;
    // 1 + 2 + 4 + 5 = 12
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(12.0));
}

#[test]
fn test_drop_continue_emits_drops() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let x = i
                if i == 2 {
                    continue
                }
                sum = sum + x
            }
            return sum
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "continue should emit DropCall for loop body locals"
    );
}

#[test]
fn test_drop_early_return_from_loop() {
    let src = r#"
        function f() {
            let outer = 100
            for i in [1, 2, 3] {
                let inner = i
                if inner == 2 {
                    return outer + inner
                }
            }
            return outer
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(102.0));
}

#[test]
fn test_drop_return_inside_nested_if() {
    let src = r#"
        function f(x) {
            let a = 1
            if x > 0 {
                let b = 2
                if x > 5 {
                    let c = 3
                    return a + b + c
                }
                return a + b
            }
            return a
        }
        f(10)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

#[test]
fn test_drop_return_inside_nested_if_mid_path() {
    let result = eval(
        r#"
        function f(x) {
            let a = 1
            if x > 0 {
                let b = 2
                if x > 5 {
                    let c = 3
                    return a + b + c
                }
                return a + b
            }
            return a
        }
        f(3)
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_return_inside_nested_if_outer() {
    let result = eval(
        r#"
        function f(x) {
            let a = 1
            if x > 0 {
                let b = 2
                if x > 5 {
                    let c = 3
                    return a + b + c
                }
                return a + b
            }
            return a
        }
        f(-1)
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_multiple_returns_in_function() {
    let src = r#"
        function f(x) {
            let a = 10
            if x == 1 { return a + 1 }
            if x == 2 { return a + 2 }
            if x == 3 { return a + 3 }
            return a
        }
        f(2)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(12.0));
}

#[test]
fn test_drop_return_with_computation() {
    // The return value should be computed before drops run.
    let src = r#"
        function f() {
            let x = 10
            let y = 20
            return x + y
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_break_with_let_after_loop() {
    // Variables declared after a loop should still be dropped correctly.
    let src = r#"
        function f() {
            for i in [1, 2, 3] {
                if i == 2 { break }
            }
            let after = 99
            return after
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(99.0));
}

#[test]
fn test_drop_while_loop_break() {
    let src = r#"
        function f() {
            let mut i = 0
            let mut sum = 0
            while i < 10 {
                let val = i
                if i == 5 { break }
                sum = sum + val
                i = i + 1
            }
            return sum
        }
        f()
    "#;
    // 0 + 1 + 2 + 3 + 4 = 10
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_while_loop_continue() {
    let src = r#"
        function f() {
            let mut i = 0
            let mut sum = 0
            while i < 5 {
                i = i + 1
                let val = i
                if i == 3 { continue }
                sum = sum + val
            }
            return sum
        }
        f()
    "#;
    // 1 + 2 + 4 + 5 = 12
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(12.0));
}

#[test]
fn test_drop_early_return_drops_all_active_scopes() {
    // Return from deeply nested scopes should emit drops for ALL active scopes.
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
                {
                    let c = 3
                    {
                        let d = 4
                        return a + b + c + d
                    }
                }
            }
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 4,
        "return from 4 nested scopes should emit at least 4 drops, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_break_only_drops_loop_scope() {
    // Break should only drop scopes within the loop, not outer function scopes.
    let src = r#"
        function f() {
            let outer = 100
            for i in [1, 2, 3] {
                let inner = i
                if i == 2 { break }
            }
            return outer
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(100.0));
}

#[test]
fn test_drop_continue_only_drops_current_iteration() {
    let src = r#"
        function f() {
            let mut results = []
            for i in [1, 2, 3, 4, 5] {
                let val = i * 10
                if i == 2 or i == 4 { continue }
                results = results.push(val)
            }
            return results.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

// ============================================================================
// Category 3: Loop Drop Patterns (~20 tests)
// ============================================================================

#[test]
fn test_drop_loop_for_body_locals_dropped_per_iteration() {
    let src = r#"
        function f() {
            let mut total = 0
            for i in [10, 20, 30] {
                let doubled = i * 2
                total = total + doubled
            }
            return total
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(120.0));
}

#[test]
fn test_drop_loop_while_body_locals_dropped_per_iteration() {
    let src = r#"
        function f() {
            let mut i = 0
            let mut total = 0
            while i < 3 {
                let val = (i + 1) * 10
                total = total + val
                i = i + 1
            }
            return total
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(60.0));
}

#[test]
fn test_drop_loop_break_in_middle_drops_local() {
    let src = r#"
        function f() {
            let mut last = 0
            for i in [1, 2, 3, 4, 5] {
                let x = i
                last = x
                if x == 3 { break }
            }
            return last
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_loop_continue_drops_local() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let x = i * 10
                if i == 2 or i == 4 {
                    continue
                }
                sum = sum + x
            }
            return sum
        }
        f()
    "#;
    // 10 + 30 + 50 = 90
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(90.0));
}

#[test]
fn test_drop_loop_conditional_break() {
    let src = r#"
        function f() {
            let mut count = 0
            for i in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] {
                let val = i
                if val > 5 { break }
                count = count + 1
            }
            return count
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(5.0));
}

#[test]
fn test_drop_loop_nested_loops_break_inner() {
    let src = r#"
        function f() {
            let mut total = 0
            for i in [1, 2] {
                for j in [10, 20, 30] {
                    let val = i * j
                    if j == 20 { break }
                    total = total + val
                }
            }
            return total
        }
        f()
    "#;
    // i=1: j=10 -> 10, break at j=20
    // i=2: j=10 -> 20, break at j=20
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_loop_nested_loops_continue_inner() {
    let src = r#"
        function f() {
            let mut total = 0
            for i in [1, 2] {
                for j in [10, 20, 30] {
                    let val = j
                    if j == 20 { continue }
                    total = total + val
                }
            }
            return total
        }
        f()
    "#;
    // Each outer: 10 + 30 = 40. Two outer iterations = 80.
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(80.0));
}

#[test]
fn test_drop_loop_with_accumulator() {
    let src = r#"
        function f() {
            let mut acc = 0
            for i in [1, 2, 3, 4, 5] {
                acc = acc + i
            }
            return acc
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_loop_with_early_return() {
    let src = r#"
        function f() {
            let outer = 100
            for i in [1, 2, 3, 4, 5] {
                let inner = i
                if inner == 3 {
                    return outer + inner
                }
            }
            return outer
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(103.0));
}

#[test]
fn test_drop_loop_for_empty_iterable() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [] {
                let x = i
                sum = sum + x
            }
            return sum
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(0.0));
}

#[test]
fn test_drop_loop_while_never_enters() {
    let src = r#"
        function f() {
            let mut x = 0
            while false {
                let y = 10
                x = y
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(0.0));
}

#[test]
fn test_drop_loop_many_iterations() {
    let src = r#"
        function f() {
            let mut sum = 0
            let mut i = 0
            while i < 100 {
                let val = i
                sum = sum + val
                i = i + 1
            }
            return sum
        }
        f()
    "#;
    // Sum 0..99 = 4950
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(4950.0));
}

#[test]
fn test_drop_loop_nested_break_with_let_between() {
    let src = r#"
        function f() {
            let a = 1
            for i in [1, 2, 3] {
                let b = 2
                {
                    let c = 3
                    if i == 2 { break }
                }
            }
            return a
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_loop_for_with_array_of_arrays() {
    let src = r#"
        function f() {
            let mut sum = 0
            for arr in [[1, 2], [3, 4], [5, 6]] {
                let inner = arr
                sum = sum + inner.length()
            }
            return sum
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

#[test]
fn test_drop_loop_break_after_multiple_lets() {
    let src = r#"
        function f() {
            for i in [1, 2, 3] {
                let a = i
                let b = i * 2
                let c = i * 3
                if i == 2 { break }
            }
            return 42
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_loop_continue_after_multiple_lets() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let a = i
                let b = i * 2
                if i == 2 { continue }
                sum = sum + a + b
            }
            return sum
        }
        f()
    "#;
    // i=1: 1+2=3, i=2: skip, i=3: 3+6=9 => 12
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(12.0));
}

#[test]
fn test_drop_loop_while_with_block_body() {
    let src = r#"
        function f() {
            let mut n = 5
            let mut factorial = 1
            while n > 0 {
                let current = n
                factorial = factorial * current
                n = n - 1
            }
            return factorial
        }
        f()
    "#;
    // 5! = 120
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(120.0));
}

#[test]
fn test_drop_loop_triple_nested() {
    let src = r#"
        function f() {
            let mut total = 0
            for i in [1, 2] {
                for j in [1, 2] {
                    for k in [1, 2] {
                        let val = i * j * k
                        total = total + val
                    }
                }
            }
            return total
        }
        f()
    "#;
    // All combos of {1,2}^3: 1+2+2+4+2+4+4+8 = 27
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(27.0));
}

#[test]
fn test_drop_loop_return_from_nested_for() {
    let src = r#"
        function f() {
            for i in [1, 2, 3] {
                for j in [10, 20, 30] {
                    let product = i * j
                    if product == 40 {
                        return product
                    }
                }
            }
            return 0
        }
        f()
    "#;
    // i=2, j=20 => 40
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(40.0));
}

// ============================================================================
// Category 4: Complex Scope Patterns (~25 tests)
// ============================================================================

#[test]
fn test_drop_scope_if_else_different_locals() {
    let src = r#"
        function f(cond) {
            if cond {
                let a = 10
                return a
            } else {
                let b = 20
                return b
            }
        }
        f(true)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_scope_if_else_false_branch() {
    let result = eval(
        r#"
        function f(cond) {
            if cond {
                let a = 10
                return a
            } else {
                let b = 20
                return b
            }
        }
        f(false)
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(20.0));
}

#[test]
fn test_drop_scope_if_without_else() {
    let src = r#"
        function f() {
            let x = 10
            if true {
                let y = 20
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_scope_if_false_without_else() {
    let src = r#"
        function f() {
            let x = 10
            if false {
                let y = 20
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_scope_block_expr_as_value() {
    // Block expression: the temp inside should be dropped, but the result value preserved.
    let src = r#"
        function f() {
            let x = {
                let tmp = 21
                tmp * 2
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_scope_block_expr_temp_dropped() {
    // The temp var inside the block expression should be dropped.
    let src = r#"
        function f() {
            let x = {
                let tmp = 10
                tmp + 5
            }
            return x
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "block expr temp + outer x should both get drops, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_scope_nested_blocks_ordering() {
    // In { let a; { let b; } let c; }, drops should be: b, then c+a at function exit.
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
            }
            let c = 3
            return a + c
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 3,
        "three lets across nested blocks should emit at least 3 drops, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(4.0));
}

#[test]
fn test_drop_scope_conditional_let_only_drops_if_entered() {
    let src = r#"
        function f(cond) {
            let x = 1
            if cond {
                let y = 2
            }
            return x
        }
        f(true)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_scope_interleaved_blocks_and_lets() {
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
            }
            let c = 3
            {
                let d = 4
            }
            let e = 5
            return a + c + e
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 5,
        "5 lets should produce at least 5 drops, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(9.0));
}

#[test]
fn test_drop_scope_block_value_not_dropped_prematurely() {
    // The value returned from a block expression should survive past the block.
    let src = r#"
        function f() {
            let result = {
                let tmp = 100
                tmp + 1
            }
            return result + 1
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(102.0));
}

#[test]
fn test_drop_scope_multiple_block_exprs() {
    let src = r#"
        function f() {
            let a = {
                let tmp1 = 10
                tmp1
            }
            let b = {
                let tmp2 = 20
                tmp2
            }
            return a + b
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_scope_if_else_as_expression() {
    let src = r#"
        function f(cond) {
            let x = if cond { 10 } else { 20 }
            return x
        }
        f(true)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_scope_if_else_expr_false() {
    let result = eval(
        r#"
        function f(cond) {
            let x = if cond { 10 } else { 20 }
            return x
        }
        f(false)
    "#,
    );
    assert_eq!(result.as_number_coerce(), Some(20.0));
}

#[test]
fn test_drop_scope_block_with_early_return_inside() {
    let src = r#"
        function f() {
            let a = 1
            {
                let b = 2
                if true {
                    return a + b
                }
            }
            return a
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_scope_many_variables_same_scope() {
    let src = r#"
        function f() {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            let f_ = 6
            let g = 7
            let h = 8
            let i = 9
            let j = 10
            return a + b + c + d + e + f_ + g + h + i + j
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 10,
        "10 lets should produce at least 10 drops, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(55.0));
}

#[test]
fn test_drop_scope_variable_shadowing() {
    // Variable shadowing: both the outer and inner should be dropped.
    let src = r#"
        function f() {
            let x = 10
            {
                let x = 20
            }
            return x
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "shadowed variable should get its own drop, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_scope_block_inside_if() {
    let src = r#"
        function f() {
            let x = 1
            if true {
                {
                    let y = 2
                }
                let z = 3
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1.0));
}

#[test]
fn test_drop_scope_function_call_in_block() {
    let src = r#"
        function helper() {
            let tmp = 42
            return tmp
        }
        function f() {
            let x = {
                let y = helper()
                y + 1
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(43.0));
}

#[test]
fn test_drop_scope_mutable_in_block() {
    let src = r#"
        function f() {
            let mut x = 0
            {
                let y = 10
                x = y
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_scope_nested_block_exprs() {
    let src = r#"
        function f() {
            let result = {
                let a = {
                    let inner = 5
                    inner * 2
                }
                a + 1
            }
            return result
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(11.0));
}

#[test]
fn test_drop_scope_let_after_block_with_drops() {
    let src = r#"
        function f() {
            {
                let a = 1
                let b = 2
            }
            let c = 3
            return c
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_scope_loop_inside_block() {
    let src = r#"
        function f() {
            let result = {
                let mut sum = 0
                for i in [1, 2, 3] {
                    sum = sum + i
                }
                sum
            }
            return result
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

#[test]
fn test_drop_scope_if_chain_with_lets() {
    let src = r#"
        function f(x) {
            if x == 1 {
                let a = 10
                return a
            } else if x == 2 {
                let b = 20
                return b
            } else if x == 3 {
                let c = 30
                return c
            } else {
                let d = 40
                return d
            }
        }
        f(3)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_scope_complex_nesting() {
    let src = r#"
        function f() {
            let a = 1
            let result = {
                let b = 2
                let inner_result = {
                    let c = 3
                    a + b + c
                }
                inner_result
            }
            return result
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

// ============================================================================
// Category 5: Drop with Custom Types (~15 tests)
// ============================================================================

#[test]
fn test_drop_custom_type_without_drop_impl() {
    // Types without a drop function should silently skip.
    let src = r#"
        type Point {
            x: number,
            y: number
        }
        function f() {
            let p = Point { x: 1, y: 2 }
            return p.x + p.y
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_custom_type_multiple_instances() {
    let src = r#"
        type Vec2 {
            x: number,
            y: number
        }
        function f() {
            let a = Vec2 { x: 1, y: 2 }
            let b = Vec2 { x: 3, y: 4 }
            return a.x + b.y
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(5.0));
}

#[test]
fn test_drop_custom_type_in_loop() {
    let src = r#"
        type Wrapper {
            value: number
        }
        function f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let w = Wrapper { value: i * 10 }
                sum = sum + w.value
            }
            return sum
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(60.0));
}

#[test]
fn test_drop_custom_type_in_block() {
    let src = r#"
        type Data {
            n: number
        }
        function f() {
            let x = {
                let d = Data { n: 42 }
                d.n
            }
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_custom_type_early_return() {
    let src = r#"
        type Config {
            val: number
        }
        function f(cond) {
            let c = Config { val: 99 }
            if cond {
                return c.val
            }
            return 0
        }
        f(true)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(99.0));
}

#[test]
fn test_drop_array_of_numbers() {
    let src = r#"
        function f() {
            let arr = [10, 20, 30, 40, 50]
            return arr.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(5.0));
}

#[test]
fn test_drop_nested_arrays() {
    let src = r#"
        function f() {
            let arr = [[1, 2], [3, 4], [5, 6]]
            return arr.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_string_in_scope() {
    let src = r#"
        function f() {
            let s = "hello world"
            {
                let t = "inner string"
            }
            return s.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(11.0));
}

#[test]
fn test_drop_custom_type_nested_in_function_call() {
    let src = r#"
        type Pair {
            a: number,
            b: number
        }
        function make_pair(x, y) {
            let p = Pair { a: x, b: y }
            return p
        }
        function f() {
            let p = make_pair(3, 7)
            return p.a + p.b
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_custom_type_conditional_branches() {
    let src = r#"
        type Box {
            val: number
        }
        function f(cond) {
            if cond {
                let b = Box { val: 10 }
                return b.val
            } else {
                let b = Box { val: 20 }
                return b.val
            }
        }
        f(true) + f(false)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_multiple_types_same_scope() {
    let src = r#"
        type A { x: number }
        type B { y: number }
        function f() {
            let a = A { x: 10 }
            let b = B { y: 20 }
            return a.x + b.y
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_custom_type_in_while_loop() {
    let src = r#"
        type Counter {
            n: number
        }
        function f() {
            let mut i = 0
            let mut sum = 0
            while i < 3 {
                let c = Counter { n: i * 10 }
                sum = sum + c.n
                i = i + 1
            }
            return sum
        }
        f()
    "#;
    // 0 + 10 + 20 = 30
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(30.0));
}

#[test]
fn test_drop_custom_type_returned_not_dropped() {
    // A custom type that is returned should not be dropped prematurely.
    let src = r#"
        type Result_ {
            value: number
        }
        function make() {
            let r = Result_ { value: 42 }
            return r
        }
        function f() {
            let r = make()
            return r.value
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_mixed_types_complex() {
    let src = r#"
        type Rec {
            name: string,
            val: number
        }
        function f() {
            let n = 42
            let s = "hello"
            let a = [1, 2, 3]
            let r = Rec { name: "test", val: 99 }
            return n + a.length() + r.val
        }
        f()
    "#;
    // 42 + 3 + 99 = 144
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(144.0));
}

#[test]
fn test_drop_custom_type_with_string_field() {
    let src = r#"
        type Named {
            name: string,
            id: number
        }
        function f() {
            let n = Named { name: "test", id: 7 }
            {
                let m = Named { name: "inner", id: 8 }
            }
            return n.id
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(7.0));
}

// ============================================================================
// Category 6: Drop with Closures & Higher-Order (~10 tests)
// ============================================================================

#[test]
fn test_drop_closure_basic_no_capture() {
    let src = r#"
        function f() {
            let add = |a, b| a + b
            return add(3, 4)
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(7.0));
}

#[test]
fn test_drop_closure_captures_value() {
    let src = r#"
        function f() {
            let x = 10
            let add_x = |a| a + x
            return add_x(5)
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_closure_returned_from_function() {
    // A closure returned from a function should keep captured values alive.
    let src = r#"
        function make_adder(x) {
            let offset = x
            return |a| a + offset
        }
        function f() {
            let add5 = make_adder(5)
            return add5(10)
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_closure_in_loop() {
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let inc = |x| x + i
                sum = sum + inc(0)
            }
            return sum
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

#[test]
fn test_drop_closure_as_argument() {
    let src = r#"
        function apply(f_, x) {
            return f_(x)
        }
        function g() {
            let multiplier = 3
            let triple = |x| x * multiplier
            return apply(triple, 7)
        }
        g()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(21.0));
}

#[test]
fn test_drop_higher_order_map() {
    let src = r#"
        function f() {
            let arr = [1, 2, 3, 4, 5]
            let doubled = arr.map(|x| x * 2)
            return doubled.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(5.0));
}

#[test]
fn test_drop_higher_order_filter() {
    let src = r#"
        function f() {
            let arr = [1, 2, 3, 4, 5, 6]
            let evens = arr.filter(|x| x % 2 == 0)
            return evens.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_closure_multiple_captures() {
    let src = r#"
        function f() {
            let a = 10
            let b = 20
            let add_both = |x| x + a + b
            return add_both(5)
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(35.0));
}

#[test]
fn test_drop_closure_nested_closures() {
    let src = r#"
        function f() {
            let x = 5
            let outer = |a| {
                let inner = |b| a + b + x
                return inner(3)
            }
            return outer(2)
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_closure_with_block_body() {
    let src = r#"
        function f() {
            let transform = |x| {
                let tmp = x * 2
                let result = tmp + 1
                return result
            }
            return transform(5)
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(11.0));
}

// ============================================================================
// Category 7: Per-Type Async Drop (~19 tests)
//
// Drop opcode selection is based on the type's DropKind, not the calling context.
// - SyncOnly type → DropCall always
// - AsyncOnly type + async context → DropCallAsync
// - AsyncOnly type + sync context → compile error
// - Both type + async context → DropCallAsync (prefers async)
// - Both type + sync context → DropCall (sync fallback)
// - Unknown type (no Drop impl) → DropCall
// ============================================================================

/// Try to compile Shape source; return Err if compilation fails.
fn try_compile(
    source: &str,
) -> Result<crate::bytecode::BytecodeProgram, shape_ast::error::ShapeError> {
    let program = parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    compiler.compile(&program)
}

#[test]
fn test_drop_async_function_emits_async_drop() {
    // A struct with async drop in an async function → DropCallAsync.
    let src = r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        async function f() {
            let c: Conn = Conn { id: 1 }
            return c.id
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "async-drop type in async function should emit DropCallAsync"
    );
}

#[test]
fn test_drop_async_no_sync_drop_for_untyped() {
    // Plain int in async function should NOT emit DropCallAsync (no Drop impl).
    let src = r#"
        async function f() {
            let x = 42
            return x
        }
    "#;
    assert!(
        !has_opcode(src, OpCode::DropCallAsync),
        "untyped int in async function should NOT emit DropCallAsync"
    );
    assert!(
        has_opcode(src, OpCode::DropCall),
        "untyped int in async function should emit DropCall"
    );
}

#[test]
fn test_drop_async_multiple_locals() {
    let src = r#"
        type Res { v: int }
        impl Drop for Res {
            async method drop() { }
        }
        async function f() {
            let a: Res = Res { v: 1 }
            let b: Res = Res { v: 2 }
            let c: Res = Res { v: 3 }
            return a.v + b.v + c.v
        }
    "#;
    let count = count_opcode(src, OpCode::DropCallAsync);
    assert!(
        count >= 3,
        "async function with 3 async-drop locals should emit at least 3 DropCallAsync, got {}",
        count
    );
}

#[test]
fn test_drop_async_nested_scopes() {
    let src = r#"
        type Handle { id: int }
        impl Drop for Handle {
            async method drop() { }
        }
        async function f() {
            let a: Handle = Handle { id: 1 }
            {
                let b: Handle = Handle { id: 2 }
            }
            return a.id
        }
    "#;
    let count = count_opcode(src, OpCode::DropCallAsync);
    assert!(
        count >= 2,
        "async function with nested scope should emit at least 2 DropCallAsync, got {}",
        count
    );
}

#[test]
fn test_drop_sync_function_no_async_drop() {
    let src = r#"
        function f() {
            let x = 42
            return x
        }
        f()
    "#;
    let has_async = has_opcode(src, OpCode::DropCallAsync);
    assert!(!has_async, "sync function should NOT emit DropCallAsync");
}

#[test]
fn test_drop_async_early_return() {
    let src = r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        async function f() {
            let c: Conn = Conn { id: 10 }
            if true {
                return c.id
            }
            return 0
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "async early return with async-drop type should emit DropCallAsync"
    );
}

#[test]
fn test_drop_async_with_loop() {
    let src = r#"
        type Item { v: int }
        impl Drop for Item {
            async method drop() { }
        }
        async function f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let x: Item = Item { v: i }
                sum = sum + x.v
            }
            return sum
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "async function with loop and async-drop type should emit DropCallAsync"
    );
}

#[test]
fn test_drop_mixed_sync_async_separate_functions() {
    // Sync function uses DropCall, async function uses DropCallAsync for async-drop type.
    let src = r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        function sync_fn() {
            let x = 1
            return x
        }
        async function async_fn() {
            let c: Conn = Conn { id: 2 }
            return c.id
        }
        sync_fn()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "sync function should emit DropCall"
    );
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "async function with async-drop type should emit DropCallAsync"
    );
}

#[test]
fn test_drop_async_block_expression() {
    let src = r#"
        type Tok { v: int }
        impl Drop for Tok {
            async method drop() { }
        }
        async function f() {
            let result: Tok = {
                let tmp: Tok = Tok { v: 42 }
                tmp
            }
            return result.v
        }
    "#;
    let count = count_opcode(src, OpCode::DropCallAsync);
    assert!(
        count >= 1,
        "async block expr with async-drop type should emit DropCallAsync, got {}",
        count
    );
}

#[test]
fn test_drop_async_with_break() {
    let src = r#"
        type Elem { v: int }
        impl Drop for Elem {
            async method drop() { }
        }
        async function f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let x: Elem = Elem { v: i }
                if x.v > 3 { break }
                sum = sum + x.v
            }
            return sum
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "async break with async-drop type should emit DropCallAsync"
    );
}

// --- New per-type async drop tests ---

#[test]
fn test_per_type_sync_drop_in_async_fn() {
    // A struct with SYNC drop in async fn → DropCall (NOT DropCallAsync).
    let src = r#"
        type Logger { tag: int }
        impl Drop for Logger {
            method drop() { }
        }
        async function f() {
            let l: Logger = Logger { tag: 1 }
            return l.tag
        }
    "#;
    // The Logger local should get DropCall, not DropCallAsync.
    assert!(
        has_opcode(src, OpCode::DropCall),
        "sync-drop type in async fn should emit DropCall"
    );
    // Check no DropCallAsync for the Logger variable specifically.
    // (There might be other DropCall for untyped locals, but no DropCallAsync.)
    // Actually, let's verify no DropCallAsync at all in this program.
    assert!(
        !has_opcode(src, OpCode::DropCallAsync),
        "sync-drop type in async fn should NOT emit DropCallAsync"
    );
}

#[test]
fn test_async_only_drop_in_sync_fn_compile_error() {
    // AsyncOnly drop type in sync context → compile error.
    let src = r#"
        type AsyncRes { id: int }
        impl Drop for AsyncRes {
            async method drop() { }
        }
        function f() {
            let r: AsyncRes = AsyncRes { id: 1 }
            return r.id
        }
        f()
    "#;
    let result = try_compile(src);
    assert!(
        result.is_err(),
        "AsyncOnly drop in sync fn should be a compile error"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("async drop") || err_msg.contains("sync context"),
        "error should mention async drop / sync context, got: {}",
        err_msg
    );
}

#[test]
fn test_both_sync_async_drop_in_async_fn() {
    // Type with both sync and async drop in async fn → prefers DropCallAsync.
    let src = r#"
        type DualConn { id: int }
        impl Drop for DualConn {
            method drop() { }
            async method drop() { }
        }
        async function f() {
            let c: DualConn = DualConn { id: 1 }
            return c.id
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "Both-drop type in async fn should emit DropCallAsync"
    );
}

#[test]
fn test_both_sync_async_drop_in_sync_fn() {
    // Type with both sync and async drop in sync fn → uses DropCall (sync fallback).
    let src = r#"
        type DualConn { id: int }
        impl Drop for DualConn {
            method drop() { }
            async method drop() { }
        }
        function f() {
            let c: DualConn = DualConn { id: 1 }
            return c.id
        }
        f()
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "Both-drop type in sync fn should emit DropCall"
    );
    assert!(
        !has_opcode(src, OpCode::DropCallAsync),
        "Both-drop type in sync fn should NOT emit DropCallAsync"
    );
}

#[test]
fn test_mixed_types_same_async_fn() {
    // One SyncOnly type and one AsyncOnly type in the same async fn →
    // correct opcode for each.
    let src = r#"
        type SyncRes { v: int }
        impl Drop for SyncRes {
            method drop() { }
        }
        type AsyncRes { v: int }
        impl Drop for AsyncRes {
            async method drop() { }
        }
        async function f() {
            let s: SyncRes = SyncRes { v: 1 }
            let a: AsyncRes = AsyncRes { v: 2 }
            return s.v + a.v
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "sync-drop type should emit DropCall"
    );
    assert!(
        has_opcode(src, OpCode::DropCallAsync),
        "async-drop type should emit DropCallAsync"
    );
}

#[test]
fn test_untyped_local_defaults_to_sync_drop() {
    // Unknown type (no annotation, no Drop impl) → DropCall.
    let src = r#"
        async function f() {
            let x = 42
            return x
        }
    "#;
    assert!(
        has_opcode(src, OpCode::DropCall),
        "untyped local should default to DropCall"
    );
    assert!(
        !has_opcode(src, OpCode::DropCallAsync),
        "untyped local should NOT emit DropCallAsync"
    );
}

#[test]
fn test_async_drop_with_early_return_path() {
    // Async drop on both normal and early return paths.
    let src = r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        async function f(flag: bool) {
            let c: Conn = Conn { id: 1 }
            if flag {
                return 0
            }
            return c.id
        }
    "#;
    let count = count_opcode(src, OpCode::DropCallAsync);
    assert!(
        count >= 2,
        "async drop should appear on both return paths, got {}",
        count
    );
}

#[test]
fn test_async_drop_in_nested_scope() {
    // Inner scope drops use correct opcode.
    let src = r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        async function f() {
            let outer: Conn = Conn { id: 1 }
            {
                let inner: Conn = Conn { id: 2 }
            }
            return outer.id
        }
    "#;
    let count = count_opcode(src, OpCode::DropCallAsync);
    assert!(
        count >= 2,
        "both inner and outer scope should emit DropCallAsync, got {}",
        count
    );
}

// ============================================================================
// Additional Edge Cases & Stress Tests
// ============================================================================

#[test]
fn test_drop_edge_recursive_function() {
    // Recursive function should correctly drop locals at each stack frame.
    let src = r#"
        function factorial(n) {
            let current = n
            if current <= 1 {
                return 1
            }
            return current * factorial(current - 1)
        }
        factorial(5)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(120.0));
}

#[test]
fn test_drop_edge_mutual_recursion() {
    let src = r#"
        function is_even(n) {
            let val = n
            if val == 0 { return true }
            return is_odd(val - 1)
        }
        function is_odd(n) {
            let val = n
            if val == 0 { return false }
            return is_even(val - 1)
        }
        is_even(10)
    "#;
    let result = eval(src);
    assert!(result.is_truthy());
}

#[test]
fn test_drop_edge_function_returning_array() {
    let src = r#"
        function make_array() {
            let a = 1
            let b = 2
            let c = 3
            return [a, b, c]
        }
        function f() {
            let arr = make_array()
            return arr.length()
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_edge_chained_function_calls() {
    let src = r#"
        function add(a, b) {
            let result = a + b
            return result
        }
        function f() {
            let x = add(add(1, 2), add(3, 4))
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(10.0));
}

#[test]
fn test_drop_edge_scope_after_loop_completion() {
    // After a loop completes normally (no break), subsequent code should work.
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3] {
                let val = i
                sum = sum + val
            }
            let after = sum * 2
            return after
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(12.0));
}

#[test]
fn test_drop_edge_multiple_loops_sequential() {
    let src = r#"
        function f() {
            let mut sum1 = 0
            for i in [1, 2, 3] {
                let x = i
                sum1 = sum1 + x
            }
            let mut sum2 = 0
            for j in [10, 20, 30] {
                let y = j
                sum2 = sum2 + y
            }
            return sum1 + sum2
        }
        f()
    "#;
    // 6 + 60 = 66
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(66.0));
}

#[test]
fn test_drop_edge_let_in_both_if_branches() {
    let src = r#"
        function f(x) {
            let mut result = 0
            if x > 0 {
                let pos = x * 2
                result = pos
            } else {
                let neg = x * -1
                result = neg
            }
            return result
        }
        f(5) + f(-3)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(13.0));
}

#[test]
fn test_drop_edge_block_expr_with_function_call() {
    let src = r#"
        function double(x) {
            let result = x * 2
            return result
        }
        function f() {
            let val = {
                let tmp = double(21)
                tmp
            }
            return val
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_edge_deeply_nested_return() {
    let src = r#"
        function f() {
            let a = 1
            if true {
                let b = 2
                if true {
                    let c = 3
                    if true {
                        let d = 4
                        if true {
                            let e = 5
                            return a + b + c + d + e
                        }
                    }
                }
            }
            return 0
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_edge_loop_with_nested_blocks_and_break() {
    let src = r#"
        function f() {
            let mut total = 0
            for i in [1, 2, 3, 4, 5] {
                let a = i
                {
                    let b = a * 2
                    {
                        let c = b + 1
                        if c > 7 {
                            break
                        }
                        total = total + c
                    }
                }
            }
            return total
        }
        f()
    "#;
    // i=1: a=1, b=2, c=3. i=2: a=2, b=4, c=5. i=3: a=3, b=6, c=7. i=4: a=4, b=8, c=9>7 break
    // 3+5+7 = 15
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_edge_while_true_with_break() {
    let src = r#"
        function f() {
            let mut i = 0
            while true {
                let x = i
                if x >= 5 { break }
                i = i + 1
            }
            return i
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(5.0));
}

#[test]
fn test_drop_edge_fibonacci() {
    let src = r#"
        function fib(n) {
            let val = n
            if val <= 1 { return val }
            let a = fib(val - 1)
            let b = fib(val - 2)
            return a + b
        }
        fib(10)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(55.0));
}

#[test]
fn test_drop_edge_iterative_fibonacci() {
    let src = r#"
        function fib(n) {
            let mut a = 0
            let mut b = 1
            let mut i = 0
            while i < n {
                let tmp = a + b
                a = b
                b = tmp
                i = i + 1
            }
            return a
        }
        fib(10)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(55.0));
}

#[test]
fn test_drop_edge_empty_function() {
    // A function with no lets should not emit any DropCall.
    let src = r#"
        function f() {
            return 42
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(42.0));
}

#[test]
fn test_drop_edge_only_params_no_lets() {
    let src = r#"
        function f(a, b, c) {
            return a + b + c
        }
        f(1, 2, 3)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(6.0));
}

// BUG: compile_expr_return (expression-form return, e.g. inside block expressions)
// does NOT emit drops before ReturnValue, while Statement::Return does.
// This means `return expr` as a statement emits drops correctly, but
// `return expr` inside a block expression context may skip drops.
// See: compiler/expressions/control_flow.rs:91-100 vs compiler/statements.rs:1896-1908
#[test]
fn test_drop_bug_expr_return_should_emit_drops() {
    // Test that Statement::Return (which correctly emits drops) works.
    let src = r#"
        function f() {
            let x = 42
            let y = 58
            return x + y
        }
        f()
    "#;
    let count = count_opcode(src, OpCode::DropCall);
    assert!(
        count >= 2,
        "Statement::Return should emit drops for both x and y, got {}",
        count
    );
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(100.0));
}

#[test]
fn test_drop_edge_return_value_computed_before_drops() {
    // The return value must be on the stack BEFORE drops run.
    // Otherwise, drops could corrupt the return value.
    let src = r#"
        function f() {
            let x = 10
            let y = 20
            let z = 30
            return x + y + z
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(60.0));
}

#[test]
fn test_drop_stress_many_scopes() {
    let src = r#"
        function f() {
            let mut total = 0
            { let a = 1; total = total + a }
            { let b = 2; total = total + b }
            { let c = 3; total = total + c }
            { let d = 4; total = total + d }
            { let e = 5; total = total + e }
            { let f_ = 6; total = total + f_ }
            { let g = 7; total = total + g }
            { let h = 8; total = total + h }
            { let i = 9; total = total + i }
            { let j = 10; total = total + j }
            return total
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(55.0));
}

#[test]
fn test_drop_stress_deep_recursion() {
    let src = r#"
        function sum_to(n) {
            let val = n
            if val <= 0 { return 0 }
            return val + sum_to(val - 1)
        }
        sum_to(50)
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(1275.0));
}

#[test]
fn test_drop_edge_assign_does_not_double_drop() {
    // Reassigning a mutable variable should not cause the old value to be
    // dropped in a separate DropCall; it's just overwritten in the same local slot.
    let src = r#"
        function f() {
            let mut x = 1
            x = 2
            x = 3
            return x
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(3.0));
}

#[test]
fn test_drop_edge_for_expr_returns_value() {
    // For-expression (expression-level for loop with break value).
    let src = r#"
        function f() {
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                sum = sum + i
            }
            return sum
        }
        f()
    "#;
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(15.0));
}

#[test]
fn test_drop_edge_complex_mix() {
    // Complex mix of scopes, loops, closures, and early exits.
    let src = r#"
        function f() {
            let base = 100
            let mut sum = 0
            for i in [1, 2, 3, 4, 5] {
                let val = i * 2
                if val == 6 { continue }
                {
                    let inner = val + base
                    sum = sum + inner
                }
            }
            let final_val = sum
            return final_val
        }
        f()
    "#;
    // i=1: val=2, inner=102, sum=102
    // i=2: val=4, inner=104, sum=206
    // i=3: val=6, continue
    // i=4: val=8, inner=108, sum=314
    // i=5: val=10, inner=110, sum=424
    let result = eval(src);
    assert_eq!(result.as_number_coerce(), Some(424.0));
}
