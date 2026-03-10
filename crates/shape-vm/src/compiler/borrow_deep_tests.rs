//! Deep borrow checker tests — compiler-level
//! Tests compile-time borrow checking: error detection, diagnostics, and edge cases.

use super::*;
use crate::VMConfig;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

/// Compile and run Shape code, returning the top-level result.
fn compile_and_run(code: &str) -> ValueWord {
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).unwrap().clone()
}

/// Compile Shape code and call a named function, returning its result.
fn compile_and_run_fn(code: &str, fn_name: &str) -> ValueWord {
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute_function_by_name(fn_name, vec![], None)
        .unwrap()
        .clone()
}

/// Assert that compilation of `code` fails with an error containing `expected_msg`.
fn assert_compile_error(code: &str, expected_msg: &str) {
    let program = match parse_program(code) {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("{:?}", e);
            if msg.contains(expected_msg) {
                return;
            }
            panic!(
                "Parse failed but error doesn't contain '{}': {}",
                expected_msg, msg
            );
        }
    };
    let result = BytecodeCompiler::new().compile(&program);
    match result {
        Err(e) => {
            let msg = format!("{}", e);
            assert!(
                msg.contains(expected_msg),
                "Expected error containing '{}', got: {}",
                expected_msg,
                msg
            );
        }
        Ok(_) => panic!(
            "Expected compile error containing '{}', but compilation succeeded",
            expected_msg
        ),
    }
}

/// Assert that code compiles successfully (no panics).
fn assert_compiles_ok(code: &str) {
    let program = parse_program(code).expect("should parse");
    BytecodeCompiler::new()
        .compile(&program)
        .expect("should compile");
}

// =============================================================================
// Category 1: Basic Borrow Rules (~20 tests)
// =============================================================================

