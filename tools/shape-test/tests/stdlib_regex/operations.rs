//! Tests for regex operations: regex.replace, regex.replace_all, regex.split.
//!
//! The regex module is a stdlib module accessed as a global object.
//! The semantic analyzer does not recognize stdlib globals (TDD).

use shape_test::shape_test::ShapeTest;

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_replace_first() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.replace("foo bar foo", "foo", "baz")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("baz bar foo");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_replace_all_occurrences() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.replace_all("foo bar foo", "foo", "baz")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("baz bar baz");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_replace_with_capture_group() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.replace_all("2024-01-15", "(\\d{4})-(\\d{2})-(\\d{2})", "$3/$2/$1")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("15/01/2024");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_split_by_comma() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let parts = regex.split("one,two,three", ",")
        print(parts)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_split_by_whitespace() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let parts = regex.split("hello   world  test", "\\s+")
        print(parts)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_replace_no_match() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.replace("hello world", "\\d+", "NUM")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("hello world");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_replace_all_digits() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.replace_all("abc123def456", "\\d+", "NUM")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("abcNUMdefNUM");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_split_returns_array() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let parts = regex.split("a-b-c", "-")
        let count = parts.length()
        print(count)
    "#,
    )
    .with_stdlib()
    .expect_output("3");
}
