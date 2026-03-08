//! Logical operator tests.
//!
//! Covers: and, or, not (Shape uses words, not symbols for logical ops).

use shape_test::shape_test::ShapeTest;

#[test]
fn logical_and_true() {
    ShapeTest::new(
        r#"
        true and true
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_and_false() {
    ShapeTest::new(
        r#"
        true and false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_or_true() {
    ShapeTest::new(
        r#"
        false or true
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_or_both_false() {
    ShapeTest::new(
        r#"
        false or false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_not_true() {
    ShapeTest::new(
        r#"
        !true
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_not_false() {
    ShapeTest::new(
        r#"
        !false
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_and_short_circuit() {
    // If first operand is false, second should not matter
    ShapeTest::new(
        r#"
        false and true
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_or_short_circuit() {
    // If first operand is true, second should not matter
    ShapeTest::new(
        r#"
        true or false
    "#,
    )
    .expect_bool(true);
}

#[test]
fn compound_logical_expression() {
    ShapeTest::new(
        r#"
        let x = 5
        (x > 0 and x < 10) or x == 20
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_with_comparison() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 7
        a < b and b < 10
    "#,
    )
    .expect_bool(true);
}

#[test]
fn not_with_comparison() {
    ShapeTest::new(
        r#"
        !(5 > 10)
    "#,
    )
    .expect_bool(true);
}

// Shape also supports && and || as aliases
#[test]
fn logical_and_symbol_alias() {
    ShapeTest::new(
        r#"
        true && false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_or_symbol_alias() {
    ShapeTest::new(
        r#"
        false || true
    "#,
    )
    .expect_bool(true);
}
