//! Stress tests for basic let bindings: int, number, bool, string, null,
//! type-annotated lets, width-typed locals, const bindings, expressions,
//! function parameters, module-level bindings, and large local counts.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Basic let binding
// =========================================================================

/// Verifies basic integer let binding.
#[test]
fn test_let_bind_int() {
    ShapeTest::new("let x = 42\nx").expect_number(42.0);
}

/// Verifies let binding of zero.
#[test]
fn test_let_bind_zero() {
    ShapeTest::new("let x = 0\nx").expect_number(0.0);
}

/// Verifies let binding of negative integer.
#[test]
fn test_let_bind_negative_int() {
    ShapeTest::new("let x = -7\nx").expect_number(-7.0);
}

/// Verifies let binding of float number.
#[test]
fn test_let_bind_number() {
    ShapeTest::new("let x = 3.14\nx").expect_number(3.14);
}

/// Verifies let binding of true.
#[test]
fn test_let_bind_bool_true() {
    ShapeTest::new("let x = true\nx").expect_bool(true);
}

/// Verifies let binding of false.
#[test]
fn test_let_bind_bool_false() {
    ShapeTest::new("let x = false\nx").expect_bool(false);
}

/// Verifies let binding of string.
#[test]
fn test_let_bind_string() {
    ShapeTest::new("let x = \"hello\"\nx").expect_string("hello");
}

/// Verifies let binding of empty string.
#[test]
fn test_let_bind_empty_string() {
    ShapeTest::new("let x = \"\"\nx").expect_string("");
}

/// Verifies let binding of None.
#[test]
fn test_let_bind_null() {
    ShapeTest::new("let x = None\nx").expect_none();
}

// =========================================================================
// 2. Type-annotated let
// =========================================================================

/// Verifies type-annotated int binding.
#[test]
fn test_let_typed_int() {
    ShapeTest::new("fn test() -> int { let x: int = 42\nreturn x }\ntest()").expect_number(42.0);
}

/// Verifies type-annotated number binding.
#[test]
fn test_let_typed_number() {
    ShapeTest::new("fn test() -> number { let x: number = 3.14\nreturn x }\ntest()")
        .expect_number(3.14);
}

/// Verifies type-annotated bool binding.
#[test]
fn test_let_typed_bool() {
    ShapeTest::new("fn test() -> bool { let x: bool = true\nreturn x }\ntest()")
        .expect_bool(true);
}

/// Verifies type-annotated string binding.
#[test]
fn test_let_typed_string() {
    ShapeTest::new("fn test() -> string { let x: string = \"abc\"\nreturn x }\ntest()")
        .expect_string("abc");
}

// =========================================================================
// 6. Width-typed locals
// =========================================================================

/// Verifies i8 width type.
#[test]
fn test_width_i8() {
    ShapeTest::new("fn test() -> int { let a: i8 = 100\nreturn a }\ntest()").expect_number(100.0);
}

/// Verifies i16 width type.
#[test]
fn test_width_i16() {
    ShapeTest::new("fn test() -> int { let a: i16 = 1000\nreturn a }\ntest()")
        .expect_number(1000.0);
}

/// Verifies i32 width type.
#[test]
fn test_width_i32() {
    ShapeTest::new("fn test() -> int { let a: i32 = 100000\nreturn a }\ntest()")
        .expect_number(100000.0);
}

/// Verifies u8 width type.
#[test]
fn test_width_u8() {
    ShapeTest::new("fn test() -> int { let a: u8 = 200\nreturn a }\ntest()").expect_number(200.0);
}

/// Verifies u16 width type.
#[test]
fn test_width_u16() {
    ShapeTest::new("fn test() -> int { let a: u16 = 50000\nreturn a }\ntest()")
        .expect_number(50000.0);
}

/// Verifies u32 width type.
#[test]
fn test_width_u32() {
    ShapeTest::new("fn test() -> int { let a: u32 = 3000000\nreturn a }\ntest()")
        .expect_number(3000000.0);
}

/// Verifies u64 width type.
#[test]
fn test_width_u64() {
    ShapeTest::new("fn test() -> int { let a: u64 = 999999\nreturn a }\ntest()")
        .expect_number(999999.0);
}

