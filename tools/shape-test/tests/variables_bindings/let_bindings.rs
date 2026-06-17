//! Tests for `let` immutable bindings.
//!
//! Covers: basic let, type-annotated let, multiple let, shadowing, const.

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use shape_test::shape_test::ShapeTest;

#[test]
fn let_binding_integer() {
    ShapeTest::new(
        r#"
        let x = 42
        print(x)
    "#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn let_binding_float() {
    ShapeTest::new(
        r#"
        let x = 3.14
        x
    "#,
    )
    .expect_number(3.14);
}

#[test]
fn let_binding_string() {
    ShapeTest::new(
        r#"
        let name = "hello"
        name
    "#,
    )
    .expect_string("hello");
}

#[test]
fn let_binding_bool() {
    ShapeTest::new(
        r#"
        let flag = true
        flag
    "#,
    )
    .expect_bool(true);
}

// TDD: type-annotated let bindings may not be fully enforced at runtime
#[test]
fn let_binding_type_annotated_int() {
    ShapeTest::new(
        r#"
        let x: int = 42
        x
    "#,
    )
    .expect_number(42.0);
}

// TDD: type-annotated let bindings may not be fully enforced at runtime
#[test]
fn let_binding_type_annotated_string() {
    ShapeTest::new(
        r#"
        let s: string = "world"
        s
    "#,
    )
    .expect_string("world");
}

#[test]
fn let_multiple_bindings() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        let c = 30
        a + b + c
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn let_shadowing_same_scope() {
    ShapeTest::new(
        r#"
        let x = 10
        let x = 20
        x
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn let_shadowing_different_type() {
    ShapeTest::new(
        r#"
        let x = 42
        let x = "now a string"
        x
    "#,
    )
    .expect_string("now a string");
}

// TDD: const keyword may not be supported
#[test]
fn const_binding() {
    ShapeTest::new(
        r#"
        const PI = 3.14159
        PI
    "#,
    )
    .expect_number(3.14159);
}

#[test]
fn let_binding_expression_result() {
    ShapeTest::new(
        r#"
        let x = 2 + 3 * 4
        x
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn let_immutable_cannot_reassign() {
    ShapeTest::new(
        r#"
        let x = 10
        x = 20
        x
    "#,
    )
    .expect_run_err_contains("immutable");
}

// Tuple types (book `fundamentals/variables` §Tuple Types): `[T1, T2, ...]`
// is the bracket-syntax tuple TYPE and a bracket literal `[v1, v2, ...]` is
// its value form. Elements are read by index, exactly as the chapter's
// runnable example shows.
#[test]
fn tuple_annotated_binding_indexes_by_position_first() {
    ShapeTest::new(
        r#"
        let pair: [int, int] = [3, 4]
        pair[0]
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn tuple_annotated_binding_indexes_by_position_second() {
    ShapeTest::new(
        r#"
        let pair: [int, int] = [3, 4]
        pair[1]
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn tuple_annotated_binding_sum_of_elements() {
    ShapeTest::new(
        r#"
        let pair: [int, int] = [3, 4]
        pair[0] + pair[1]
    "#,
    )
    .expect_number(7.0);
}

// Arity is fixed at compile time: a literal with the wrong count is rejected.
#[test]
fn tuple_annotated_binding_arity_mismatch_is_error() {
    ShapeTest::new(
        r#"
        let pair: [int, int] = [1, 2, 3]
        pair[0]
    "#,
    )
    .expect_run_err_contains("tuple");
}
