//! Stress tests for equality and inequality operators.
//!
//! Migrated from shape-vm stress_03_comparison.rs — equality (==), inequality (!=),
//! null equality, cross-type, computed value, and string equality sections.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 1. Equality (==) — integers
// ============================================================================

/// Verifies 1 == 1 is true.
#[test]
fn eq_int_same() {
    ShapeTest::new("1 == 1").expect_bool(true);
}

/// Verifies 1 == 2 is false.
#[test]
fn eq_int_different() {
    ShapeTest::new("1 == 2").expect_bool(false);
}

/// Verifies 0 == 0 is true.
#[test]
fn eq_int_zero() {
    ShapeTest::new("0 == 0").expect_bool(true);
}

/// Verifies -1 == -1 is true.
#[test]
fn eq_int_negative() {
    ShapeTest::new("-1 == -1").expect_bool(true);
}

/// Verifies -1 == 1 is false.
#[test]
fn eq_int_negative_vs_positive() {
    ShapeTest::new("-1 == 1").expect_bool(false);
}

/// Verifies large integer equality.
#[test]
fn eq_int_large() {
    ShapeTest::new("999999 == 999999").expect_bool(true);
}

/// Verifies large integer inequality.
#[test]
fn eq_int_large_different() {
    ShapeTest::new("999999 == 999998").expect_bool(false);
}

// ============================================================================
// 2. Equality (==) — floats / numbers
// ============================================================================

/// Verifies float equality with same values.
#[test]
fn eq_number_same() {
    ShapeTest::new("3.14 == 3.14").expect_bool(true);
}

/// Verifies float equality with different values.
#[test]
fn eq_number_different() {
    ShapeTest::new("3.14 == 2.71").expect_bool(false);
}

/// Verifies 0.0 == 0.0.
#[test]
fn eq_number_zero() {
    ShapeTest::new("0.0 == 0.0").expect_bool(true);
}

/// Verifies negative float equality.
#[test]
fn eq_number_negative() {
    ShapeTest::new("-1.5 == -1.5").expect_bool(true);
}

/// Verifies int vs number coercion: 1 == 1.0.
#[test]
fn eq_int_number_coercion() {
    ShapeTest::new("1 == 1.0").expect_bool(true);
}

/// Verifies int vs number: 1 == 1.5 is false.
#[test]
fn eq_int_number_not_equal() {
    ShapeTest::new("1 == 1.5").expect_bool(false);
}

// ============================================================================
// 3. Equality (==) — booleans
// ============================================================================

/// Verifies true == true.
#[test]
fn eq_bool_true_true() {
    ShapeTest::new("true == true").expect_bool(true);
}

/// Verifies false == false.
#[test]
fn eq_bool_false_false() {
    ShapeTest::new("false == false").expect_bool(true);
}

/// Verifies true == false is false.
#[test]
fn eq_bool_true_false() {
    ShapeTest::new("true == false").expect_bool(false);
}

/// Verifies false == true is false.
#[test]
fn eq_bool_false_true() {
    ShapeTest::new("false == true").expect_bool(false);
}

// ============================================================================
// 4. Equality (==) — strings
// ============================================================================