/// Verifies i8 negative value.
#[test]
fn test_width_i8_negative() {
    ShapeTest::new("fn test() -> int { let a: i8 = -128\nreturn a }\ntest()")
        .expect_number(-128.0);
}

/// Verifies i16 negative value.
#[test]
fn test_width_i16_negative() {
    ShapeTest::new("fn test() -> int { let a: i16 = -32768\nreturn a }\ntest()")
        .expect_number(-32768.0);
}

/// Verifies i8 max boundary.
#[test]
fn test_width_i8_max_boundary() {
    ShapeTest::new("fn test() -> int { let a: i8 = 127\nreturn a }\ntest()").expect_number(127.0);
}

/// Verifies u8 max boundary.
#[test]
fn test_width_u8_max_boundary() {
    ShapeTest::new("fn test() -> int { let a: u8 = 255\nreturn a }\ntest()").expect_number(255.0);
}

/// Verifies u16 max boundary.
#[test]
fn test_width_u16_max_boundary() {
    ShapeTest::new("fn test() -> int { let a: u16 = 65535\nreturn a }\ntest()")
        .expect_number(65535.0);
}

/// Verifies i8 overflow causes compile error.
#[test]
fn test_width_i8_overflow_compile_error() {
    ShapeTest::new("fn test() { let x: i8 = 128\nreturn x }").expect_run_err();
}

/// Verifies u8 negative causes compile error.
#[test]
fn test_width_u8_negative_compile_error() {
    ShapeTest::new("fn test() { let x: u8 = -1\nreturn x }").expect_run_err();
}

/// Verifies u16 overflow causes compile error.
#[test]
fn test_width_u16_overflow_compile_error() {
    ShapeTest::new("fn test() { let x: u16 = 65536\nreturn x }").expect_run_err();
}

/// Verifies width-typed arithmetic.
#[test]
fn test_width_typed_arithmetic() {
    ShapeTest::new(
        "fn test() -> int {
            let a: i8 = 10
            let b: i8 = 20
            return a + b
        }\ntest()",
    )
    .expect_number(30.0);
}

/// Verifies width-typed u8 arithmetic.
#[test]
fn test_width_typed_u8_arithmetic() {
    ShapeTest::new(
        "fn test() -> int {
            let a: u8 = 100
            let b: u8 = 50
            return a + b
        }\ntest()",
    )
    .expect_number(150.0);
}

/// Verifies u8 var reassign truncates.
#[test]
fn test_width_var_reassign_truncates_u8() {
    ShapeTest::new(
        "fn test() -> int {
            var x: u8 = 10
            x = 300
            return x
        }\ntest()",
    )
    .expect_number(44.0);
}

/// Verifies i8 var reassign truncates with sign extension.
#[test]
fn test_width_var_reassign_truncates_i8() {
    ShapeTest::new(
        "fn test() -> int {
            var x: i8 = 0
            x = 200
            return x
        }\ntest()",
    )
    .expect_number(-56.0);
}

/// Verifies u16 var reassign truncates.
#[test]
fn test_width_var_reassign_truncates_u16() {
    ShapeTest::new(
        "fn test() -> int {
            var x: u16 = 0
            x = 70000
            return x
        }\ntest()",
    )
    .expect_number(4464.0);
}

// =========================================================================
// 7. Variable in expressions
// =========================================================================

/// Verifies use after binding with addition.
#[test]
fn test_use_after_binding_add() {
    ShapeTest::new("fn test() -> int { let a = 3\nlet b = 4\nreturn a + b }\ntest()")
        .expect_number(7.0);
}

/// Verifies multi-step computation.
#[test]
fn test_multi_step_computation() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 2
            let b = 3
            let c = a * b
            let d = c + 1
            return d
        }\ntest()",
    )
    .expect_number(7.0);
}

/// Verifies chain of bindings.
#[test]
fn test_chain_of_bindings() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 1
            let b = a + 1
            let c = b + 1
            let d = c + 1
            let e = d + 1
            return e
        }\ntest()",
    )
    .expect_number(5.0);
}

/// Verifies complex expression with variables.
#[test]
fn test_complex_expression_with_vars() {
    ShapeTest::new(
        "fn test() -> int {
            let x = 10
            let y = 20
            let z = x * y + x - y
            return z
        }\ntest()",
    )
    .expect_number(190.0);
}

