//! Stress tests for ordering operators (<, >, <=, >=).
//!
//! Migrated from shape-vm stress_03_comparison.rs — less-than, greater-than,
//! less-or-equal, greater-or-equal, chained comparisons, null coalescing,
//! and comparison in control flow sections.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 7. Less than (<)
// ============================================================================

/// Verifies 1 < 2 is true.
#[test]
fn lt_int_true() {
    ShapeTest::new("1 < 2").expect_bool(true);
}

/// Verifies 2 < 2 is false (equal, not less).
#[test]
fn lt_int_false_equal() {
    ShapeTest::new("2 < 2").expect_bool(false);
}

/// Verifies 3 < 2 is false (greater).
#[test]
fn lt_int_false_greater() {
    ShapeTest::new("3 < 2").expect_bool(false);
}

/// Verifies -5 < -1 is true.
#[test]
fn lt_int_negative() {
    ShapeTest::new("-5 < -1").expect_bool(true);
}

/// Verifies -1 < 1 is true.
#[test]
fn lt_int_negative_vs_positive() {
    ShapeTest::new("-1 < 1").expect_bool(true);
}

/// Verifies 1.5 < 2.5 is true.
#[test]
fn lt_number_true() {
    ShapeTest::new("1.5 < 2.5").expect_bool(true);
}

/// Verifies 2.5 < 1.5 is false.
#[test]
fn lt_number_false() {
    ShapeTest::new("2.5 < 1.5").expect_bool(false);
}

/// Verifies 1.5 < 1.5 is false.
#[test]
fn lt_number_equal() {
    ShapeTest::new("1.5 < 1.5").expect_bool(false);
}

