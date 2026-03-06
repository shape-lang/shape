//! Stress tests for compound assignment, precedence, mixed-type,
//! and complex combined arithmetic expressions.
//!
//! Migrated from shape-vm stress_02_arithmetic.rs — compound assignment,
//! precedence, overflow, control flow, and misc sections.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// 8. OPERATOR PRECEDENCE
// =====================================================================

/// Verifies multiplication before addition.
#[test]
fn precedence_mul_before_add() {
    ShapeTest::new("1 + 2 * 3").expect_number(7.0);
}

/// Verifies division before addition.
#[test]
fn precedence_div_before_add() {
    ShapeTest::new("10 + 6 / 3").expect_number(12.0);
}

/// Verifies multiplication before subtraction.
#[test]
fn precedence_mul_before_sub() {
    ShapeTest::new("10 - 2 * 3").expect_number(4.0);
}

/// Verifies parentheses override precedence.
#[test]
fn precedence_parentheses_override() {
    ShapeTest::new("(1 + 2) * 3").expect_number(9.0);
}

/// Verifies nested parentheses.
#[test]
fn precedence_nested_parentheses() {
    ShapeTest::new("((1 + 2) * (3 + 4))").expect_number(21.0);
}

/// Verifies deeply nested parentheses.
#[test]
fn precedence_deeply_nested() {
    ShapeTest::new("(((2 + 3) * 2) - 1)").expect_number(9.0);
}

/// Verifies mixed ops: 2*3 + 4*5 - 6/2 = 23.
#[test]
fn precedence_mixed_ops() {
    ShapeTest::new("2 * 3 + 4 * 5 - 6 / 2").expect_number(23.0);
}

/// Verifies power is highest precedence: 2 ** 3 * 2 = 16.
#[test]
fn precedence_power_is_highest() {
    ShapeTest::new("2 ** 3 * 2").expect_number(16.0);
}

/// Verifies power with parentheses: 2 ** (3 ** 2) = 512.
#[test]
fn precedence_power_right_associative_paren() {
    ShapeTest::new("2 ** (3 ** 2)").expect_number(512.0);
}

/// Verifies modulo has same precedence as multiplication.
#[test]
fn precedence_mod_same_as_mul() {
    ShapeTest::new("10 + 7 % 3").expect_number(11.0);
}

/// Verifies unary negation has highest precedence.
#[test]
fn precedence_unary_neg_highest() {
    ShapeTest::new("-2 * 3").expect_number(-6.0);
}

// =====================================================================
// 9. COMPOUND ASSIGNMENT
// =====================================================================

/// Verifies += compound assignment.
#[test]
fn compound_add_assign() {
    ShapeTest::new(
        "fn run() {
            let mut x = 10
            x += 5
            return x
        }
run()",
    )
    .expect_number(15.0);
}

/// Verifies -= compound assignment.
#[test]
fn compound_sub_assign() {
    ShapeTest::new(
        "fn run() {
            let mut x = 10
            x -= 3
            return x
        }
run()",
    )
    .expect_number(7.0);
}

/// Verifies *= compound assignment.
#[test]
fn compound_mul_assign() {
    ShapeTest::new(
        "fn run() {
            let mut x = 10
            x *= 3
            return x
        }
run()",
    )
    .expect_number(30.0);
}

/// Verifies /= compound assignment.
#[test]
fn compound_div_assign() {
    ShapeTest::new(
        "fn run() {
            let mut x = 20
            x /= 4
            return x
        }
run()",
    )
    .expect_number(5.0);
}

/// Verifies %= compound assignment.
#[test]
fn compound_mod_assign() {
    ShapeTest::new(
        "fn run() {
            let mut x = 17
            x %= 5
            return x
        }
run()",
    )
    .expect_number(2.0);
}

/// Verifies **= compound assignment.
#[test]
fn compound_pow_assign() {
    ShapeTest::new(
        "fn run() {
            let mut x = 3
            x **= 3
            return x
        }
run()",
    )
    .expect_number(27.0);
}

/// Verifies multiple compound assignments: 10 -> 15 -> 12 -> 24 -> 6.
#[test]
fn compound_multiple_assigns() {
    ShapeTest::new(
        "fn run() {
            let mut x = 10
            x += 5
            x -= 3
            x *= 2
            x /= 4
            return x
        }
run()",
    )
    .expect_number(6.0);
}

/// Verifies += with floats.
#[test]
fn compound_add_assign_float() {
    ShapeTest::new(
        "fn run() {
            let mut x = 1.5
            x += 2.5
            return x
        }
run()",
    )
    .expect_number(4.0);
}

// =====================================================================
// 15. MIXED INT AND NUMBER ARITHMETIC
// =====================================================================

/// Verifies int + number behavior (may fail to compile or promote).
#[test]
fn mixed_int_plus_number() {
    // int + number may error or promote; just verify it doesn't panic
    ShapeTest::new("1 + 2.0").expect_run_ok();
}

/// Verifies number * int behavior.
#[test]
fn mixed_number_times_int() {
    ShapeTest::new("2.0 * 3").expect_run_ok();
}

// =====================================================================
// 16. ARITHMETIC IN CONTROL FLOW
// =====================================================================

/// Verifies arithmetic in if condition.
#[test]
fn arith_in_if_condition() {
    ShapeTest::new(
        "fn check() -> int {
            if 2 + 2 == 4 {
                return 1
            }
            return 0
        }
check()",
    )
    .expect_number(1.0);
}