/// Verifies variables in nested expressions.
#[test]
fn test_variable_in_nested_expressions() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 2
            let b = 3
            let c = (a + b) * (a - b)
            return c
        }\ntest()",
    )
    .expect_number(-5.0);
}

// =========================================================================
// 8. Immutable reassignment error
// =========================================================================

/// Verifies immutable let reassignment fails.
#[test]
fn test_let_immutable_reassignment_error() {
    ShapeTest::new("let x = 1\nx = 2\nx").expect_run_err();
}

/// Verifies immutable let reassignment in function fails.
#[test]
fn test_let_immutable_reassignment_in_function() {
    ShapeTest::new("fn test() { let x = 1\nx = 2\nreturn x }").expect_run_err();
}

/// Verifies const reassignment fails.
#[test]
fn test_const_reassignment_error() {
    ShapeTest::new("const C = 1\nC = 2\nC").expect_run_err();
}

/// Verifies const reassignment in function fails.
#[test]
fn test_const_reassignment_in_function_error() {
    ShapeTest::new("fn test() { const C = 10\nC = 20\nreturn C }").expect_run_err();
}

// =========================================================================
// 9. Undefined variable error
// =========================================================================

/// Verifies undefined variable fails.
#[test]
fn test_undefined_variable_error() {
    ShapeTest::new("x").expect_run_err();
}

/// Verifies undefined variable in expression fails.
#[test]
fn test_undefined_variable_in_expr_error() {
    ShapeTest::new("let a = 1\nlet b = a + c\nb").expect_run_err();
}

/// Verifies undefined variable in function fails.
#[test]
fn test_undefined_variable_in_fn_error() {
    ShapeTest::new("fn test() -> int { return z }").expect_run_err();
}

// =========================================================================
// 11. Variable as function arg
// =========================================================================

/// Verifies passing variable to function.
#[test]
fn test_pass_var_to_function() {
    ShapeTest::new(
        "fn double(n: int) -> int { return n * 2 }
        fn test() -> int {
            let x = 5
            return double(x)
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies passing multiple variables to function.
#[test]
fn test_pass_multiple_vars_to_function() {
    ShapeTest::new(
        "fn add(a: int, b: int) -> int { return a + b }
        fn test() -> int {
            let x = 3
            let y = 7
            return add(x, y)
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies variable from function return.
#[test]
fn test_var_from_function_return() {
    ShapeTest::new(
        "fn make_value() -> int { return 42 }
        fn test() -> int {
            let x = make_value()
            return x
        }\ntest()",
    )
    .expect_number(42.0);
}

/// Verifies chain of function calls through variables.
#[test]
fn test_var_as_function_arg_chain() {
    ShapeTest::new(
        "fn inc(n: int) -> int { return n + 1 }
        fn test() -> int {
            let a = 0
            let b = inc(a)
            let c = inc(b)
            let d = inc(c)
            return d
        }\ntest()",
    )
    .expect_number(3.0);
}

// =========================================================================
// 12. Multiple variables
// =========================================================================

/// Verifies many bindings summed together.
#[test]
fn test_many_bindings() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            return a + b + c + d + e
        }\ntest()",
    )
    .expect_number(15.0);
}

/// Verifies ten bindings summed together.
#[test]
fn test_ten_bindings() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            let f = 6
            let g = 7
            let h = 8
            let i = 9
            let j = 10
            return a + b + c + d + e + f + g + h + i + j
        }\ntest()",
    )
    .expect_number(55.0);
}

/// Verifies order of evaluation.
#[test]
fn test_order_of_evaluation_left_to_right() {
    ShapeTest::new(
        "fn test() -> int {
            let a = 10
            let b = a * 2
            let c = b + a
            return c
        }\ntest()",
    )
    .expect_number(30.0);
}

/// Verifies forward dependency chain.
#[test]
fn test_forward_dependency_chain() {
    ShapeTest::new(
        "fn test() -> int {
            let step1 = 1
            let step2 = step1 * 2
            let step3 = step2 * 2
            let step4 = step3 * 2
            return step4
        }\ntest()",
    )
    .expect_number(8.0);
}

