//! Tests for regex operations: regex::replace, regex::replace_all, regex::split.
//!
//! The regex module is a stdlib module imported via `use std::core::regex`.

use shape_test::shape_test::ShapeTest;

#[test]
fn regex_replace_first() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::replace("foo bar foo", "foo", "baz")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("baz bar foo");
}

#[test]
fn regex_replace_all_occurrences() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::replace_all("foo bar foo", "foo", "baz")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("baz bar baz");
}

#[test]
fn regex_replace_with_capture_group() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::replace_all("2024-01-15", "(\\d{4})-(\\d{2})-(\\d{2})", "$3/$2/$1")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("15/01/2024");
}

#[test]
fn regex_split_by_comma() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let parts = regex::split("one,two,three", ",")
        print(parts)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn regex_split_by_whitespace() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let parts = regex::split("hello   world  test", "\\s+")
        print(parts)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn regex_replace_no_match() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::replace("hello world", "\\d+", "NUM")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("hello world");
}

#[test]
fn regex_replace_all_digits() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::replace_all("abc123def456", "\\d+", "NUM")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("abcNUMdefNUM");
}

#[test]
fn regex_split_returns_array() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let parts = regex::split("a-b-c", "-")
        let count = parts.length()
        print(count)
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}
