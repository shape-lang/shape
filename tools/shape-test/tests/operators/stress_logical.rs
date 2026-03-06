//! Stress tests for logical operators (&&, ||, !, and, or).
//!
//! Migrated from shape-vm stress_03_comparison.rs — logical AND, OR, NOT,
//! short-circuit, precedence, truthiness, De Morgan's laws, and complex sections.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 11. Logical AND (&&)
// ============================================================================

/// Verifies true && true = true.
#[test]
fn and_true_true() {
    ShapeTest::new("true && true").expect_bool(true);
}

/// Verifies true && false = false.
#[test]
fn and_true_false() {
    ShapeTest::new("true && false").expect_bool(false);
}

/// Verifies false && true = false.
#[test]
fn and_false_true() {
    ShapeTest::new("false && true").expect_bool(false);
}

/// Verifies false && false = false.
#[test]
fn and_false_false() {
    ShapeTest::new("false && false").expect_bool(false);
}

// ============================================================================
// 12. Logical OR (||)
// ============================================================================

/// Verifies true || true = true.
#[test]
fn or_true_true() {
    ShapeTest::new("true || true").expect_bool(true);
}

/// Verifies true || false = true.
#[test]
fn or_true_false() {
    ShapeTest::new("true || false").expect_bool(true);
}

/// Verifies false || true = true.
#[test]
fn or_false_true() {
    ShapeTest::new("false || true").expect_bool(true);
}

/// Verifies false || false = false.
#[test]
fn or_false_false() {
    ShapeTest::new("false || false").expect_bool(false);
}

// ============================================================================
// 13. Logical NOT (!)
// ============================================================================

/// Verifies !true = false.
#[test]
fn not_true_is_false() {
    ShapeTest::new("!true").expect_bool(false);
}

/// Verifies !false = true.
#[test]
fn not_false() {
    ShapeTest::new("!false").expect_bool(true);
}

/// Verifies !!true = true.
#[test]
fn not_not_true() {
    ShapeTest::new("!!true").expect_bool(true);
}

/// Verifies !!false = false.
#[test]
fn not_not_false() {
    ShapeTest::new("!!false").expect_bool(false);
}

/// Verifies !!!true = false.
#[test]
fn not_not_not_true() {
    ShapeTest::new("!!!true").expect_bool(false);
}

// ============================================================================
// 14. Short-circuit evaluation
// ============================================================================

/// Verifies && short-circuits when left is false.
#[test]
fn and_short_circuit_result() {
    ShapeTest::new("false && true").expect_bool(false);
}

/// Verifies || short-circuits when left is true.
#[test]
fn or_short_circuit_result() {
    ShapeTest::new("true || false").expect_bool(true);
}

/// Verifies && evaluates right side when left is true.
#[test]
fn and_evaluates_right_when_left_true() {
    ShapeTest::new(
        "function get_false() -> bool {
            return false
        }
function test() {
            return true && get_false()
        }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies || evaluates right side when left is false.
#[test]
fn or_evaluates_right_when_left_false() {
    ShapeTest::new(
        "function get_true() -> bool {
            return true
        }
function test() {
            return false || get_true()
        }\ntest()",
    )
    .expect_bool(true);
}

// ============================================================================
// 16. Operator precedence (&&, ||, comparisons)
// ============================================================================

/// Verifies && binds tighter than ||.
#[test]
fn precedence_and_before_or() {
    ShapeTest::new("true || false && false").expect_bool(true);
}

/// Verifies && before || variant 2.
#[test]
fn precedence_and_before_or_2() {
    ShapeTest::new("false || true && true").expect_bool(true);
}

/// Verifies && before || variant 3.
#[test]
fn precedence_and_before_or_3() {
    ShapeTest::new("false && true || true").expect_bool(true);
}

/// Verifies all false with && and ||.
#[test]
fn precedence_and_before_or_all_false() {
    ShapeTest::new("false && false || false").expect_bool(false);
}

/// Verifies parentheses override default precedence.
#[test]
fn precedence_parens_override() {
    ShapeTest::new("(true || false) && false").expect_bool(false);
}

/// Verifies comparisons bind tighter than logical.
#[test]
fn precedence_comparison_before_logical() {
    ShapeTest::new("1 < 2 && 3 < 4").expect_bool(true);
}

/// Verifies comparisons bind tighter than ||.
#[test]
fn precedence_comparison_before_or() {
    ShapeTest::new("1 > 2 || 3 > 2").expect_bool(true);
}

// ============================================================================
// 17. Complex expressions
// ============================================================================

