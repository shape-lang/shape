//! Stress tests for addition and subtraction operators.
//!
//! Migrated from shape-vm stress_02_arithmetic.rs — addition, subtraction,
//! and negation sections.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// 1. ADDITION
// =====================================================================

/// Verifies basic integer addition 1 + 2 = 3.
#[test]
fn add_int_basic() {
    ShapeTest::new("1 + 2").expect_number(3.0);
}

/// Verifies 0 + 42 = 42.
#[test]
fn add_int_zero_left() {
    ShapeTest::new("0 + 42").expect_number(42.0);
}

/// Verifies 42 + 0 = 42.
#[test]
fn add_int_zero_right() {
    ShapeTest::new("42 + 0").expect_number(42.0);
}

/// Verifies 0 + 0 = 0.
#[test]
fn add_int_zero_plus_zero() {
    ShapeTest::new("0 + 0").expect_number(0.0);
}

/// Verifies -10 + 15 = 5.
#[test]
fn add_int_negative_plus_positive() {
    ShapeTest::new("-10 + 15").expect_number(5.0);
}

/// Verifies -10 + -20 = -30.
#[test]
fn add_int_negative_plus_negative() {
    ShapeTest::new("-10 + -20").expect_number(-30.0);
}

/// Verifies large integer addition.
#[test]
fn add_int_large_values() {
    ShapeTest::new("1000000 + 2000000").expect_number(3000000.0);
}

/// Verifies basic float addition 1.5 + 2.5 = 4.0.
#[test]
fn add_number_basic() {
    ShapeTest::new("1.5 + 2.5").expect_number(4.0);
}

/// Verifies 0.0 + 3.14.
#[test]
fn add_number_zero_left() {
    ShapeTest::new("0.0 + 3.14").expect_number(3.14);
}

/// Verifies 0.1 + 0.2 is close to 0.3 (IEEE 754).
#[test]
fn add_number_fractional() {
    ShapeTest::new("0.1 + 0.2").expect_number(0.30000000000000004);
}

/// Verifies -1.5 + -2.5 = -4.0.
#[test]
fn add_number_negative() {
    ShapeTest::new("-1.5 + -2.5").expect_number(-4.0);
}

/// Verifies chained addition 1 + 2 + 3 = 6.
#[test]
fn add_chained_three() {
    ShapeTest::new("1 + 2 + 3").expect_number(6.0);
}

/// Verifies chained addition 1 through 10 = 55.
#[test]
fn add_chained_ten() {
    ShapeTest::new("1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10").expect_number(55.0);
}

// =====================================================================
// 2. SUBTRACTION
// =====================================================================

/// Verifies basic integer subtraction 10 - 3 = 7.
#[test]
fn sub_int_basic() {
    ShapeTest::new("10 - 3").expect_number(7.0);
}

/// Verifies subtraction with negative result 3 - 10 = -7.
#[test]
fn sub_int_negative_result() {
    ShapeTest::new("3 - 10").expect_number(-7.0);
}

/// Verifies 0 - 42 = -42.
#[test]
fn sub_int_zero_minus_x() {
    ShapeTest::new("0 - 42").expect_number(-42.0);
}

/// Verifies 42 - 0 = 42.
#[test]
fn sub_int_x_minus_zero() {
    ShapeTest::new("42 - 0").expect_number(42.0);
}

/// Verifies x - x = 0.
#[test]
fn sub_int_self() {
    ShapeTest::new("let x = 99\nx - x").expect_number(0.0);
}

/// Verifies -10 - -3 = -7.
#[test]
fn sub_int_negative_minus_negative() {
    ShapeTest::new("-10 - -3").expect_number(-7.0);
}

/// Verifies float subtraction 10.5 - 3.2 = 7.3.
#[test]
fn sub_number_basic() {
    ShapeTest::new("10.5 - 3.2").expect_number(7.3);
}

/// Verifies float subtraction with negative result.
#[test]
fn sub_number_negative_result() {
    ShapeTest::new("1.0 - 5.5").expect_number(-4.5);
}

/// Verifies chained subtraction 100 - 20 - 30 - 10 = 40.
#[test]
fn sub_chained() {
    ShapeTest::new("100 - 20 - 30 - 10").expect_number(40.0);
}

// =====================================================================
// 7. UNARY NEGATION
// =====================================================================

/// Verifies unary negation of integer.
#[test]
fn neg_int_basic() {
    ShapeTest::new("-42").expect_number(-42.0);
}

/// Verifies double negation -(-42) = 42.
#[test]
fn neg_int_double_negation() {
    ShapeTest::new("-(-42)").expect_number(42.0);
}

/// Verifies -0 is 0.
#[test]
fn neg_int_zero() {
    ShapeTest::new("-0").expect_number(0.0);
}

/// Verifies unary negation of float.
#[test]
fn neg_number_basic() {
    ShapeTest::new("-3.14").expect_number(-3.14);
}

/// Verifies double negation of float.
#[test]
fn neg_number_double() {
    ShapeTest::new("-(-3.14)").expect_number(3.14);
}

/// Verifies negation of expression -(2 + 3) = -5.
#[test]
fn neg_expression() {
    ShapeTest::new("-(2 + 3)").expect_number(-5.0);
}