/// Verifies arithmetic in while loop (sum 1..10 = 55).
#[test]
fn arith_in_while_loop() {
    ShapeTest::new(
        "fn sum_to_ten() -> int {
            let mut total = 0
            let mut i = 1
            while i <= 10 {
                total += i
                i += 1
            }
            return total
        }
sum_to_ten()",
    )
    .expect_number(55.0);
}

/// Verifies factorial via loop: 10! = 3628800.
#[test]
fn arith_factorial_loop() {
    ShapeTest::new(
        "fn factorial(n: int) -> int {
            let mut result = 1
            let mut i = 2
            while i <= n {
                result *= i
                i += 1
            }
            return result
        }
factorial(10)",
    )
    .expect_number(3628800.0);
}

/// Verifies Fibonacci via loop: fib(10) = 55.
#[test]
fn arith_fibonacci_loop() {
    ShapeTest::new(
        "fn fib(n: int) -> int {
            let mut a = 0
            let mut b = 1
            let mut i = 0
            while i < n {
                let temp = a + b
                a = b
                b = temp
                i += 1
            }
            return a
        }
fib(10)",
    )
    .expect_number(55.0);
}

// =====================================================================
// 20. COMPLEX COMBINED EXPRESSIONS
// =====================================================================

/// Verifies quadratic discriminant: b^2 - 4ac where a=1,b=5,c=6 => 1.
#[test]
fn complex_quadratic_discriminant() {
    ShapeTest::new(
        "let a = 1
let b = 5
let c = 6
b ** 2 - 4 * a * c",
    )
    .expect_number(1.0);
}

/// Verifies distance squared: (3-0)^2 + (4-0)^2 = 25.
#[test]
fn complex_distance_squared() {
    ShapeTest::new(
        "let x1 = 0
let y1 = 0
let x2 = 3
let y2 = 4
(x2 - x1) ** 2 + (y2 - y1) ** 2",
    )
    .expect_number(25.0);
}

/// Verifies Celsius to Fahrenheit: 100C = 212F.
#[test]
fn complex_celsius_to_fahrenheit() {
    ShapeTest::new(
        "let celsius = 100
celsius * 9 / 5 + 32",
    )
    .expect_number(212.0);
}

/// Verifies average of three: (10 + 20 + 30) / 3 = 20.
#[test]
fn complex_average_of_three() {
    ShapeTest::new(
        "let a = 10
let b = 20
let c = 30
(a + b + c) / 3",
    )
    .expect_number(20.0);
}

/// Verifies power of sum: (2 + 3) ** 2 = 25.
#[test]
fn complex_power_of_sum() {
    ShapeTest::new("(2 + 3) ** 2").expect_number(25.0);
}

/// Verifies complex expression with parens.
#[test]
fn complex_expression_with_parens() {
    ShapeTest::new("(2 + 3) * (4 - 1) + 6 / 2").expect_number(18.0);
}

/// Verifies deeply nested parentheses in addition.
#[test]
fn deeply_nested_parens() {
    ShapeTest::new("((((1 + 1) + 1) + 1) + 1)").expect_number(5.0);
}

// =====================================================================
// 21. COMPOUND ASSIGNMENT IN LOOPS
// =====================================================================

/// Verifies accumulation in loop: sum 0..99 = 4950.
#[test]
fn compound_assign_accumulate_loop() {
    ShapeTest::new(
        "fn accumulate() -> int {
            let mut sum = 0
            let mut i = 0
            while i < 100 {
                sum += i
                i += 1
            }
            return sum
        }
accumulate()",
    )
    .expect_number(4950.0);
}

/// Verifies power via loop: 2^10 = 1024.
#[test]
fn compound_assign_power_loop() {
    ShapeTest::new(
        "fn power_loop() -> int {
            let mut result = 1
            let mut i = 0
            while i < 10 {
                result *= 2
                i += 1
            }
            return result
        }
power_loop()",
    )
    .expect_number(1024.0);
}

// =====================================================================
// 22. ADDITIONAL MISC TESTS
// =====================================================================

/// Verifies all ops combined: (2+3)*4 - 10/2 + 7%3 = 16.
#[test]
fn misc_all_ops_combined() {
    ShapeTest::new("(2 + 3) * 4 - 10 / 2 + 7 % 3").expect_number(16.0);
}

/// Verifies identity operations combined.
#[test]
fn misc_identity_operations_combined() {
    ShapeTest::new(
        "let x = 42
(x + 0 - 0) * 1 / 1",
    )
    .expect_number(42.0);
}

/// Verifies expression used as let initializer.
#[test]
fn misc_expression_as_let_init() {
    ShapeTest::new(
        "let x = 2 * 3 + 4
x",
    )
    .expect_number(10.0);
}

/// Verifies nested function calls with arithmetic.
#[test]
fn misc_nested_function_arithmetic() {
    ShapeTest::new(
        "fn square(x: int) -> int {
            return x * x
        }
fn sum_of_squares(a: int, b: int) -> int {
            return square(a) + square(b)
        }
sum_of_squares(3, 4)",
    )
    .expect_number(25.0);
}

/// Verifies many locals in arithmetic.
#[test]
fn misc_many_locals_arithmetic() {
    ShapeTest::new(
        "fn many_locals() -> int {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            let f = 6
            let g = 7
            let h = 8
            return a + b + c + d + e + f + g + h
        }
many_locals()",
    )
    .expect_number(36.0);
}