/// Verifies string lexicographic ordering.
#[test]
fn lt_string_lexicographic() {
    ShapeTest::new(r#""abc" < "abd""#).expect_bool(true);
}

/// Verifies same string is not less than itself.
#[test]
fn lt_string_same() {
    ShapeTest::new(r#""abc" < "abc""#).expect_bool(false);
}

/// Verifies shorter prefix is less than longer string.
#[test]
fn lt_string_shorter_prefix() {
    ShapeTest::new(r#""ab" < "abc""#).expect_bool(true);
}

/// Verifies longer string is not less than shorter prefix.
#[test]
fn lt_string_longer_vs_shorter() {
    ShapeTest::new(r#""abc" < "ab""#).expect_bool(false);
}

/// Verifies empty string is less than any non-empty string.
#[test]
fn lt_string_empty_vs_nonempty() {
    ShapeTest::new(r#""" < "a""#).expect_bool(true);
}

// ============================================================================
// 8. Greater than (>)
// ============================================================================

/// Verifies 3 > 2 is true.
#[test]
fn gt_int_true() {
    ShapeTest::new("3 > 2").expect_bool(true);
}

/// Verifies 2 > 2 is false.
#[test]
fn gt_int_false_equal() {
    ShapeTest::new("2 > 2").expect_bool(false);
}

/// Verifies 1 > 2 is false.
#[test]
fn gt_int_false_less() {
    ShapeTest::new("1 > 2").expect_bool(false);
}

/// Verifies -1 > -5 is true.
#[test]
fn gt_int_negative() {
    ShapeTest::new("-1 > -5").expect_bool(true);
}

/// Verifies 3.14 > 2.71 is true.
#[test]
fn gt_number_true() {
    ShapeTest::new("3.14 > 2.71").expect_bool(true);
}

/// Verifies 2.71 > 3.14 is false.
#[test]
fn gt_number_false() {
    ShapeTest::new("2.71 > 3.14").expect_bool(false);
}

/// Verifies string greater-than lexicographic.
#[test]
fn gt_string_lexicographic() {
    ShapeTest::new(r#""b" > "a""#).expect_bool(true);
}

/// Verifies string reverse order.
#[test]
fn gt_string_reverse() {
    ShapeTest::new(r#""a" > "b""#).expect_bool(false);
}

/// Verifies longer prefix string is greater.
#[test]
fn gt_string_longer_prefix() {
    ShapeTest::new(r#""abc" > "ab""#).expect_bool(true);
}

// ============================================================================
// 9. Less-or-equal (<=)
// ============================================================================

/// Verifies 1 <= 2 is true.
#[test]
fn lte_int_less() {
    ShapeTest::new("1 <= 2").expect_bool(true);
}

/// Verifies 2 <= 2 is true.
#[test]
fn lte_int_equal() {
    ShapeTest::new("2 <= 2").expect_bool(true);
}

/// Verifies 3 <= 2 is false.
#[test]
fn lte_int_greater() {
    ShapeTest::new("3 <= 2").expect_bool(false);
}

/// Verifies 1.0 <= 2.0.
#[test]
fn lte_number_less() {
    ShapeTest::new("1.0 <= 2.0").expect_bool(true);
}

/// Verifies 2.5 <= 2.5.
#[test]
fn lte_number_equal() {
    ShapeTest::new("2.5 <= 2.5").expect_bool(true);
}

/// Verifies 3.0 <= 2.0 is false.
#[test]
fn lte_number_greater() {
    ShapeTest::new("3.0 <= 2.0").expect_bool(false);
}

/// Verifies string less-or-equal.
#[test]
fn lte_string_less() {
    ShapeTest::new(r#""abc" <= "abd""#).expect_bool(true);
}

/// Verifies string less-or-equal with same string.
#[test]
fn lte_string_equal() {
    ShapeTest::new(r#""abc" <= "abc""#).expect_bool(true);
}

/// Verifies string less-or-equal false.
#[test]
fn lte_string_greater() {
    ShapeTest::new(r#""abd" <= "abc""#).expect_bool(false);
}

// ============================================================================
// 10. Greater-or-equal (>=)
// ============================================================================

/// Verifies 3 >= 2 is true.
#[test]
fn gte_int_greater() {
    ShapeTest::new("3 >= 2").expect_bool(true);
}

/// Verifies 2 >= 2 is true.
#[test]
fn gte_int_equal() {
    ShapeTest::new("2 >= 2").expect_bool(true);
}

/// Verifies 1 >= 2 is false.
#[test]
fn gte_int_less() {
    ShapeTest::new("1 >= 2").expect_bool(false);
}

/// Verifies 3.14 >= 2.71.
#[test]
fn gte_number_greater() {
    ShapeTest::new("3.14 >= 2.71").expect_bool(true);
}

/// Verifies 2.71 >= 2.71.
#[test]
fn gte_number_equal() {
    ShapeTest::new("2.71 >= 2.71").expect_bool(true);
}

/// Verifies 2.0 >= 3.0 is false.
#[test]
fn gte_number_less() {
    ShapeTest::new("2.0 >= 3.0").expect_bool(false);
}

/// Verifies string greater-or-equal.
#[test]
fn gte_string_greater() {
    ShapeTest::new(r#""b" >= "a""#).expect_bool(true);
}

/// Verifies string greater-or-equal with same string.
#[test]
fn gte_string_equal() {
    ShapeTest::new(r#""abc" >= "abc""#).expect_bool(true);
}

/// Verifies string greater-or-equal false.
#[test]
fn gte_string_less() {
    ShapeTest::new(r#""a" >= "b""#).expect_bool(false);
}

// ============================================================================
// 15. Chained comparisons (a < b && b < c)
// ============================================================================

/// Verifies chained less-than with AND.
#[test]
fn chained_lt_and() {
    ShapeTest::new(
        "function test() {
            let a = 1
            let b = 2
            let c = 3
            return a < b && b < c
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies chained less-than with AND where second is false.
#[test]
fn chained_lt_and_false() {
    ShapeTest::new(
        "function test() {
            let a = 1
            let b = 5
            let c = 3
            return a < b && b < c
        }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies range check: x >= 1 && x <= 10.
#[test]
fn chained_gte_and_lte() {
    ShapeTest::new(
        "function test() {
            let x = 5
            return x >= 1 && x <= 10
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies range check outside range.
#[test]
fn chained_gte_and_lte_out_of_range() {
    ShapeTest::new(
        "function test() {
            let x = 15
            return x >= 1 && x <= 10
        }\ntest()",
    )
    .expect_bool(false);
}

// ============================================================================
// 18. Comparison results used in expressions
// ============================================================================

/// Verifies comparison result stored in variable.
#[test]
fn comparison_result_in_variable() {
    ShapeTest::new(
        "function test() {
            let x = 5 > 3
            return x
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies comparison result in if statement.
#[test]
fn comparison_result_in_if() {
    ShapeTest::new(
        "function test() {
            let a = 10
            let b = 20
            if a < b {
                return 1
            } else {
                return 0
            }
        }\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies comparison used in conditional assignment.
#[test]
fn comparison_used_in_conditional_assignment() {
    ShapeTest::new(
        r#"function test() {
            let x = 5
            let result = if x > 3 { "big" } else { "small" }
            return result
        }
test()"#,
    )
    .expect_string("big");
}

// ============================================================================
// 19. Null coalescing (??)
// ============================================================================

/// Verifies null coalescing with None left.
#[test]
fn null_coalesce_null_left() {
    ShapeTest::new(
        "function test() {
            let x = None
            return x ?? 42
        }\ntest()",
    )
    .expect_number(42.0);
}

/// Verifies null coalescing with non-null left.
#[test]
fn null_coalesce_non_null_left() {
    ShapeTest::new(
        "function test() {
            let x = 10
            return x ?? 42
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies null coalescing with string fallback.
#[test]
fn null_coalesce_string() {
    ShapeTest::new(
        r#"function test() {
            let x = None
            return x ?? "default"
        }
test()"#,
    )
    .expect_string("default");
}

/// Verifies chained null coalescing.
#[test]
fn null_coalesce_chain() {
    ShapeTest::new(
        "function test() {
            let a = None
            let b = None
            return a ?? b ?? 99
        }\ntest()",
    )
    .expect_number(99.0);
}

/// Verifies first non-null in coalescing chain.
#[test]
fn null_coalesce_first_non_null() {
    ShapeTest::new(
        "function test() {
            let a = None
            let b = 7
            return a ?? b ?? 99
        }\ntest()",
    )
    .expect_number(7.0);
}

// ============================================================================
// 25. Comparison in loops / iterative contexts
// ============================================================================

/// Verifies comparison in while loop.
#[test]
fn comparison_in_while_loop() {
    ShapeTest::new(
        "function test() {
            let mut i = 0
            while i < 10 {
                i = i + 1
            }
            return i
        }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies comparison in for loop with break.
#[test]
fn comparison_in_for_loop_with_break() {
    ShapeTest::new(
        "function test() {
            let mut result = 0
            for i in range(0, 100) {
                if i >= 5 {
                    break
                }
                result = result + 1
            }
            return result
        }\ntest()",
    )
    .expect_number(5.0);
}

// ============================================================================
// 30. Comparison in function return values
// ============================================================================

/// Verifies function returning comparison result.
#[test]
fn function_returns_comparison() {
    ShapeTest::new(
        "function is_positive(n: int) -> bool {
            return n > 0
        }
function test() {
            return is_positive(5)
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies function returning comparison result false.
#[test]
fn function_returns_comparison_false() {
    ShapeTest::new(
        "function is_positive(n: int) -> bool {
            return n > 0
        }
function test() {
            return is_positive(-3)
        }\ntest()",
    )
    .expect_bool(false);
}

/// Verifies function returning logical result.
#[test]
fn function_returns_logical() {
    ShapeTest::new(
        "function in_range(x: int, lo: int, hi: int) -> bool {
            return x >= lo && x <= hi
        }
function test() {
            return in_range(5, 1, 10)
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies function returning logical result false.
#[test]
fn function_returns_logical_false() {
    ShapeTest::new(
        "function in_range(x: int, lo: int, hi: int) -> bool {
            return x >= lo && x <= hi
        }
function test() {
            return in_range(15, 1, 10)
        }\ntest()",
    )
    .expect_bool(false);
}

// ============================================================================
// 34. Comparison stability
// ============================================================================

/// Verifies same comparison yields same result 100 times.
#[test]
fn comparison_stability_loop() {
    ShapeTest::new(
        "function test() {
            let mut count = 0
            for i in range(0, 100) {
                if 5 > 3 {
                    count = count + 1
                }
            }
            return count
        }\ntest()",
    )
    .expect_number(100.0);
}

// ============================================================================
// 38. Comparison results chained via variables
// ============================================================================

/// Verifies variable-based chained comparisons.
#[test]
fn variable_based_chained_comparison() {
    ShapeTest::new(
        "function test() {
            let a_gt_b = 10 > 5
            let c_lt_d = 3 < 7
            return a_gt_b && c_lt_d
        }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies variable-based negated comparison.
#[test]
fn variable_based_negated_comparison() {
    ShapeTest::new(
        "function test() {
            let result = 5 == 5
            return !result
        }\ntest()",
    )
    .expect_bool(false);
}