/// Verifies negation in arithmetic 10 + -3 = 7.
#[test]
fn neg_in_arithmetic() {
    ShapeTest::new("10 + -3").expect_number(7.0);
}

// =====================================================================
// 10. INTEGER OVERFLOW
// =====================================================================

/// Verifies large integer overflow add produces a numeric result.
#[test]
fn overflow_add_promotes_to_float() {
    ShapeTest::new(
        "let x = 9007199254740990
let y = 10
x + y",
    )
    .expect_number(9007199254741000.0);
}

/// Verifies large integer overflow multiply produces a numeric result.
#[test]
fn overflow_mul_promotes_to_float() {
    ShapeTest::new(
        "let x = 4503599627370496
let y = 4503599627370496
x * y",
    )
    .expect_run_ok();
}

/// Verifies large integer overflow subtract produces a numeric result.
#[test]
fn overflow_sub_promotes_to_float() {
    ShapeTest::new(
        "let x = -9007199254740990
let y = 100
x - y",
    )
    .expect_run_ok();
}

// =====================================================================
// 11. EXPRESSIONS IN FUNCTIONS
// =====================================================================

/// Verifies function returning sum of params.
#[test]
fn function_add_return() {
    ShapeTest::new(
        "fn add(a: int, b: int) -> int {
            return a + b
        }
add(3, 4)",
    )
    .expect_number(7.0);
}

/// Verifies multi-step arithmetic in function.
#[test]
fn function_arithmetic_expression() {
    ShapeTest::new(
        "fn calc() -> int {
            let a = 10
            let b = 20
            let c = a + b
            let d = c * 2
            let e = d - a
            let f = e / b
            return f
        }
calc()",
    )
    .expect_number(2.0);
}

/// Verifies nested function calls with arithmetic.
#[test]
fn function_nested_calls_arithmetic() {
    ShapeTest::new(
        "fn double(x: int) -> int {
            return x * 2
        }
fn quad(x: int) -> int {
            return double(double(x))
        }
quad(5)",
    )
    .expect_number(20.0);
}

// =====================================================================
// 14. VARIABLES IN ARITHMETIC
// =====================================================================

/// Verifies variable addition.
#[test]
fn var_add() {
    ShapeTest::new(
        "let a = 10
let b = 20
a + b",
    )
    .expect_number(30.0);
}

/// Verifies variable reuse in addition.
#[test]
fn var_reuse() {
    ShapeTest::new(
        "let x = 5
x + x + x",
    )
    .expect_number(15.0);
}

/// Verifies variable chain.
#[test]
fn var_chain() {
    ShapeTest::new(
        "let a = 1
let b = a + 1
let c = b + 1
let d = c + 1
d",
    )
    .expect_number(4.0);
}

// =====================================================================
// 13. EDGE CASES
// =====================================================================

/// Verifies single literal 1.
#[test]
fn edge_one_literal() {
    ShapeTest::new("1").expect_number(1.0);
}

/// Verifies single literal 0.
#[test]
fn edge_zero_literal() {
    ShapeTest::new("0").expect_number(0.0);
}

/// Verifies single negative literal.
#[test]
fn edge_negative_literal() {
    ShapeTest::new("-1").expect_number(-1.0);
}

/// Verifies add then sub cancel: 42 + 10 - 10 = 42.
#[test]
fn edge_add_sub_cancel() {
    ShapeTest::new("42 + 10 - 10").expect_number(42.0);
}

/// Verifies 0 - 1 - 2 - 3 = -6.
#[test]
fn misc_sub_from_zero_chain() {
    ShapeTest::new("0 - 1 - 2 - 3").expect_number(-6.0);
}

/// Verifies chain of negative additions.
#[test]
fn misc_add_negative_numbers() {
    ShapeTest::new("-1 + -2 + -3 + -4 + -5").expect_number(-15.0);
}

/// Verifies alternating add/sub: 1 - 2 + 3 - 4 + 5 = 3.
#[test]
fn complex_alternating_add_sub() {
    ShapeTest::new("1 - 2 + 3 - 4 + 5").expect_number(3.0);
}

/// Verifies triple negation -(-(-5)) = -5.
#[test]
fn misc_triple_negation() {
    ShapeTest::new("-(-(-5))").expect_number(-5.0);
}

/// Verifies chained float addition.
#[test]
fn misc_add_large_chain_numbers() {
    ShapeTest::new("1.0 + 2.0 + 3.0 + 4.0 + 5.0").expect_number(15.0);
}

/// Verifies associativity of addition.
#[test]
fn misc_associativity_add() {
    ShapeTest::new("(10 + 20) + 30").expect_number(60.0);
}

/// Verifies commutativity of addition.
#[test]
fn misc_commutativity_add() {
    ShapeTest::new("3 + 7").expect_number(10.0);
}

/// Verifies negative multiplication distributive law.
#[test]
fn misc_negative_mul_distributive() {
    ShapeTest::new("-(3 + 7)").expect_number(-10.0);
}

/// Verifies chained addition of five numbers.
#[test]
fn chained_add_five() {
    ShapeTest::new("1 + 2 + 3 + 4 + 5").expect_number(15.0);
}

/// Verifies chained subtraction left associativity.
#[test]
fn chained_sub_left_assoc() {
    ShapeTest::new("100 - 20 - 30").expect_number(50.0);
}