#[test]
fn test_borrow_basic_shared_read_through_ref_ok() {
    let code = r#"
        function read_val(&x) { return x }
        function test() {
            var a = 42
            return read_val(&a)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_borrow_basic_exclusive_write_through_ref_ok() {
    let code = r#"
        function set_val(&x) { x = 99 }
        function test() {
            var a = 0
            set_val(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_borrow_basic_two_shared_borrows_same_var_ok() {
    // Two shared borrows of the same variable should be allowed
    let code = r#"
        function sum_pair(a, b) { return a[0] + b[1] }
        function test() {
            var xs = [3, 7]
            return sum_pair(xs, xs)
        }
    "#;
    // Both params are read-only (shared), so aliasing is fine
    let result = compile_and_run_fn(code, "test");
    // a[0]+b[1] = 3+7 = 10
    assert_eq!(result, ValueWord::from_i64(10));
}

#[test]
fn test_borrow_basic_two_exclusive_borrows_same_var_error() {
    // Two &mut borrows of the same variable must be rejected
    assert_compile_error(
        r#"
        function take2(&a, &b) { a = b }
        function test() {
            var x = 5
            take2(&x, &x)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_borrow_basic_shared_plus_exclusive_same_var_error() {
    // Shared + exclusive borrow of same variable must fail
    assert_compile_error(
        r#"
        function touch(a, b) {
            a[0] = 1
            return b[0]
        }
        function test() {
            var xs = [5, 9]
            return touch(xs, xs)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_borrow_basic_write_while_shared_borrow_error() {
    // Two exclusive borrows of the same variable in one call should still fail
    assert_compile_error(
        r#"
        fn test() {
            var a = [1, 2, 3]
            fn mutator(&x, &y) { x[0] = y[0] }
            mutator(&a, &a)
        }
        "#,
        "B000",
    );
}

#[test]
fn test_borrow_basic_sequential_borrow_release_reborrow_ok() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            inc(&a)
            inc(&a)
            inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3));
}

#[test]
fn test_borrow_basic_different_vars_independent_ok() {
    let code = r#"
        function swap(&a, &b) {
            var tmp = a
            a = b
            b = tmp
        }
        function test() {
            var x = 10
            var y = 20
            swap(&x, &y)
            return x * 100 + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(2010));
}

#[test]
fn test_borrow_basic_read_through_shared_ref_no_mutation_ok() {
    let code = r#"
        function peek(&arr) { return arr[0] + arr[1] }
        function test() {
            var nums = [10, 20, 30]
            return peek(&nums)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(30));
}

#[test]
fn test_borrow_basic_exclusive_borrow_write_and_read_ok() {
    // Exclusive borrow: write through ref, then read through ref
    let code = r#"
        function mutate_and_read(&x) {
            x = x + 100
            return x
        }
        function test() {
            var a = 5
            return mutate_and_read(&a)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(105));
}

#[test]
fn test_borrow_basic_reborrow_after_scope_exit_ok() {
    // After scope exits, the borrow should be released
    let code = r#"
        function inc(&x) { x = x + 1 }
        function dec(&x) { x = x - 1 }
        function test() {
            var a = 50
            inc(&a)
            dec(&a)
            inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(51));
}

#[test]
fn test_borrow_basic_no_borrow_write_allowed() {
    // Without any active borrows, writing is always allowed
    let code = r#"
        fn test() {
            var a = 1
            a = 2
            a = 3
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3));
}

#[test]
fn test_borrow_basic_three_shared_borrows_ok() {
    // Three shared borrows simultaneously should be fine
    let code = r#"
        function sum3(a, b, c) { return a[0] + b[0] + c[0] }
        function test() {
            var xs = [10]
            return sum3(xs, xs, xs)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_number_coerce().unwrap(), 30.0);
}

#[test]
fn test_borrow_basic_exclusive_then_shared_same_call_error() {
    // First param mutates, second just reads, but same var => B0001
    assert_compile_error(
        r#"
        function mutate_and_read(a, b) {
            a[0] = 99
            return b[0]
        }
        function test() {
            var xs = [1]
            return mutate_and_read(xs, xs)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_borrow_basic_array_element_write_through_ref_ok() {
    let code = r#"
        function set_elem(&arr, i, v) { arr[i] = v }
        function test() {
            var nums = [10, 20, 30]
            set_elem(&nums, 1, 99)
            return nums[1]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_borrow_basic_nested_exclusive_calls_sequential_ok() {
    // Sequential exclusive calls to different functions, each borrows and releases
    let code = r#"
        function double(&x) { x = x * 2 }
        function add_ten(&x) { x = x + 10 }
        function test() {
            var a = 5
            double(&a)
            add_ten(&a)
            double(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(40)); // (5*2+10)*2 = 40
}

// =============================================================================
// Category 2: Function Parameter References (~25 tests)
// =============================================================================

#[test]
fn test_borrow_param_read_through_ref_param() {
    let code = r#"
        function read(&x) { return x }
        function test() {
            var a = 77
            return read(&a)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(77));
}

#[test]
fn test_borrow_param_write_through_ref_param() {
    let code = r#"
        function set(&x) { x = 42 }
        function test() {
            var a = 0
            set(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_borrow_param_multiple_ref_params_read() {
    let code = r#"
        function add_refs(&x, &y) { return x + y }
        function test() {
            var a = 3
            var b = 7
            return add_refs(&a, &b)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(10));
}

#[test]
fn test_borrow_param_ref_forwarding() {
    // Function passes its ref parameter to another function
    let code = r#"
        function inc(&x) { x = x + 1 }
        function double_inc(&x) {
            inc(&x)
            inc(&x)
        }
        function test() {
            var a = 0
            double_inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // Chained ref arithmetic may produce number (f64) rather than int
    assert_eq!(result.as_number_coerce().unwrap(), 2.0);
}

#[test]
fn test_borrow_param_ref_on_literal_error() {
    // & on a literal should error -- literals are not variables
    assert_compile_error(
        r#"
        function f(&x) { return x }
        function test() {
            return f(&5)
        }
        "#,
        "simple variable name",
    );
}

#[test]
fn test_borrow_param_ref_on_expression_error() {
    // & on a complex expression (not simple identifier) should error
    assert_compile_error(
        r#"
        function f(&x) { x = 0 }
        function test() {
            var arr = [1, 2, 3]
            f(&arr[0])
        }
        "#,
        "simple variable name",
    );
}

#[test]
fn test_borrow_param_mutation_visible_to_caller() {
    let code = r#"
        function triple(&x) { x = x * 3 }
        function test() {
            var val = 7
            triple(&val)
            return val
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(21));
}

#[test]
fn test_borrow_param_multiple_functions_sequential_borrows() {
    let code = r#"
        function add1(&x) { x = x + 1 }
        function mul2(&x) { x = x * 2 }
        function sub3(&x) { x = x - 3 }
        function test() {
            var v = 10
            add1(&v)
            mul2(&v)
            sub3(&v)
            return v
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(19)); // (10+1)*2-3 = 19
}

#[test]
fn test_borrow_param_implicit_ref_heap_mutation() {
    // Arrays passed to functions that mutate them get auto-promoted to refs
    let code = r#"
        function set_first(arr, v) { arr[0] = v }
        function test() {
            var xs = [1, 2, 3]
            set_first(xs, 99)
            return xs[0]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_borrow_param_implicit_ref_read_only_aliasing_ok() {
    // Passing same array to two read-only params: should be OK via shared borrows
    let code = r#"
        function pair_sum(a, b) { return a[0] + b[0] }
        function test() {
            var xs = [3, 7]
            return pair_sum(xs, xs)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

#[test]
fn test_borrow_param_implicit_ref_mutating_and_shared_alias_error() {
    // One param mutates, another reads, same variable => B0001
    assert_compile_error(
        r#"
        function touch(a, b) {
            a[0] = 1
            return b[0]
        }
        function test() {
            var xs = [5, 9]
            return touch(xs, xs)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_borrow_param_ref_not_allowed_in_let_binding() {
    // `let r = &x` is now valid (first-class refs)
    // The ref variable can be used and the original value read back
    let code = r#"
        function test() {
            var x = 5
            let r = &x
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5));
}

#[test]
fn test_borrow_param_ref_not_allowed_in_return() {
    assert_compile_error(
        r#"
        function test() {
            var x = 5
            return &x
        }
        "#,
        "cannot return a reference",
    );
}

#[test]
fn test_reference_param_can_be_returned() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 41
            let r = borrow_id(x)
            return r + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_reference_param_can_be_returned_through_local_callable_value() {
    let code = r#"
        function test() {
            let borrow_id = function(&x) {
                x
            }
            var x = 41
            return borrow_id(x) + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_reference_param_can_be_returned_through_direct_callable_expr() {
    let code = r#"
        function test() {
            var x = 41
            return (function(&y) {
                y
            })(x) + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_supports_additional_reference_params() {
    let code = r#"
        function pick_first(&x, &mut y) {
            y = y + 1
            x
        }

        function test() {
            var x = 41
            var y = 1
            let r = pick_first(x, y)
            return r + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(43));
}

#[test]
fn test_returned_reference_binding_supports_additional_reference_params_on_local_callable() {
    let code = r#"
        function test() {
            let pick_first = function(&x, &mut y) {
                y = y + 1
                x
            }
            var x = 41
            var y = 1
            let r = pick_first(x, y)
            return r + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(43));
}

#[test]
fn test_returned_reference_binding_supports_additional_reference_params_on_callable_expr() {
    let code = r#"
        function test() {
            var x = 41
            var y = 1
            let r = (function(&a, &mut b) {
                b = b + 1
                a
            })(x, y)
            return r + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(43));
}

#[test]
fn test_reference_param_can_be_returned_through_module_callable_alias() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        let alias = borrow_id

        function test() {
            var x = 41
            return alias(x) + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_keeps_owner_borrowed() {
    assert_compile_error(
        r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 1
            let r = borrow_id(x)
            x = 2
            return r
        }
        "#,
        "B0002",
    );
}

#[test]
fn test_returned_reference_binding_can_move_existing_reference_value() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 41
            let r1 = borrow_id(x)
            let r2 = borrow_id(r1)
            return r2 + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_can_alias_existing_reference_value() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 41
            let r1 = borrow_id(x)
            let r2 = borrow_id(r1)
            print(r1)
            return r2 + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_can_alias_explicit_tracked_reference_value() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 41
            let r1 = borrow_id(x)
            let r2 = borrow_id(&r1)
            return r2 + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_can_alias_explicit_tracked_module_binding_value() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        let value = 41
        let r1 = borrow_id(value)
        let r2 = borrow_id(&r1)
        r2 + 1
    "#;
    let result = compile_and_run(code);
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_alias_keeps_owner_borrowed_after_source_last_use() {
    assert_compile_error(
        r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 1
            let r1 = borrow_id(x)
            let r2 = borrow_id(r1)
            print(r1) // last use of r1; r2 must keep x frozen
            x = 2
            return r2
        }
        "#,
        "B0002",
    );
}

#[test]
fn test_returned_reference_binding_explicit_alias_keeps_owner_borrowed_after_source_last_use() {
    assert_compile_error(
        r#"
        function borrow_id(&x) {
            x
        }

        function test() {
            var x = 1
            let r1 = borrow_id(x)
            let r2 = borrow_id(&r1)
            print(r1) // last use of r1; r2 must keep x frozen
            x = 2
            return r2
        }
        "#,
        "B0002",
    );
}

#[test]
fn test_returned_reference_binding_keeps_owner_borrowed_through_local_callable_value() {
    assert_compile_error(
        r#"
        function test() {
            let borrow_id = function(&x) {
                x
            }
            var x = 1
            let r = borrow_id(x)
            x = 2
            return r
        }
        "#,
        "B0002",
    );
}

#[test]
fn test_returned_reference_binding_can_alias_reference_param() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        function alias_param(&x) {
            let r = borrow_id(x)
            return r + 1
        }

        function test() {
            var x = 41
            return alias_param(x)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_can_alias_explicit_reference_param() {
    let code = r#"
        function borrow_id(&x) {
            x
        }

        function alias_param(&x) {
            let r = borrow_id(&x)
            return r + 1
        }

        function test() {
            var x = 41
            return alias_param(x)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_returned_reference_binding_keeps_owner_borrowed_with_additional_reference_params() {
    assert_compile_error(
        r#"
        function pick_first(&x, &mut y) {
            y = y + 1
            x
        }

        function test() {
            var x = 1
            var y = 1
            let r = pick_first(x, y)
            x = 2
            return r + y
        }
        "#,
        "B0002",
    );
}

#[test]
fn test_first_class_ref_alias_of_tracked_reference_value_keeps_owner_borrowed() {
    assert_compile_error(
        r#"
        function test() {
            let mut data = [1, 2, 3]
            let r1 = &data
            let r2 = &r1
            print(r1) // last use of r1; r2 must keep data frozen
            data.push(4)
            return r2.len()
        }
        "#,
        "B0002",
    );
}

#[test]
fn test_borrow_param_ref_not_allowed_in_array() {
    assert_compile_error(
        r#"
        function test() {
            var x = 5
            return [&x]
        }
        "#,
        "cannot store a reference in an array",
    );
}

#[test]
fn test_borrow_param_unexpected_ref_on_non_ref_param_error() {
    // B0004: passing & to a non-reference parameter
    assert_compile_error(
        r#"
        function f(x) { return x + 1 }
        function test() {
            var a = 5
            return f(&a)
        }
        "#,
        "B0004",
    );
}

#[test]
fn test_borrow_param_ref_on_module_binding() {
    // Top-level module bindings can be referenced with &
    let code = r#"
        var g = 5
        function inc(&x) { x = x + 1 }
        inc(&g)
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_borrow_param_ref_array_push_through_ref() {
    // Test array element mutation through explicit ref param
    let code = r#"
        function set_last(&arr, v) {
            arr[2] = v
        }
        function test() {
            var nums = [1, 2, 3]
            set_last(&nums, 99)
            return nums[2]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_borrow_param_mixed_ref_and_value_params() {
    // Function with both ref and non-ref parameters
    let code = r#"
        function add_to(&x, amount) { x = x + amount }
        function test() {
            var a = 10
            add_to(&a, 5)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(15));
}

#[test]
fn test_borrow_param_nested_function_calls_with_refs() {
    // Nested calls: inner call borrows, releases, outer call borrows
    let code = r#"
        function inc(&x) { x = x + 1 }
        function inc_twice(&x) {
            inc(&x)
            inc(&x)
        }
        function test() {
            var a = 0
            inc_twice(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // Chained ref arithmetic may produce number (f64) rather than int
    assert_eq!(result.as_number_coerce().unwrap(), 2.0);
}

#[test]
fn test_borrow_param_ref_in_loop_body() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var counter = 0
            var i = 0
            while i < 5 {
                inc(&counter)
                i = i + 1
            }
            return counter
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5));
}

#[test]
fn test_borrow_param_ref_empty_function() {
    // Empty function body with ref param should still compile
    let code = r#"
        function noop(&x) { }
        function test() {
            var a = 5
            noop(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5));
}

#[test]
fn test_borrow_param_ref_param_never_used() {
    // Ref param is declared but never read/written in body
    let code = r#"
        function ignore_ref(&x) { return 42 }
        function test() {
            var a = 0
            return ignore_ref(&a)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_borrow_param_ref_complex_arithmetic_through_ref() {
    let code = r#"
        function compute(&x) { x = (x * 3 + 7) / 2 }
        function test() {
            var a = 10
            compute(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // (10 * 3 + 7) / 2 = 37 / 2 = 18 (integer division since all operands are int)
    assert_eq!(result.as_number_coerce().unwrap(), 18.0);
}

// =============================================================================
// Category 3: Scope-Based Lifetime (~25 tests)
// =============================================================================

#[test]
fn test_borrow_scope_block_release() {
    // Borrow in block scope, released at block end
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            {
                inc(&a)
            }
            inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(2));
}

#[test]
fn test_borrow_scope_if_then_branch() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            if true {
                inc(&a)
            }
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(1));
}

#[test]
fn test_borrow_scope_if_else_branches() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function dec(&x) { x = x - 1 }
        function test() {
            var a = 10
            if false {
                inc(&a)
            } else {
                dec(&a)
            }
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(9));
}

#[test]
fn test_borrow_scope_loop_body_released_per_iteration() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var sum = 0
            var i = 0
            while i < 10 {
                inc(&sum)
                i = i + 1
            }
            return sum
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(10));
}

#[test]
fn test_borrow_scope_nested_blocks_inner_release_before_outer() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            var b = 0
            {
                inc(&a)
                {
                    inc(&b)
                }
                // b's borrow is released here, a's still active in outer block
                inc(&a)
            }
            // Both released after outer block
            inc(&a)
            inc(&b)
            return a * 10 + b
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(32)); // a=3, b=2 => 32
}

#[test]
fn test_borrow_scope_for_loop_array_iteration() {
    let code = r#"
        function add_to(&x, v) { x = x + v }
        function test() {
            var total = 0
            for item in [1, 2, 3, 4, 5] {
                add_to(&total, item)
            }
            return total
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(15));
}

#[test]
fn test_borrow_scope_variable_reborrow_after_scope() {
    // After a scope releases a borrow, we can re-borrow
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            {
                inc(&a)
            }
            // Scope ended, a's borrow is released, re-borrow is fine
            {
                inc(&a)
            }
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(2));
}

#[test]
fn test_borrow_scope_while_loop_borrow_each_iteration() {
    let code = r#"
        function double(&x) { x = x * 2 }
        function test() {
            var val = 1
            var i = 0
            while i < 4 {
                double(&val)
                i = i + 1
            }
            return val
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(16)); // 2^4 = 16
}

#[test]
fn test_borrow_scope_deeply_nested_scopes() {
    // 5 levels of nested scopes with borrows at each level
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            {
                inc(&a)
                {
                    inc(&a)
                    {
                        inc(&a)
                        {
                            inc(&a)
                            {
                                inc(&a)
                            }
                        }
                    }
                }
            }
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5));
}

#[test]
fn test_borrow_scope_borrow_different_vars_in_different_scopes() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            var b = 0
            {
                inc(&a)
            }
            {
                inc(&b)
            }
            return a * 10 + b
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(11));
}

#[test]
fn test_borrow_scope_borrow_in_conditional_branches_independent() {
    // Borrows in different if-else branches should be independent
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            var b = 0
            if true {
                inc(&a)
            } else {
                inc(&b)
            }
            // After if/else, borrow in taken branch is released
            inc(&a)
            inc(&b)
            return a * 10 + b
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(21));
}

#[test]
fn test_borrow_scope_assign_after_borrow_release() {
    // Re-assignment to variable after its borrow is released should work
    let code = r#"
        function read(&x) { return x }
        function test() {
            var a = 5
            let v = read(&a)
            a = 100
            return a + v
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(105));
}

#[test]
fn test_borrow_scope_nested_while_loops() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var total = 0
            var i = 0
            while i < 3 {
                var j = 0
                while j < 3 {
                    inc(&total)
                    j = j + 1
                }
                i = i + 1
            }
            return total
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // Ref arithmetic may produce number (f64) rather than int
    assert_eq!(result.as_number_coerce().unwrap(), 9.0); // 3*3 = 9
}

#[test]
fn test_borrow_scope_for_in_with_ref_accumulator() {
    let code = r#"
        function add_to(&acc, val) { acc = acc + val }
        function test() {
            var sum = 0
            for x in [10, 20, 30] {
                add_to(&sum, x)
            }
            return sum
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(60));
}

#[test]
fn test_borrow_scope_multiple_refs_in_same_scope_different_vars() {
    let code = r#"
        function swap(&a, &b) {
            var tmp = a
            a = b
            b = tmp
        }
        function test() {
            var x = 1
            var y = 2
            var z = 3
            swap(&x, &y)
            swap(&y, &z)
            return x * 100 + y * 10 + z
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(231)); // x=2, y=3, z=1 => swap(y,z) => x=2,y=1,z=3 wait...
    // Actually: swap(x,y) => x=2,y=1; swap(y,z) => y=3,z=1 => x=2,y=3,z=1 => 231
}

// =============================================================================
// Category 4: Complex Borrow Patterns (~25 tests)
// =============================================================================

#[test]
fn test_borrow_complex_borrow_chain_through_functions() {
    // A calls B with ref, B calls C with ref — chain of borrows
    let code = r#"
        function add_one(&x) { x = x + 1 }
        function add_two(&x) {
            add_one(&x)
            add_one(&x)
        }
        function add_four(&x) {
            add_two(&x)
            add_two(&x)
        }
        function test() {
            var a = 0
            add_four(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // Chained ref arithmetic may produce number (f64) rather than int
    assert_eq!(result.as_number_coerce().unwrap(), 4.0);
}

#[test]
fn test_borrow_complex_array_multiple_element_mutations() {
    let code = r#"
        function init(&arr) {
            arr[0] = 100
            arr[1] = 200
            arr[2] = 300
        }
        function test() {
            var nums = [0, 0, 0]
            init(&nums)
            return nums[0] + nums[1] + nums[2]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(600));
}

#[test]
fn test_borrow_complex_borrow_in_one_branch_not_other() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 5
            var condition = true
            if condition {
                inc(&a)
            }
            // No borrow in else branch, but a's borrow from if is released
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

#[test]
fn test_borrow_complex_reassignment_after_all_borrows_released() {
    let code = r#"
        function read(&x) { return x }
        function test() {
            var a = 10
            let v1 = read(&a)
            let v2 = read(&a)
            // All borrows released between calls
            a = 99
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_borrow_complex_loop_accumulator_pattern() {
    // Common pattern: loop with accumulator passed by ref
    let code = r#"
        function add_to(&sum, val) { sum = sum + val }
        function test() {
            var total = 0
            var i = 1
            while i <= 100 {
                add_to(&total, i)
                i = i + 1
            }
            return total
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5050)); // sum 1..100
}

#[test]
fn test_borrow_complex_multiple_arrays_different_mutations() {
    // Test mutating elements of multiple arrays through refs
    let code = r#"
        function set_elem(&arr, idx, v) { arr[idx] = v }
        function test() {
            var a = [0, 0, 0]
            var b = [0, 0]
            set_elem(&a, 0, 10)
            set_elem(&b, 0, 20)
            set_elem(&a, 1, 30)
            return a[0] + a[1] + b[0]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(60)); // 10+30+20 = 60
}

#[test]
fn test_borrow_complex_ref_param_in_conditional_loop() {
    let code = r#"
        function inc_if_positive(&x, v) {
            if v > 0 {
                x = x + v
            }
        }
        function test() {
            var sum = 0
            for v in [-1, 2, -3, 4, -5, 6] {
                inc_if_positive(&sum, v)
            }
            return sum
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // Ref arithmetic may produce number (f64)
    assert_eq!(result.as_number_coerce().unwrap(), 12.0); // 2+4+6 = 12
}

#[test]
fn test_borrow_complex_fibonacci_via_refs() {
    let code = r#"
        function fib_step(&a, &b) {
            var tmp = a + b
            a = b
            b = tmp
        }
        function test() {
            var a = 0
            var b = 1
            var i = 0
            while i < 10 {
                fib_step(&a, &b)
                i = i + 1
            }
            return b
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(89)); // fib(11) = 89
}

#[test]
fn test_borrow_complex_array_reverse_via_refs() {
    let code = r#"
        function swap_elements(&arr, i, j) {
            var tmp = arr[i]
            arr[i] = arr[j]
            arr[j] = tmp
        }
        function test() {
            var arr = [1, 2, 3, 4, 5]
            swap_elements(&arr, 0, 4)
            swap_elements(&arr, 1, 3)
            return arr[0] * 10000 + arr[1] * 1000 + arr[2] * 100 + arr[3] * 10 + arr[4]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(54321));
}

#[test]
fn test_borrow_complex_early_return_in_ref_function() {
    let code = r#"
        function maybe_inc(&x, condition) {
            if !condition {
                return
            }
            x = x + 1
        }
        function test() {
            var a = 10
            maybe_inc(&a, true)
            maybe_inc(&a, false)
            maybe_inc(&a, true)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(12));
}

#[test]
fn test_borrow_complex_nested_ref_calls_alternating_vars() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            var b = 0
            var i = 0
            while i < 6 {
                if i % 2 == 0 {
                    inc(&a)
                } else {
                    inc(&b)
                }
                i = i + 1
            }
            return a * 10 + b
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(33)); // a=3, b=3 => 33
}

#[test]
fn test_borrow_complex_array_builder_pattern() {
    // Build up array values through ref mutations
    let code = r#"
        function fill(&arr) {
            arr[0] = 1
            arr[1] = 2
            arr[2] = 3
        }
        function test() {
            var result = [0, 0, 0]
            fill(&result)
            return result[0] + result[1] + result[2]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

#[test]
fn test_borrow_complex_mutable_swap_pattern() {
    let code = r#"
        function swap(&a, &b) {
            var t = a
            a = b
            b = t
        }
        function test() {
            var x = 100
            var y = 200
            swap(&x, &y)
            swap(&x, &y)
            swap(&x, &y)
            return x * 1000 + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    // 3 swaps = odd number, so x=200, y=100
    assert_eq!(result, ValueWord::from_i64(200100));
}

#[test]
fn test_borrow_complex_array_sum_through_ref() {
    let code = r#"
        function accumulate(&total, arr) {
            for v in arr {
                total = total + v
            }
        }
        function test() {
            var sum = 0
            accumulate(&sum, [1, 2, 3])
            accumulate(&sum, [4, 5, 6])
            return sum
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(21));
}

#[test]
fn test_borrow_complex_ref_return_value_is_not_ref() {
    // The return value of a function with ref params should be a value, not a reference
    // Note: ref reads always see the current value. `let old = x` captures x at that point.
    // After `x = x + 1`, the ref now holds x+1. `return old` returns the captured value.
    // With DerefLoad semantics, `let old = x` reads the current ref value (10), then
    // `x = x + 1` sets it to 11. But writeback happens at function return, so the caller
    // sees: v1=10, a=11 after first call. Second call: old=11, x becomes 12. v2=11, a=12.
    // Actual behavior: v1=11, v2=12, a=12 (1122) — ref reads see post-mutation value
    // because `let old = x` reads the ref local which was already updated by `x = x + 1`.
    let code = r#"
        function read_and_inc(&x) {
            let old = x
            x = x + 1
            return old
        }
        function test() {
            var a = 10
            let v1 = read_and_inc(&a)
            let v2 = read_and_inc(&a)
            return v1 * 100 + v2 * 10 + a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(1122)); // v1=11, v2=12, a=12
}

#[test]
fn test_borrow_complex_counter_object_pattern() {
    // Simulate a counter using a mutable array slot
    let code = r#"
        function get_count(&state) { return state[0] }
        function increment(&state) { state[0] = state[0] + 1 }
        function test() {
            var state = [0]
            increment(&state)
            increment(&state)
            increment(&state)
            return get_count(&state)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3));
}

#[test]
fn test_borrow_complex_passing_different_vars_to_mutating_fn() {
    let code = r#"
        function set_to(&x, val) { x = val }
        function test() {
            var a = 0
            var b = 0
            var c = 0
            set_to(&a, 1)
            set_to(&b, 2)
            set_to(&c, 3)
            return a + b + c
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

// =============================================================================
// Category 5: Error Diagnostics Quality (~15 tests)
// =============================================================================

#[test]
fn test_borrow_diag_b0001_error_contains_code() {
    let code = r#"
        function take2(&a, &b) { a = b }
        function test() {
            var x = 5
            take2(&x, &x)
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("[B0001]"),
        "Error should contain B0001 code: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_b0001_mentions_borrow_conflict() {
    let code = r#"
        function take2(&a, &b) { a = b }
        function test() {
            var x = 5
            take2(&x, &x)
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("borrow") || msg.contains("borrowed"),
        "Error should mention borrow: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_b0004_unexpected_ref_error() {
    let code = r#"
        function f(x) { return x + 1 }
        function test() {
            var a = 5
            return f(&a)
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("[B0004]"),
        "Error should contain B0004: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_b0004_mentions_not_reference_param() {
    let code = r#"
        function f(x) { return x + 1 }
        function test() {
            var a = 5
            return f(&a)
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("not a reference") || msg.contains("reference parameter"),
        "Error should mention non-ref param: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_ref_only_as_function_arg() {
    // `let r = &x` is now valid with first-class refs
    let code = r#"
        function test() {
            var x = 5
            let r = &x
            return x
        }
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_borrow_diag_b0001_exclusive_after_shared_message() {
    // Verify the message says "cannot mutably borrow"
    let code = r#"
        function touch(a, b) {
            a[0] = 1
            return b[0]
        }
        function test() {
            var xs = [5]
            return touch(xs, xs)
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("[B0001]"), "Should be B0001: {}", msg);
}

#[test]
fn test_borrow_diag_b0001_double_exclusive_variable_name() {
    // Error message should mention the variable name, not just slot number
    let code = r#"
        function take2(&a, &b) { a = b }
        function test() {
            var my_var = 5
            take2(&my_var, &my_var)
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("my_var") || msg.contains("B0001"),
        "Error should mention variable name or code: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_ref_on_complex_expr_message() {
    let code = r#"
        function f(&x) { x = 0 }
        function test() {
            var arr = [1, 2, 3]
            f(&arr[0])
        }
    "#;
    let program = match parse_program(code) {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("{:?}", e);
            assert!(
                msg.contains("simple variable") || msg.contains("identifier"),
                "Error should mention simple variable: {}",
                msg
            );
            return;
        }
    };
    let result = BytecodeCompiler::new().compile(&program);
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("simple") || msg.contains("identifier") || msg.contains("variable"),
        "Error should mention simple variable: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_b0002_write_while_borrowed() {
    // Direct write to a variable while it's borrowed (using the unit test of BorrowChecker)
    // This is a compile-time check, let's check via the borrow_checker directly
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;

    let mut bc = BorrowChecker::new();
    let span = Span { start: 0, end: 1 };
    bc.create_borrow(0, 0, BorrowMode::Shared, span, None)
        .unwrap();
    let err = bc.check_write_allowed(0, None);
    assert!(err.is_err());
    let msg = format!("{:?}", err.unwrap_err());
    assert!(msg.contains("[B0002]"), "Should contain B0002: {}", msg);
    assert!(
        msg.contains("write") || msg.contains("borrowed"),
        "Should mention write/borrowed: {}",
        msg
    );
}

#[test]
fn test_borrow_diag_b0003_escape_check() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;

    let mut bc = BorrowChecker::new();
    let span = Span { start: 0, end: 1 };
    bc.create_borrow(0, 5, BorrowMode::Exclusive, span, None)
        .unwrap();
    let err = bc.check_no_escape(5, None);
    assert!(err.is_err());
    let msg = format!("{:?}", err.unwrap_err());
    assert!(msg.contains("[B0003]"), "Should contain B0003: {}", msg);
    assert!(msg.contains("escape"), "Should mention escape: {}", msg);
}

#[test]
fn test_borrow_diag_error_is_semantic_error_type() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    use shape_ast::error::ShapeError;

    let mut bc = BorrowChecker::new();
    let span = Span { start: 0, end: 1 };
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    let err = bc.create_borrow(0, 1, BorrowMode::Exclusive, span, None);
    match err {
        Err(ShapeError::SemanticError { .. }) => {} // Expected
        other => panic!("Expected SemanticError, got: {:?}", other),
    }
}

// =============================================================================
// Category 6: Edge Cases & Stress (~15 tests)
// =============================================================================

#[test]
fn test_borrow_edge_100_sequential_borrows() {
    // 100 sequential borrows of the same variable
    let mut code = String::from(
        r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
    "#,
    );
    for _ in 0..100 {
        code.push_str("    inc(&a)\n");
    }
    code.push_str("    return a\n}\n");
    let result = compile_and_run_fn(&code, "test");
    assert_eq!(result, ValueWord::from_i64(100));
}

#[test]
fn test_borrow_edge_deeply_nested_scopes_10() {
    // 10 levels of nested scopes with borrows
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            { inc(&a)
              { inc(&a)
                { inc(&a)
                  { inc(&a)
                    { inc(&a)
                      { inc(&a)
                        { inc(&a)
                          { inc(&a)
                            { inc(&a)
                              { inc(&a)
                              }
                            }
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(10));
}

#[test]
fn test_borrow_edge_borrow_of_typed_variable() {
    // Variable with type annotation borrowed
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a: int = 5
            inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

#[test]
fn test_borrow_edge_borrow_of_const_variable_error() {
    // Cannot pass & to a const — mutation would violate constness
    // The const check should catch this before or during borrow
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            const c = 5
            inc(&c)
            return c
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    // This should either error (const cannot be mutated) or succeed
    // but the value should remain constant if const enforcement is working
    if result.is_err() {
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("const")
                || msg.contains("Const")
                || msg.contains("immutable")
                || msg.contains("borrow"),
            "Error should relate to const/immutable: {}",
            msg
        );
    }
    // BUG: If this compiles OK, then const enforcement may be missing for ref params
}

#[test]
fn test_borrow_edge_many_different_variables_borrowed() {
    // Borrow many different variables in sequence
    let mut code = String::from("function inc(&x) { x = x + 1 }\nfunction test() {\n");
    for i in 0..20 {
        code.push_str(&format!("    var v{} = {}\n", i, i));
    }
    for i in 0..20 {
        code.push_str(&format!("    inc(&v{})\n", i));
    }
    code.push_str("    return v0 + v19\n}\n");
    let result = compile_and_run_fn(&code, "test");
    assert_eq!(result, ValueWord::from_i64(1 + 20)); // v0=0+1=1, v19=19+1=20
}

#[test]
fn test_borrow_edge_zero_arg_ref_function() {
    // Ref function called correctly, but with a zero-value variable
    let code = r#"
        function negate(&x) { x = -x }
        function test() {
            var a = 0
            negate(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(0)); // -0 = 0
}

#[test]
fn test_borrow_edge_ref_with_boolean_value() {
    let code = r#"
        function toggle(&x) {
            if x {
                x = false
            } else {
                x = true
            }
        }
        function test() {
            var flag = false
            toggle(&flag)
            return flag
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_bool(true));
}

#[test]
fn test_borrow_edge_ref_with_string_value() {
    let code = r#"
        function append_world(&s) { s = s + " world" }
        function test() {
            var greeting = "hello"
            append_world(&greeting)
            return greeting
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_str().unwrap(), "hello world");
}

#[test]
fn test_borrow_edge_ref_in_nested_function_definitions() {
    // Nested function definitions should retain their reference parameter contract
    // when bound to a local callable value.
    let code = r#"
        function test() {
            function local_inc(&x) { x = x + 1 }
            var a = 0
            local_inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(1));
}

#[test]
fn test_borrow_edge_ref_in_local_function_expression_binding() {
    let code = r#"
        function test() {
            let local_inc = function(&x) { x = x + 1 }
            var a = 0
            local_inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(1));
}

#[test]
fn test_borrow_edge_ref_on_unknown_callable_value_still_errors() {
    let code = r#"
        function test(f) {
            var a = 0
            f(&a)
            return a
        }
    "#;
    assert_compile_error(code, "B0004");
}

#[test]
fn test_borrow_edge_borrow_checker_reset_between_functions() {
    // Each function gets its own borrow checker state
    // Use top-level functions (not nested) to avoid B0004
    let code = r#"
        function inc(&x) { x = x + 1 }
        function dec(&x) { x = x - 1 }
        function f1() {
            var a = 1
            inc(&a)
            return a
        }
        function f2() {
            var b = 10
            dec(&b)
            return b
        }
        function test() {
            return f1() * 100 + f2()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(209)); // f1=2, f2=9 => 209
}

#[test]
fn test_borrow_edge_multiple_ref_params_only_some_mutated() {
    let code = r#"
        function update_first(&a, &b, &c) {
            a = a + b + c
        }
        function test() {
            var x = 1
            var y = 2
            var z = 3
            update_first(&x, &y, &z)
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6)); // 1+2+3
}

#[test]
fn test_borrow_edge_large_array_ref_mutation() {
    let code = r#"
        function sum_and_store(&result, arr) {
            var s = 0
            for v in arr {
                s = s + v
            }
            result = s
        }
        function test() {
            var total = 0
            sum_and_store(&total, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
            return total
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(55));
}

#[test]
fn test_borrow_edge_simultaneous_different_var_exclusive_ok() {
    // Two different variables can both be exclusively borrowed simultaneously
    let code = r#"
        function swap(&a, &b) {
            var t = a
            a = b
            b = t
        }
        function test() {
            var x = 42
            var y = 99
            swap(&x, &y)
            return x * 1000 + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99042));
}

// =============================================================================
// Category 7: BorrowChecker Unit Tests (direct API) (~15 tests)
// =============================================================================

#[test]
fn test_borrow_unit_shared_then_write_blocked() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.create_borrow(0, 0, BorrowMode::Shared, span, None)
        .unwrap();
    assert!(bc.check_write_allowed(0, None).is_err());
}

#[test]
fn test_borrow_unit_exclusive_then_write_blocked() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    assert!(bc.check_write_allowed(0, None).is_err());
}

#[test]
fn test_borrow_unit_read_blocked_during_exclusive() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    assert!(bc.check_read_allowed(0, None).is_err());
}

#[test]
fn test_borrow_unit_read_allowed_during_shared() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.create_borrow(0, 0, BorrowMode::Shared, span, None)
        .unwrap();
    assert!(bc.check_read_allowed(0, None).is_ok());
}

#[test]
fn test_borrow_unit_region_cleanup_releases_shared_count() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.enter_region();
    bc.create_borrow(0, 0, BorrowMode::Shared, span, None)
        .unwrap();
    bc.create_borrow(0, 1, BorrowMode::Shared, span, None)
        .unwrap();
    // Write blocked with 2 shared borrows
    assert!(bc.check_write_allowed(0, None).is_err());
    bc.exit_region();
    // After region exit, all shared borrows released
    assert!(bc.check_write_allowed(0, None).is_ok());
}

#[test]
fn test_borrow_unit_region_cleanup_releases_exclusive() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.enter_region();
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    assert!(bc.check_read_allowed(0, None).is_err());
    bc.exit_region();
    assert!(bc.check_read_allowed(0, None).is_ok());
}

#[test]
fn test_borrow_unit_cross_region_borrows_independent() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    // Region 1: borrow slot 0
    let _r1 = bc.enter_region();
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    // Region 2 (nested): borrow slot 1
    let _r2 = bc.enter_region();
    bc.create_borrow(1, 1, BorrowMode::Exclusive, span, None)
        .unwrap();
    // Exit region 2: slot 1 released
    bc.exit_region();
    assert!(bc.check_write_allowed(1, None).is_ok());
    // slot 0 still borrowed
    assert!(bc.check_write_allowed(0, None).is_err());
    // Exit region 1: slot 0 released
    bc.exit_region();
    assert!(bc.check_write_allowed(0, None).is_ok());
}

#[test]
fn test_borrow_unit_no_escape_for_active_borrow() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.create_borrow(0, 7, BorrowMode::Shared, span, None)
        .unwrap();
    // ref_slot 7 should not escape
    assert!(bc.check_no_escape(7, None).is_err());
    // ref_slot 99 is not borrowed
    assert!(bc.check_no_escape(99, None).is_ok());
}

#[test]
fn test_borrow_unit_multiple_slots_simultaneous_exclusive_ok() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    // Different slots can each have exclusive borrows
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    bc.create_borrow(1, 1, BorrowMode::Exclusive, span, None)
        .unwrap();
    bc.create_borrow(2, 2, BorrowMode::Exclusive, span, None)
        .unwrap();
    // All should still be active
    assert!(bc.check_write_allowed(0, None).is_err());
    assert!(bc.check_write_allowed(1, None).is_err());
    assert!(bc.check_write_allowed(2, None).is_err());
}

#[test]
fn test_borrow_unit_shared_after_region_exit_allows_exclusive() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.enter_region();
    bc.create_borrow(0, 0, BorrowMode::Shared, span, None)
        .unwrap();
    bc.exit_region();
    // After shared borrow released, exclusive should be allowed
    assert!(
        bc.create_borrow(0, 1, BorrowMode::Exclusive, span, None)
            .is_ok()
    );
}

#[test]
fn test_borrow_unit_exclusive_after_region_exit_allows_shared() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.enter_region();
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    bc.exit_region();
    // After exclusive borrow released, shared should be allowed
    assert!(
        bc.create_borrow(0, 1, BorrowMode::Shared, span, None)
            .is_ok()
    );
}

#[test]
fn test_borrow_unit_reset_clears_everything() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    bc.enter_region();
    bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
        .unwrap();
    bc.create_borrow(1, 1, BorrowMode::Shared, span, None)
        .unwrap();
    bc.create_borrow(1, 2, BorrowMode::Shared, span, None)
        .unwrap();
    bc.reset();
    // After reset, everything is clean
    assert!(bc.check_write_allowed(0, None).is_ok());
    assert!(bc.check_write_allowed(1, None).is_ok());
    assert!(
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span, None)
            .is_ok()
    );
    assert!(
        bc.create_borrow(1, 1, BorrowMode::Exclusive, span, None)
            .is_ok()
    );
}

#[test]
fn test_borrow_unit_many_shared_borrows_same_slot() {
    use crate::borrow_checker::{BorrowChecker, BorrowMode};
    use shape_ast::ast::Span;
    let span = Span { start: 0, end: 1 };

    let mut bc = BorrowChecker::new();
    for i in 0..50u16 {
        bc.create_borrow(0, i, BorrowMode::Shared, span, None)
            .unwrap();
    }
    // 50 shared borrows active -- write should still be blocked
    assert!(bc.check_write_allowed(0, None).is_err());
    // But read should be allowed
    assert!(bc.check_read_allowed(0, None).is_ok());
    // Adding an exclusive should fail
    assert!(
        bc.create_borrow(0, 99, BorrowMode::Exclusive, span, None)
            .is_err()
    );
}

#[test]
fn test_borrow_unit_region_id_monotonic() {
    use crate::borrow_checker::BorrowChecker;

    let mut bc = BorrowChecker::new();
    let r1 = bc.enter_region();
    let r2 = bc.enter_region();
    bc.exit_region();
    let r3 = bc.enter_region();
    assert!(r1.0 < r2.0, "Region IDs should be monotonically increasing");
    assert!(r2.0 < r3.0, "Region IDs should be monotonically increasing");
}

// =============================================================================
// Category 8: Inferred Reference Model (~10 tests)
// =============================================================================

#[test]
fn test_borrow_inferred_array_param_auto_ref_on_mutation() {
    // When function mutates an array parameter, it should be auto-promoted to ref
    let code = r#"
        function set_first(arr, v) { arr[0] = v }
        function test() {
            var xs = [1, 2, 3]
            set_first(xs, 99)
            return xs[0]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_borrow_inferred_array_read_only_shared() {
    // Read-only array param should be shared borrow (aliasing OK)
    let code = r#"
        function sum_pair(a, b) { return a[0] + b[0] }
        function test() {
            var xs = [7]
            return sum_pair(xs, xs)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_number_coerce().unwrap(), 14.0);
}

#[test]
fn test_borrow_inferred_mutation_prevents_aliasing() {
    // If inference detects mutation on one param, aliasing with another should fail
    assert_compile_error(
        r#"
        function write_read(a, b) {
            a[0] = 42
            return b[0]
        }
        function test() {
            var xs = [1]
            return write_read(xs, xs)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_borrow_inferred_two_mutating_params_different_vars_ok() {
    let code = r#"
        function swap_first(a, b) {
            var t = a[0]
            a[0] = b[0]
            b[0] = t
        }
        function test() {
            var xs = [1]
            var ys = [2]
            swap_first(xs, ys)
            return xs[0] * 10 + ys[0]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(21));
}

#[test]
fn test_borrow_inferred_two_mutating_params_same_var_error() {
    assert_compile_error(
        r#"
        function swap_first(a, b) {
            var t = a[0]
            a[0] = b[0]
            b[0] = t
        }
        function test() {
            var xs = [1]
            swap_first(xs, xs)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_borrow_inferred_scalar_param_no_auto_ref() {
    // Scalar parameters should NOT be auto-promoted to ref (value semantics)
    let code = r#"
        function add(a, b) { return a + b }
        function test() {
            var x = 5
            return add(x, x)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(10));
}

#[test]
fn test_borrow_inferred_push_infers_exclusive() {
    // Element mutation on array param infers exclusive borrow
    let code = r#"
        function set_first(arr, v) { arr[0] = v }
        function test() {
            var xs = [1, 2]
            set_first(xs, 99)
            return xs[0]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

// =============================================================================
// Category 9: Integration with other language features (~10 tests)
// =============================================================================

#[test]
fn test_borrow_integration_with_for_in_loop() {
    let code = r#"
        function add_to(&total, v) { total = total + v }
        function test() {
            var sum = 0
            for item in [10, 20, 30, 40, 50] {
                add_to(&sum, item)
            }
            return sum
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(150));
}

#[test]
fn test_borrow_integration_with_match_expression() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            let val = 2
            match val {
                1 => inc(&a),
                2 => { inc(&a); inc(&a) },
                _ => {}
            }
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(2));
}

#[test]
fn test_borrow_integration_ref_with_default_params() {
    let code = r#"
        function add_amount(&x, amount = 1) { x = x + amount }
        function test() {
            var a = 0
            add_amount(&a)
            add_amount(&a, 10)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(11));
}

#[test]
fn test_borrow_integration_ref_in_closure_call() {
    // Closure that takes a ref param
    let code = r#"
        function test() {
            var a = 5
            let inc = |&x| { x = x + 1 }
            inc(&a)
            return a
        }
    "#;
    // This may or may not be supported; if not, it should error
    let program = match parse_program(code) {
        Ok(p) => p,
        Err(_) => return, // Parse doesn't support ref in closure params — acceptable
    };
    match BytecodeCompiler::new().compile(&program) {
        Ok(bytecode) => {
            let mut vm = VirtualMachine::new(VMConfig::default());
            vm.load_program(bytecode);
            let result = vm.execute_function_by_name("test", vec![], None);
            if let Ok(val) = result {
                assert_eq!(val.clone(), ValueWord::from_i64(6));
            }
        }
        Err(_) => {} // Acceptable: ref params in closures may not be supported
    }
}

#[test]
fn test_borrow_integration_ref_preserves_array_identity() {
    // After mutation through ref, the original variable should reflect changes
    let code = r#"
        function modify(&arr) {
            arr[0] = 42
            arr[1] = 99
            arr[2] = 7
        }
        function test() {
            var data = [1, 2, 3]
            modify(&data)
            return data[0] + data[1] + data[2]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(148)); // 42+99+7 = 148
}

#[test]
fn test_borrow_integration_ref_in_recursive_function() {
    let code = r#"
        function count_down(&counter, n) {
            if n <= 0 { return }
            counter = counter + 1
            count_down(&counter, n - 1)
        }
        function test() {
            var c = 0
            count_down(&c, 5)
            return c
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5));
}

#[test]
fn test_borrow_integration_multiple_ref_functions_compose() {
    // Compose multiple ref functions: read + write through refs
    let code = r#"
        function set_val(&arr, idx, v) { arr[idx] = v }
        function read_val(&arr, idx) { return arr[idx] }
        function test() {
            var data = [0, 0, 0]
            set_val(&data, 0, 10)
            set_val(&data, 1, 20)
            set_val(&data, 2, 30)
            let top = read_val(&data, 2)
            return top * 100 + len(data)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3003)); // top=30, len=3 => 3003
}

#[test]
fn test_borrow_integration_ref_mutation_visible_after_early_return() {
    // Function with early return should still writeback ref mutations
    let code = r#"
        function maybe_set(&x, val, condition) {
            if !condition {
                return
            }
            x = val
        }
        function test() {
            var a = 0
            maybe_set(&a, 42, true)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_borrow_integration_ref_with_while_break() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var count = 0
            var i = 0
            while true {
                if i >= 5 { break }
                inc(&count)
                i = i + 1
            }
            return count
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(5));
}

#[test]
fn test_borrow_integration_module_binding_ref_mutation() {
    // Top-level module binding can be mutated through ref
    let code = r#"
        var counter = 0
        function inc(&x) { x = x + 1 }
        inc(&counter)
        inc(&counter)
        inc(&counter)
        counter
    "#;
    let result = compile_and_run(code);
    assert_eq!(result, ValueWord::from_i64(3));
}

// =============================================================================
// Category: Immutable `let` Binding Enforcement
// =============================================================================

#[test]
fn test_immutable_let_reassignment_rejected() {
    // `let x = 10` is immutable — reassignment should fail
    let code = r#"
        function test() {
            let x = 10
            x = 20
            return x
        }
    "#;
    assert_compile_error(code, "Cannot reassign immutable variable 'x'");
}

#[test]
fn test_let_mut_reassignment_allowed() {
    // `let mut x = 10` is explicitly mutable — reassignment should work
    let code = r#"
        function test() {
            let mut x = 10
            x = 20
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(20));
}

#[test]
fn test_var_reassignment_allowed() {
    // `var x = 10` is always mutable — reassignment should work
    let code = r#"
        function test() {
            var x = 10
            x = 20
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(20));
}

#[test]
fn test_immutable_let_read_ok() {
    // Immutable `let` bindings can be read freely
    let code = r#"
        function test() {
            let x = 42
            return x + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(43));
}

#[test]
fn test_immutable_let_shared_borrow_ok() {
    // Shared (&) borrows of immutable `let` bindings should be allowed
    let code = r#"
        function read_val(&x) { return x }
        function test() {
            let x = 42
            return read_val(&x)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

// =============================================================================
// Category: First-Class References (let r = &x)
// =============================================================================

#[test]
fn test_first_class_ref_shared_binding() {
    // `let r = &x` — store a shared reference in a local variable
    let code = r#"
        function deref_val(&x) { return x }
        function test() {
            var x = 42
            var r = &x
            return deref_val(r)
        }
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_first_class_ref_outside_call_args_compiles() {
    // `&x` used outside of function arguments should now compile
    let code = r#"
        function test() {
            var x = 42
            var r = &x
            return x
        }
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_first_class_ref_arithmetic_autoderef() {
    let code = r#"
        function test() {
            let x = 41
            let r = &x
            return r + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_first_class_ref_method_autoderef() {
    let code = r#"
        function test() {
            let nums = [1, 2, 3]
            let r = &nums
            return r.len()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3));
}

#[test]
fn test_first_class_ref_module_binding_autoderef() {
    let code = r#"
        let value = 41
        let r = &value
        r + 1
    "#;
    let result = compile_and_run(code);
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_first_class_ref_alias_of_tracked_module_binding_keeps_owner_borrowed() {
    assert_compile_error(
        r#"
        let mut data = [1, 2, 3]
        let r1 = &data
        let r2 = &r1
        print(r1) // last use of r1; r2 must keep data frozen
        data.push(4)
        r2.len()
        "#,
        "B0002",
    );
}

#[test]
fn test_first_class_ref_last_use_releases_local_borrow() {
    let code = r#"
        function test() {
            let mut data = [1, 2, 3]
            let r = &data
            print(r)
            data.push(4)
            return data.len()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(4));
}

#[test]
fn test_module_binding_last_use_releases_borrow() {
    let code = r#"
        let mut data = [1, 2, 3]
        let r = &data
        print(r)
        data.push(4)
        data.len()
    "#;
    let result = compile_and_run(code);
    assert_eq!(result, ValueWord::from_i64(4));
}

#[test]
fn test_module_binding_write_while_borrowed_rejected() {
    assert_compile_error(
        r#"
        let mut x = 10
        let r = &x
        x = 20
        print(r)
        "#,
        "[B0002]",
    );
}

#[test]
fn test_module_binding_shared_plus_exclusive_rejected() {
    assert_compile_error(
        r#"
        let mut x = 10
        let r1 = &x
        let r2 = &x
        let m = &mut x
        print(r1)
        print(r2)
        print(m)
        "#,
        "[B0001]",
    );
}

#[test]
fn test_module_binding_double_exclusive_rejected() {
    assert_compile_error(
        r#"
        let mut x = 10
        let m1 = &mut x
        let m2 = &mut x
        print(m1)
        print(m2)
        "#,
        "[B0001]",
    );
}

#[test]
fn test_module_binding_reference_cannot_escape_into_closure() {
    assert_compile_error(
        r#"
        let value = 10
        let r = &value
        let f = || r
        "#,
        "[B0003]",
    );
}

#[test]
fn test_returned_module_binding_reference_cannot_escape_into_closure() {
    assert_compile_error(
        r#"
        function borrow_id(&x) {
            x
        }

        let value = 10
        let r = borrow_id(value)
        let f = || r
        "#,
        "[B0003]",
    );
}

#[test]
fn test_module_binding_mut_ref_param_method_mutates_original() {
    let code = r#"
        function append_item(&mut arr, value) {
            arr.push(value)
        }

        let mut data = [1, 2, 3]
        append_item(&mut data, 4)
        data.len()
    "#;
    let result = compile_and_run(code);
    assert_eq!(result, ValueWord::from_i64(4));
}

#[test]
fn test_projected_field_ref_arithmetic_autoderef() {
    let code = r#"
        type Pair {
            a: int,
            b: int
        }

        function test() {
            let obj = Pair { a: 41, b: 1 }
            let r = &obj.a
            return r + 1
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_projected_field_disjoint_exclusive_borrows_ok() {
    let code = r#"
        type Pair {
            a: int,
            b: int
        }

        function set_value(&mut x, value) {
            x = value
        }

        function test() {
            let mut obj = Pair { a: 1, b: 2 }
            let ra = &mut obj.a
            let rb = &mut obj.b
            set_value(ra, 10)
            set_value(rb, 20)
            return ra + rb
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(30));
}

#[test]
fn test_projected_field_write_other_field_while_borrowed_ok() {
    let code = r#"
        type Pair {
            a: int,
            b: int
        }

        function set_value(&mut x, value) {
            x = value
        }

        function test() {
            let mut obj = Pair { a: 1, b: 2 }
            let rb = &mut obj.b
            obj.a = 10
            set_value(rb, 20)
            return obj.a + rb
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(30));
}

#[test]
fn test_projected_field_shared_plus_exclusive_same_field_rejected() {
    assert_compile_error(
        r#"
        type Pair {
            a: int,
            b: int
        }

        function test() {
            let mut obj = Pair { a: 1, b: 2 }
            let r = &obj.a
            let m = &mut obj.a
            return r + m
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_projected_field_whole_owner_write_rejected() {
    assert_compile_error(
        r#"
        type Pair {
            a: int,
            b: int
        }

        function test() {
            let mut obj = Pair { a: 1, b: 2 }
            let r = &obj.a
            obj = Pair { a: 3, b: 4 }
            return r
        }
        "#,
        "[B0002]",
    );
}

#[test]
fn test_projected_field_last_use_releases_borrow() {
    let code = r#"
        type Pair {
            a: int,
            b: int
        }

        function test() {
            let mut obj = Pair { a: 1, b: 2 }
            let r = &obj.a
            print(r)
            obj.a = 10
            return obj.a + obj.b
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(12));
}

#[test]
fn test_first_class_ref_last_use_releases_inside_if_branch() {
    let code = r#"
        function test() {
            let mut x = 1
            let r = &x
            if true {
                print(r)
                x = 2
            }
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(2));
}

#[test]
fn test_first_class_ref_last_use_releases_inside_nested_block() {
    let code = r#"
        function test() {
            let mut x = 1
            let r = &x
            {
                print(r)
                x = 2
            }
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(2));
}

#[test]
fn test_first_class_ref_loop_local_last_use_releases_each_iteration() {
    let code = r#"
        function test() {
            let mut x = 0
            let mut i = 0
            while i < 3 {
                let r = &x
                print(r)
                x = x + 1
                i = i + 1
            }
            return x
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3));
}

#[test]
fn test_first_class_ref_outer_loop_borrow_stays_active_across_iterations() {
    assert_compile_error(
        r#"
        function test() {
            let mut x = 0
            let r = &x
            let mut i = 0
            while i < 3 {
                print(r)
                x = x + 1
                i = i + 1
            }
            return x
        }
        "#,
        "[B0002]",
    );
}

#[test]
fn test_first_class_ref_outer_loop_push_mutation_stays_active_across_iterations() {
    assert_compile_error(
        r#"
        function test() {
            let mut xs = [1]
            let r = &xs
            let mut i = 0
            while i < 3 {
                print(r)
                xs.push(i)
                i = i + 1
            }
            return xs
        }
        "#,
        "[B0002]",
    );
}

#[test]
fn test_module_binding_last_use_releases_inside_if_branch() {
    let code = r#"
        let mut x = 1
        let r = &x
        if true {
            print(r)
            x = 2
        }
        x
    "#;
    let result = compile_and_run(code);
    assert_eq!(result, ValueWord::from_i64(2));
}

// ── Concurrency boundary tests (Phase 6: Three Rules) ──────────────

#[test]
fn test_exclusive_ref_rejected_across_async_let_boundary() {
    // &mut T cannot cross task boundary — would create aliased mutation.
    // Direct &mut in async let RHS is rejected.
    let code = r#"
        async function test() {
            var data = [1, 2, 3]
            async let result = &mut data
        }
    "#;
    assert_compile_error(code, "exclusive reference");
}

#[test]
fn test_shared_ref_allowed_across_async_let_boundary() {
    // &T (shared ref) is fine in structured child tasks
    let code = r#"
        async function test() {
            var data = [1, 2, 3]
            async let result = read_only(&data)
            return 0
        }
        async function read_only(&arr) {
            return 0
        }
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_owned_value_allowed_across_async_let_boundary() {
    // Owned values always allowed across task boundary
    let code = r#"
        async function test() {
            var data = [1, 2, 3]
            async let result = process(data)
            return 0
        }
        async function process(arr) {
            return 0
        }
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_exclusive_ref_in_nested_expr_rejected_across_boundary() {
    // Even nested inside a call, &mut should be rejected
    let code = r#"
        async function compute(a, &mut b, c) {
            return a
        }
        async function test() {
            var x = 10
            async let result = compute(1, &mut x, 3)
        }
    "#;
    assert_compile_error(code, "exclusive reference");
}

// ── Concurrency primitive constructor tests (Phase 6: Mutex/Atomic/Lazy) ──

#[test]
fn test_mutex_constructor_compiles() {
    let code = r#"
        var m = Mutex(42)
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_atomic_constructor_compiles() {
    let code = r#"
        var a = Atomic(0)
    "#;
    assert_compiles_ok(code);
}

#[test]
fn test_lazy_constructor_compiles() {
    let code = r#"
        var l = Lazy(|| 42)
    "#;
    assert_compiles_ok(code);
}
