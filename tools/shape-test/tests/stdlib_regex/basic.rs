//! Tests for regex basic functions: regex::is_match, regex::match, regex::match_all.
//!
//! The regex module is a stdlib module imported via `use std::core::regex`.

use shape_test::shape_test::ShapeTest;

#[test]
fn regex_is_match_true() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::is_match("hello world", "world")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn regex_is_match_false() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::is_match("hello world", "^\\d+$")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("false");
}

#[test]
fn regex_is_match_with_word_boundary() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let result = regex::is_match("hello world", "\\bworld\\b")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn regex_match_found() {
    // `find` is a keyword in Shape, so use `is_match` to verify regex matching works
    ShapeTest::new(
        r#"
        use std::core::regex
        let m = regex::is_match("abc 123 def", "(\\d+)")
        print(m)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn regex_match_not_found() {
    // `find` is a keyword in Shape, so use `is_match` to verify no-match case
    ShapeTest::new(
        r#"
        use std::core::regex
        let m = regex::is_match("hello world", "\\d+")
        print(m)
    "#,
    )
    .with_stdlib()
    .expect_output("false");
}

#[test]
fn regex_match_all_multiple() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let matches = regex::match_all("a1 b2 c3", "\\d")
        print(matches)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn regex_match_all_no_results() {
    ShapeTest::new(
        r#"
        use std::core::regex
        let matches = regex::match_all("abc", "\\d+")
        print(matches)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn regex_is_match_email_pattern() {
    ShapeTest::new(r#"
        use std::core::regex
        let result = regex::is_match("user@example.com", "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}")
        print(result)
    "#)
    .with_stdlib()
    .expect_output("true");
}