// =========================================================================
// 15. Const
// =========================================================================

/// Verifies const int binding.
#[test]
fn test_const_int() {
    ShapeTest::new(
        "fn test() -> int {
            const PI_APPROX = 3
            return PI_APPROX
        }\ntest()",
    )
    .expect_number(3.0);
}

/// Verifies const used in expression.
#[test]
fn test_const_used_in_expression() {
    ShapeTest::new(
        "fn test() -> int {
            const BASE = 10
            let x = BASE + 5
            return x
        }\ntest()",
    )
    .expect_number(15.0);
}

/// Verifies multiple const bindings.
#[test]
fn test_multiple_const_bindings() {
    ShapeTest::new(
        "fn test() -> int {
            const A = 1
            const B = 2
            const C = 3
            return A + B + C
        }\ntest()",
    )
    .expect_number(6.0);
}

// =========================================================================
// 16. Additional edge cases
// =========================================================================

/// Verifies large int binding.
#[test]
fn test_let_bind_large_int() {
    ShapeTest::new("fn test() -> int { let x = 1000000\nreturn x }\ntest()")
        .expect_number(1_000_000.0);
}

/// Verifies negative number binding.
#[test]
fn test_let_bind_negative_number() {
    ShapeTest::new("fn test() -> number { let x = -2.5\nreturn x }\ntest()")
        .expect_number(-2.5);
}

/// Verifies let bind expression result.
#[test]
fn test_let_bind_expression_result() {
    ShapeTest::new("fn test() -> int { let x = 3 + 4 * 2\nreturn x }\ntest()")
        .expect_number(11.0);
}

/// Verifies let bind comparison result.
#[test]
fn test_let_bind_comparison_result() {
    ShapeTest::new("fn test() -> bool { let x = 5 > 3\nreturn x }\ntest()").expect_bool(true);
}

/// Verifies let bind comparison false.
#[test]
fn test_let_bind_comparison_false() {
    ShapeTest::new("fn test() -> bool { let x = 2 > 3\nreturn x }\ntest()").expect_bool(false);
}

/// Verifies let bind logical and.
#[test]
fn test_let_bind_logical_and() {
    ShapeTest::new(
        "fn test() -> bool { let a = true\nlet b = false\nlet c = a and b\nreturn c }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies let bind logical or.
#[test]
fn test_let_bind_logical_or() {
    ShapeTest::new(
        "fn test() -> bool { let a = true\nlet b = false\nlet c = a or b\nreturn c }\ntest()",
    )
    .expect_bool(true);
}

// =========================================================================
// 17. Function-scoped variables
// =========================================================================

/// Verifies function params as locals.
#[test]
fn test_function_params_as_locals() {
    ShapeTest::new(
        "fn square(n: int) -> int { return n * n }
        fn test() -> int { return square(7) }\ntest()",
    )
    .expect_number(49.0);
}

/// Verifies function local does not leak.
#[test]
fn test_function_local_does_not_leak() {
    ShapeTest::new(
        "fn foo() -> int {
            let secret = 42
            return secret
        }
        fn bar() -> int {
            return secret
        }",
    )
    .expect_run_err();
}

/// Verifies two functions with same local name.
#[test]
fn test_two_functions_same_local_name() {
    ShapeTest::new(
        "fn foo() -> int {
            let x = 10
            return x
        }
        fn bar() -> int {
            let x = 20
            return x
        }
        fn test() -> int {
            return foo() + bar()
        }\ntest()",
    )
    .expect_number(30.0);
}

// =========================================================================
// 18. Recursive function with local variable
// =========================================================================

/// Verifies recursion with local variable.
#[test]
fn test_recursive_with_local() {
    ShapeTest::new(
        "fn factorial(n: int) -> int {
            if n <= 1 { return 1 }
            let sub = factorial(n - 1)
            return n * sub
        }
        fn test() -> int { return factorial(5) }\ntest()",
    )
    .expect_number(120.0);
}

// =========================================================================
// 19. Variables with string concatenation
// =========================================================================

