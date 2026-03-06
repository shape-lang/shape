//! Basic type inference tests — integers, floats, strings, bools,
//! closures, function calls, and reassignment.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 1. Type Inference (20 tests)
// =========================================================================

#[test]
fn test_infer_int_basic() {
    ShapeTest::new(
        r#"
        let x = 42
        x + 1
    "#,
    )
    .expect_number(43.0);
}

#[test]
fn test_infer_int_negative() {
    ShapeTest::new(
        r#"
        let x = -10
        x + 20
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_infer_number_float() {
    ShapeTest::new(
        r#"
        let x = 3.14
        x * 2
    "#,
    )
    .expect_number(6.28);
}

#[test]
fn test_infer_number_float_division() {
    ShapeTest::new(
        r#"
        let x = 10.0
        x / 4.0
    "#,
    )
    .expect_number(2.5);
}

#[test]
fn test_infer_string_length() {
    ShapeTest::new(
        r#"
        let s = "hello"
        s.length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_infer_string_concatenation() {
    ShapeTest::new(
        r#"
        let s = "hello"
        s + " world"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_infer_bool_negation() {
    ShapeTest::new(
        r#"
        let b = true
        !b
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_infer_bool_from_comparison() {
    ShapeTest::new(
        r#"
        let x = 10
        let b = x > 5
        b
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_infer_array_length() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        a.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_infer_through_function_call() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        let result = double(21)
        result
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_infer_through_multiple_assignments() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = a + 5
        let c = b * 2
        c
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_infer_closure_return_type() {
    ShapeTest::new(
        r#"
        let f = |x| x * 3
        f(10)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_infer_closure_param_from_usage() {
    ShapeTest::new(
        r#"
        let nums = [1, 2, 3]
        let doubled = nums.map(|x| x * 2)
        doubled[0] + doubled[1] + doubled[2]
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_infer_operator_result_bool() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        a < b
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_infer_operator_result_arithmetic() {
    ShapeTest::new(
        r#"
        let a = 7
        let b = 3
        a % b
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_infer_mixed_expression_types() {
    ShapeTest::new(
        r#"
        let x = 10
        let y = 20
        let result = if x < y { "less" } else { "greater" }
        result
    "#,
    )
    .expect_string("less");
}

#[test]
fn test_infer_int_to_number_promotion() {
    // Integer arithmetic that stays integer
    ShapeTest::new(
        r#"
        let x = 10
        let y = 3
        x + y
    "#,
    )
    .expect_number(13.0);
}

#[test]
fn test_infer_nested_function_calls() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        fn double(x) { x * 2 }
        double(add(5, 10))
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_infer_let_from_match() {
    ShapeTest::new(
        r#"
        let x = 42
        let label = match x {
            42 => "the answer",
            _ => "other"
        }
        label
    "#,
    )
    .expect_string("the answer");
}

#[test]
fn test_infer_reassignment_preserves_type() {
    ShapeTest::new(
        r#"
        var x = 10
        x = x + 5
        x = x * 2
        x
    "#,
    )
    .expect_number(30.0);
}
