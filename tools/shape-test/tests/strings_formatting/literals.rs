//! String literal tests.
//!
//! Covers: double-quoted strings, escapes, multi-line strings.

use shape_test::shape_test::ShapeTest;

#[test]
fn double_quoted_string() {
    ShapeTest::new(
        r#"
        "hello world"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn empty_string() {
    ShapeTest::new(
        r#"
        ""
    "#,
    )
    .expect_string("");
}

#[test]
fn string_with_escape_newline() {
    ShapeTest::new(
        r#"
        let s = "line1\nline2"
        print(s)
    "#,
    )
    .expect_run_ok()
    .expect_output("line1\nline2");
}

#[test]
fn string_with_escape_tab() {
    ShapeTest::new(
        r#"
        let s = "col1\tcol2"
        print(s)
    "#,
    )
    .expect_run_ok()
    .expect_output("col1\tcol2");
}

#[test]
fn string_with_escaped_backslash() {
    ShapeTest::new(
        r#"
        let s = "path\\to\\file"
        print(s)
    "#,
    )
    .expect_run_ok()
    .expect_output("path\\to\\file");
}

#[test]
fn string_concatenation_plus() {
    ShapeTest::new(
        r#"
        "hello" + " " + "world"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn string_with_numbers() {
    ShapeTest::new(
        r#"
        "count: " + "42"
    "#,
    )
    .expect_string("count: 42");
}

#[test]
fn single_character_string() {
    ShapeTest::new(
        r#"
        "x"
    "#,
    )
    .expect_string("x");
}