/// Verifies string variable concatenation.
#[test]
fn test_string_var_concat() {
    ShapeTest::new(
        "fn test() -> string {
            let a = \"hello\"
            let b = \" world\"
            return a + b
        }\ntest()",
    )
    .expect_string("hello world");
}

// =========================================================================
// 20. Module-level bindings
// =========================================================================

/// Verifies module-level let.
#[test]
fn test_module_level_let() {
    ShapeTest::new("let x = 42\nx").expect_number(42.0);
}

/// Verifies module-level multiple lets.
#[test]
fn test_module_level_multiple_lets() {
    ShapeTest::new("let a = 10\nlet b = 20\na + b").expect_number(30.0);
}

/// Verifies module-level const.
#[test]
fn test_module_level_const() {
    ShapeTest::new("const X = 99\nX").expect_number(99.0);
}

// =========================================================================
// 21. Edge cases: same name in different scopes
// =========================================================================

/// Verifies function shadows module binding.
#[test]
fn test_same_name_function_and_module() {
    ShapeTest::new(
        "let x = 100
        fn test() -> int {
            let x = 1
            return x
        }\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies function reads module binding.
#[test]
fn test_function_reads_module_binding() {
    ShapeTest::new(
        "let GLOBAL = 42
        fn test() -> int {
            return GLOBAL
        }\ntest()",
    )
    .expect_number(42.0);
}

// =========================================================================
// 23. Width types: mixed widths
// =========================================================================

/// Verifies mixed width addition.
#[test]
fn test_mixed_width_add() {
    ShapeTest::new(
        "fn test() -> int {
            let a: i8 = 10
            let b: i16 = 200
            return a + b
        }\ntest()",
    )
    .expect_number(210.0);
}

/// Verifies u8 zero.
#[test]
fn test_width_u8_zero() {
    ShapeTest::new("fn test() -> int { let x: u8 = 0\nreturn x }\ntest()").expect_number(0.0);
}

/// Verifies i32 large negative.
#[test]
fn test_width_i32_large_negative() {
    ShapeTest::new("fn test() -> int { let x: i32 = -2000000000\nreturn x }\ntest()")
        .expect_number(-2_000_000_000.0);
}

// =========================================================================
// 25. Unused variables
// =========================================================================

/// Verifies unused variable compiles fine.
#[test]
fn test_unused_variable_compiles() {
    ShapeTest::new(
        "fn test() -> int {
            let unused = 999
            return 42
        }\ntest()",
    )
    .expect_number(42.0);
}

// =========================================================================
// 26. Large number of locals
// =========================================================================

/// Verifies twenty locals summed together.
#[test]
fn test_twenty_locals() {
    ShapeTest::new(
        "fn test() -> int {
            let a1 = 1
            let a2 = 2
            let a3 = 3
            let a4 = 4
            let a5 = 5
            let a6 = 6
            let a7 = 7
            let a8 = 8
            let a9 = 9
            let a10 = 10
            let a11 = 11
            let a12 = 12
            let a13 = 13
            let a14 = 14
            let a15 = 15
            let a16 = 16
            let a17 = 17
            let a18 = 18
            let a19 = 19
            let a20 = 20
            return a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10 + a11 + a12 + a13 + a14 + a15 + a16 + a17 + a18 + a19 + a20
        }\ntest()",
    )
    .expect_number(210.0);
}

// =========================================================================
// 29. Number (float) variables
// =========================================================================

/// Verifies number variable operations.
#[test]
fn test_number_var_operations() {
    ShapeTest::new(
        "fn test() -> number {
            let a = 1.5
            let b = 2.5
            return a + b
        }\ntest()",
    )
    .expect_number(4.0);
}

/// Verifies number variable multiplication.
#[test]
fn test_number_var_multiply() {
    ShapeTest::new(
        "fn test() -> number {
            let a = 3.0
            let b = 0.5
            return a * b
        }\ntest()",
    )
    .expect_number(1.5);
}

// =========================================================================
// 30. Const used with other constructs
// =========================================================================

/// Verifies const in expression with var.
#[test]
fn test_const_in_expression_with_var() {
    ShapeTest::new(
        "fn test() -> int {
            const OFFSET = 100
            var x = 5
            x = x + OFFSET
            return x
        }\ntest()",
    )
    .expect_number(105.0);
}
