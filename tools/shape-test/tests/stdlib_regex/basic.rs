//! Tests for regex basic functions: regex.is_match, regex.match, regex.match_all.
//!
//! The regex module is a stdlib module accessed as a global object.
//! The semantic analyzer does not recognize stdlib globals (TDD).

use shape_test::shape_test::ShapeTest;

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_is_match_true() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.is_match("hello world", "world")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_is_match_false() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.is_match("hello world", "^\\d+$")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("false");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_is_match_with_word_boundary() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = regex.is_match("hello world", "\\bworld\\b")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_match_found() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let m = regex.match("abc 123 def", "(\\d+)")
        print(m)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_match_not_found() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let m = regex.match("hello world", "\\d+")
        print(m)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_match_all_multiple() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let matches = regex.match_all("a1 b2 c3", "\\d")
        print(matches)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_match_all_no_results() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let matches = regex.match_all("abc", "\\d+")
        print(matches)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `regex` as a global
#[test]
fn regex_is_match_email_pattern() {
    // TDD: regex global not recognized by semantic analyzer
    ShapeTest::new(r#"
        let result = regex.is_match("user@example.com", "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}")
        print(result)
    "#)
    .with_stdlib()
    .expect_output("true");
}
