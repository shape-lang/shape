//! String method tests.
//!
//! Covers: .length, .toUpperCase(), .toLowerCase(), .trim(), .split(),
//!         .contains(), .startsWith(), .endsWith(), .replace(), .slice().

use shape_test::shape_test::ShapeTest;

#[test]
fn string_length() {
    ShapeTest::new(
        r#"
        "hello".length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn string_length_empty() {
    ShapeTest::new(
        r#"
        "".length
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn string_to_upper_case() {
    ShapeTest::new(
        r#"
        "hello".toUpperCase()
    "#,
    )
    .expect_string("HELLO");
}

#[test]
fn string_to_lower_case() {
    ShapeTest::new(
        r#"
        "HELLO".toLowerCase()
    "#,
    )
    .expect_string("hello");
}

#[test]
fn string_trim() {
    ShapeTest::new(
        r#"
        "  hello  ".trim()
    "#,
    )
    .expect_string("hello");
}

#[test]
fn string_contains_true() {
    ShapeTest::new(
        r#"
        "hello world".contains("world")
    "#,
    )
    .expect_bool(true);
}

#[test]
fn string_contains_false() {
    ShapeTest::new(
        r#"
        "hello world".contains("xyz")
    "#,
    )
    .expect_bool(false);
}

#[test]
fn string_starts_with_true() {
    ShapeTest::new(
        r#"
        "hello world".startsWith("hello")
    "#,
    )
    .expect_bool(true);
}

#[test]
fn string_starts_with_false() {
    ShapeTest::new(
        r#"
        "hello world".startsWith("world")
    "#,
    )
    .expect_bool(false);
}

#[test]
fn string_ends_with_true() {
    ShapeTest::new(
        r#"
        "hello world".endsWith("world")
    "#,
    )
    .expect_bool(true);
}

#[test]
fn string_ends_with_false() {
    ShapeTest::new(
        r#"
        "hello world".endsWith("hello")
    "#,
    )
    .expect_bool(false);
}

#[test]
fn string_replace() {
    ShapeTest::new(
        r#"
        "hello world".replace("world", "Shape")
    "#,
    )
    .expect_string("hello Shape");
}

// TDD: .split() may return array
#[test]
fn string_split() {
    ShapeTest::new(
        r#"
        let parts = "a,b,c".split(",")
        parts.length
    "#,
    )
    .expect_number(3.0);
}

// TDD: .slice() may not be supported
#[test]
fn string_slice() {
    ShapeTest::new(
        r#"
        "hello world".slice(0, 5)
    "#,
    )
    .expect_string("hello");
}