/// Verifies string equality with same values.
#[test]
fn eq_string_same() {
    ShapeTest::new(r#""hello" == "hello""#).expect_bool(true);
}

/// Verifies string equality with different values.
#[test]
fn eq_string_different() {
    ShapeTest::new(r#""hello" == "world""#).expect_bool(false);
}

/// Verifies empty string equality.
#[test]
fn eq_string_empty() {
    ShapeTest::new(r#""" == """#).expect_bool(true);
}

/// Verifies empty vs non-empty string.
#[test]
fn eq_string_empty_vs_nonempty() {
    ShapeTest::new(r#""" == "a""#).expect_bool(false);
}

// ============================================================================
// 5. Equality (==) — null
// ============================================================================

/// Verifies None == None.
#[test]
fn eq_null_null() {
    ShapeTest::new("None == None").expect_bool(true);
}

/// Verifies None != 0 (no implicit coercion).
#[test]
fn eq_null_vs_int() {
    ShapeTest::new("None == 0").expect_bool(false);
}

/// Verifies None != false (strict typing).
#[test]
fn eq_null_vs_false() {
    ShapeTest::new("None == false").expect_bool(false);
}

/// Verifies None != "".
#[test]
fn eq_null_vs_empty_string() {
    ShapeTest::new(r#"None == """#).expect_bool(false);
}

/// Verifies 1 == None is false.
#[test]
fn eq_int_vs_null() {
    ShapeTest::new("1 == None").expect_bool(false);
}

// ============================================================================
// 6. Inequality (!=)
// ============================================================================

/// Verifies 1 != 1 is false.
#[test]
fn neq_int_same() {
    ShapeTest::new("1 != 1").expect_bool(false);
}

/// Verifies 1 != 2 is true.
#[test]
fn neq_int_different() {
    ShapeTest::new("1 != 2").expect_bool(true);
}

/// Verifies float inequality with same values.
#[test]
fn neq_number_same() {
    ShapeTest::new("3.14 != 3.14").expect_bool(false);
}

/// Verifies float inequality with different values.
#[test]
fn neq_number_different() {
    ShapeTest::new("3.14 != 2.71").expect_bool(true);
}

/// Verifies bool inequality with same values.
#[test]
fn neq_bool_same() {
    ShapeTest::new("true != true").expect_bool(false);
}

/// Verifies bool inequality with different values.
#[test]
fn neq_bool_different() {
    ShapeTest::new("true != false").expect_bool(true);
}

/// Verifies string inequality with same values.
#[test]
fn neq_string_same() {
    ShapeTest::new(r#""abc" != "abc""#).expect_bool(false);
}

/// Verifies string inequality with different values.
#[test]
fn neq_string_different() {
    ShapeTest::new(r#""abc" != "xyz""#).expect_bool(true);
}

/// Verifies None != None is false.
#[test]
fn neq_null_null() {
    ShapeTest::new("None != None").expect_bool(false);
}

/// Verifies None != 1 is true.
#[test]
fn neq_null_vs_int() {
    ShapeTest::new("None != 1").expect_bool(true);
}

/// Verifies None != false is true.
#[test]
fn neq_null_vs_false() {
    ShapeTest::new("None != false").expect_bool(true);
}

// ============================================================================
// 20. Cross-type comparisons (int vs float)
// ============================================================================

/// Verifies int vs number: 1 < 2.5.
#[test]
fn lt_int_vs_number() {
    ShapeTest::new("1 < 2.5").expect_bool(true);
}

/// Verifies int vs number: 3 > 2.5.
#[test]
fn gt_int_vs_number() {
    ShapeTest::new("3 > 2.5").expect_bool(true);
}

/// Verifies int vs number: 2 <= 2.0.
#[test]
fn lte_int_vs_number_equal() {
    ShapeTest::new("2 <= 2.0").expect_bool(true);
}

/// Verifies int vs number: 2 >= 2.0.
#[test]
fn gte_int_vs_number_equal() {
    ShapeTest::new("2 >= 2.0").expect_bool(true);
}

// ============================================================================
// 21. Edge cases for number comparisons
// ============================================================================

/// Verifies 0.0 == -0.0 in IEEE 754.
#[test]
fn eq_zero_negative_zero() {
    ShapeTest::new("0.0 == -0.0").expect_bool(true);
}

/// Verifies very small difference comparison.
#[test]
fn lt_very_small_difference() {
    ShapeTest::new("1.0000000001 < 1.0000000002").expect_bool(true);
}

/// Verifies large integer comparison.
#[test]
fn gt_very_large_int() {
    ShapeTest::new("100000000 > 99999999").expect_bool(true);
}

/// Verifies adjacent large integers are not equal.
#[test]
fn eq_max_safe_int_adjacent() {
    ShapeTest::new("100000000000 == 100000000001").expect_bool(false);
}

// ============================================================================
// 22. NOT applied to comparison results
// ============================================================================

/// Verifies !(1 < 2) is false.
#[test]
fn not_comparison_true() {
    ShapeTest::new("!(1 < 2)").expect_bool(false);
}

/// Verifies !(2 < 1) is true.
#[test]
fn not_comparison_false() {
    ShapeTest::new("!(2 < 1)").expect_bool(true);
}

/// Verifies !(1 == 1) is false.
#[test]
fn not_equality() {
    ShapeTest::new("!(1 == 1)").expect_bool(false);
}

/// Verifies !(1 != 1) is true.
#[test]
fn not_inequality() {
    ShapeTest::new("!(1 != 1)").expect_bool(true);
}

// ============================================================================
// 23. Fuzzy comparison (~=)
// ============================================================================

/// Verifies default 2% tolerance: 100 ~= 102 is true.
#[test]
fn fuzzy_eq_default_tolerance() {
    ShapeTest::new("function test() { return 100 ~= 102; }\ntest()").expect_bool(true);
}

/// Verifies outside default tolerance: 100 ~= 105 is false.
#[test]
fn fuzzy_eq_outside_default_tolerance() {
    ShapeTest::new("function test() { return 100 ~= 105; }\ntest()").expect_bool(false);
}

/// Verifies absolute tolerance: 100 ~= 105 within 10 is true.
#[test]
fn fuzzy_eq_absolute_tolerance() {
    ShapeTest::new("function test() { return 100 ~= 105 within 10; }\ntest()").expect_bool(true);
}

/// Verifies absolute tolerance out of range.
#[test]
fn fuzzy_eq_absolute_tolerance_false() {
    ShapeTest::new("function test() { return 100 ~= 200 within 5; }\ntest()").expect_bool(false);
}

/// Verifies percentage tolerance.
#[test]
fn fuzzy_eq_percentage_tolerance() {
    ShapeTest::new("function test() { return 100 ~= 110 within 15%; }\ntest()").expect_bool(true);
}

// ============================================================================
// 26. Equality of expressions involving arithmetic
// ============================================================================

/// Verifies 2 + 3 == 5.
#[test]
fn eq_arithmetic_result() {
    ShapeTest::new("2 + 3 == 5").expect_bool(true);
}

/// Verifies 2 + 3 == 6 is false.
#[test]
fn eq_arithmetic_result_false() {
    ShapeTest::new("2 + 3 == 6").expect_bool(false);
}

/// Verifies 2 * 3 < 7.
#[test]
fn lt_arithmetic_result() {
    ShapeTest::new("2 * 3 < 7").expect_bool(true);
}

/// Verifies 10 - 3 > 5.
#[test]
fn gt_arithmetic_result() {
    ShapeTest::new("10 - 3 > 5").expect_bool(true);
}

// ============================================================================
// 27. String comparison edge cases
// ============================================================================

/// Verifies string equality is case-sensitive.
#[test]
fn eq_string_case_sensitive() {
    ShapeTest::new(r#""Hello" == "hello""#).expect_bool(false);
}

/// Verifies uppercase < lowercase in ASCII.
#[test]
fn lt_string_uppercase_vs_lowercase() {
    ShapeTest::new(r#""A" < "a""#).expect_bool(true);
}

/// Verifies z > a.
#[test]
fn gt_string_z_vs_a() {
    ShapeTest::new(r#""z" > "a""#).expect_bool(true);
}

/// Verifies string equality with spaces.
#[test]
fn eq_string_with_spaces() {
    ShapeTest::new(r#""a b" == "a b""#).expect_bool(true);
}

/// Verifies string inequality with trailing space.
#[test]
fn neq_string_with_trailing_space() {
    ShapeTest::new(r#""a" != "a ""#).expect_bool(true);
}

// ============================================================================
// 28. Cross-type inequality errors
// ============================================================================

/// Verifies string < int fails at runtime.
#[test]
fn lt_string_vs_int_fails() {
    ShapeTest::new(r#"function test() { return "abc" < 1; } test()"#).expect_run_err();
}

/// Verifies bool > int fails at runtime.
#[test]
fn gt_bool_vs_int_fails() {
    ShapeTest::new("function test() { return true > 1; } test()").expect_run_err();
}

// ============================================================================
// 31. Equality with computed values
// ============================================================================

/// Verifies equality of concatenated strings.
#[test]
fn eq_computed_strings() {
    ShapeTest::new(
        r#"function test() {
            let a = "hel" + "lo"
            let b = "hello"
            return a == b
        }
test()"#,
    )
    .expect_bool(true);
}

/// Verifies inequality of computed ints.
#[test]
fn neq_computed_ints() {
    ShapeTest::new(
        "function test() {
            let a = 2 * 5
            let b = 3 * 3
            return a != b
        }\ntest()",
    )
    .expect_bool(true);
}

// ============================================================================
// 33. Negation + comparison combined
// ============================================================================

/// Verifies -10 < -5 is true.
#[test]
fn negative_number_comparison() {
    ShapeTest::new("-10 < -5").expect_bool(true);
}

/// Verifies -7 == -7.
#[test]
fn negative_number_eq() {
    ShapeTest::new("-7 == -7").expect_bool(true);
}

/// Verifies -1 > -2.
#[test]
fn negative_number_gt() {
    ShapeTest::new("-1 > -2").expect_bool(true);
}

// ============================================================================
// 36. String equality with special characters
// ============================================================================

/// Verifies string with newline equality.
#[test]
fn eq_string_with_newline() {
    ShapeTest::new("function test() { return \"a\\nb\" == \"a\\nb\"; }\ntest()").expect_bool(true);
}

/// Verifies string with tab vs space are not equal.
#[test]
fn neq_string_with_tab_vs_space() {
    ShapeTest::new("function test() { return \"a\\tb\" != \"a b\"; }\ntest()").expect_bool(true);
}

// ============================================================================
// 37. Null propagation / null safety
// ============================================================================

/// Verifies None != 0 (explicit).
#[test]
fn null_not_equal_to_zero() {
    ShapeTest::new("None != 0").expect_bool(true);
}

/// Verifies None != "" (explicit).
#[test]
fn null_not_equal_to_empty_string() {
    ShapeTest::new(r#"None != """#).expect_bool(true);
}

/// Verifies None != false (explicit).
#[test]
fn null_not_equal_to_false_explicit() {
    ShapeTest::new("None != false").expect_bool(true);
}

/// Verifies None == None (explicit).
#[test]
fn null_equal_to_null_explicit() {
    ShapeTest::new("None == None").expect_bool(true);
}

// ============================================================================
// 39. Additional edge cases
// ============================================================================

/// Verifies (true == true) == true.
#[test]
fn eq_bool_eq_bool() {
    ShapeTest::new("(true == true) == true").expect_bool(true);
}

/// Verifies comparison returns bool not int.
#[test]
fn comparison_returns_bool_not_int() {
    ShapeTest::new("1 < 2").expect_bool(true);
}

/// Verifies 0 == 0 in integer domain.
#[test]
fn eq_negative_zero_and_zero_int() {
    ShapeTest::new("0 == 0").expect_bool(true);
}

/// Verifies 42 != 43.
#[test]
fn neq_adjacent_ints() {
    ShapeTest::new("42 != 43").expect_bool(true);
}

/// Verifies 7.7 <= 7.7.
#[test]
fn lte_equal_numbers() {
    ShapeTest::new("7.7 <= 7.7").expect_bool(true);
}

/// Verifies 7.7 >= 7.7.
#[test]
fn gte_equal_numbers() {
    ShapeTest::new("7.7 >= 7.7").expect_bool(true);
}
