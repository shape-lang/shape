//! Comparison operator tests.
//!
//! Covers: ==, !=, <, >, <=, >=, string comparison, bool comparison.

use shape_test::shape_test::ShapeTest;

#[test]
fn equal_integers() {
    ShapeTest::new(
        r#"
        5 == 5
    "#,
    )
    .expect_bool(true);
}

#[test]
fn not_equal_integers() {
    ShapeTest::new(
        r#"
        5 != 3
    "#,
    )
    .expect_bool(true);
}

#[test]
fn less_than() {
    ShapeTest::new(
        r#"
        3 < 5
    "#,
    )
    .expect_bool(true);
}

#[test]
fn greater_than() {
    ShapeTest::new(
        r#"
        10 > 3
    "#,
    )
    .expect_bool(true);
}

#[test]
fn less_than_or_equal() {
    ShapeTest::new(
        r#"
        5 <= 5
    "#,
    )
    .expect_bool(true);
}

#[test]
fn greater_than_or_equal() {
    ShapeTest::new(
        r#"
        7 >= 3
    "#,
    )
    .expect_bool(true);
}

#[test]
fn equal_false() {
    ShapeTest::new(
        r#"
        5 == 3
    "#,
    )
    .expect_bool(false);
}

#[test]
fn not_equal_false() {
    ShapeTest::new(
        r#"
        5 != 5
    "#,
    )
    .expect_bool(false);
}

#[test]
fn string_equality() {
    ShapeTest::new(
        r#"
        "hello" == "hello"
    "#,
    )
    .expect_bool(true);
}

#[test]
fn string_inequality() {
    ShapeTest::new(
        r#"
        "hello" != "world"
    "#,
    )
    .expect_bool(true);
}

#[test]
fn string_less_than() {
    ShapeTest::new(
        r#"
        "abc" < "def"
    "#,
    )
    .expect_bool(true);
}

#[test]
fn string_greater_than() {
    ShapeTest::new(
        r#"
        "xyz" > "abc"
    "#,
    )
    .expect_bool(true);
}

#[test]
fn bool_equality_true() {
    ShapeTest::new(
        r#"
        true == true
    "#,
    )
    .expect_bool(true);
}

#[test]
fn bool_equality_false() {
    ShapeTest::new(
        r#"
        true == false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn comparison_in_if_condition() {
    ShapeTest::new(
        r#"
        let x = 10
        if x > 5 {
            "big"
        } else {
            "small"
        }
    "#,
    )
    .expect_string("big");
}

#[test]
fn chained_comparison_via_and() {
    ShapeTest::new(
        r#"
        let x = 5
        x >= 1 and x <= 10
    "#,
    )
    .expect_bool(true);
}