/// Verifies complex AND/OR with comparison.
#[test]
fn complex_and_or_comparison() {
    ShapeTest::new(
        "function test() {
            let a = 5
            let b = 10
            let c = 0
            return (a > 0 && b > 0) || c == 0
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies complex nested AND.
#[test]
fn complex_nested_and() {
    ShapeTest::new(
        "function test() {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            return a < b && b < c && c < d
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies complex nested OR with last true.
#[test]
fn complex_nested_or() {
    ShapeTest::new(
        "function test() {
            return false || false || false || true
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies all false OR chain.
#[test]
fn complex_all_false_or() {
    ShapeTest::new(
        "function test() {
            return false || false || false || false
        }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies mixed types in condition.
#[test]
fn complex_mixed_types_in_conditions() {
    ShapeTest::new(
        r#"function test() {
            let x = 42
            let name = "hello"
            return x > 0 && name == "hello"
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies mixed types in condition with false.
#[test]
fn complex_mixed_types_false() {
    ShapeTest::new(
        r#"function test() {
            let x = 42
            let name = "hello"
            return x > 100 && name == "hello"
        }
test()"#,
    )
    .expect_bool(false);
}

// ============================================================================
// 24. Comparison with boolean expressions
// ============================================================================

/// Verifies && with !.
#[test]
fn and_with_not() {
    ShapeTest::new("true && !false").expect_bool(true);
}

/// Verifies || with !.
#[test]
fn or_with_not() {
    ShapeTest::new("false || !false").expect_bool(true);
}

/// Verifies !(true && false).
#[test]
fn not_and() {
    ShapeTest::new("!(true && false)").expect_bool(true);
}

/// Verifies !(false || false).
#[test]
fn not_or() {
    ShapeTest::new("!(false || false)").expect_bool(true);
}

/// Verifies De Morgan's: !(a && b) == (!a || !b).
#[test]
fn demorgan_not_and_equiv() {
    ShapeTest::new(
        "function test() {
            let a = true
            let b = false
            return !(a && b) == (!a || !b)
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies De Morgan's: !(a || b) == (!a && !b).
#[test]
fn demorgan_not_or_equiv() {
    ShapeTest::new(
        "function test() {
            let a = false
            let b = false
            return !(a || b) == (!a && !b)
        }\ntest()",
    )
    .expect_bool(true);
}

// ============================================================================
// 29. Multi-step logical with variables
// ============================================================================

/// Verifies multi-step AND chain all true.
#[test]
fn multi_step_and_chain() {
    ShapeTest::new(
        "function test() {
            let a = true
            let b = true
            let c = true
            return a && b && c
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies multi-step AND chain one false.
#[test]
fn multi_step_and_chain_one_false() {
    ShapeTest::new(
        "function test() {
            let a = true
            let b = false
            let c = true
            return a && b && c
        }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies multi-step OR chain with last true.
#[test]
fn multi_step_or_chain() {
    ShapeTest::new(
        "function test() {
            let a = false
            let b = false
            let c = true
            return a || b || c
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies multi-step OR chain all false.
#[test]
fn multi_step_or_chain_all_false() {
    ShapeTest::new(
        "function test() {
            let a = false
            let b = false
            let c = false
            return a || b || c
        }\ntest()",
    )
    .expect_bool(false);
}

// ============================================================================
// 32. Truthiness in logical operators
// ============================================================================

/// Verifies 0 is falsy in && context.
#[test]
fn and_with_zero_int() {
    ShapeTest::new("0 && true").expect_bool(false);
}

/// Verifies non-zero int is truthy in && context.
#[test]
fn and_with_nonzero_int() {
    ShapeTest::new("1 && true").expect_bool(true);
}

/// Verifies 0 is falsy in || context.
#[test]
fn or_with_zero_int() {
    ShapeTest::new("0 || true").expect_bool(true);
}

/// Verifies non-zero int is truthy in || context.
#[test]
fn or_with_nonzero_int() {
    ShapeTest::new("1 || false").expect_bool(true);
}

/// Verifies !0 = true (zero is falsy).
#[test]
fn not_zero() {
    ShapeTest::new("!0").expect_bool(true);
}

/// Verifies !1 = false (one is truthy).
#[test]
fn not_one() {
    ShapeTest::new("!1").expect_bool(false);
}

/// Verifies !None = true (null is falsy).
#[test]
fn not_null() {
    ShapeTest::new("!None").expect_bool(true);
}

// ============================================================================
// 35. Deeply nested boolean expressions
// ============================================================================

/// Verifies deeply nested OR/AND.
#[test]
fn deeply_nested_or_and() {
    ShapeTest::new(
        "function test() {
            return ((true || false) && (false || true)) && !(false && true)
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies deeply nested parenthesized true.
#[test]
fn deeply_nested_all_parens() {
    ShapeTest::new(
        "function test() {
            return ((((true))))
        }\ntest()",
    )
    .expect_bool(true);
}
