//! Content string literal tests.
//! Covers c"...", c$"...", c#"..." syntax, interpolation, print(), and assignment.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// Basic content string literals
// =====================================================================

#[test]
fn content_string_plain() {
    ShapeTest::new(
        r#"
let x = c"hello world"
print(x)
"#,
    )
    .expect_run_ok()
    .expect_output("hello world");
}

#[test]
fn content_string_empty() {
    ShapeTest::new(
        r#"
let x = c""
print(x)
"#,
    )
    .expect_run_ok()
    .expect_output("");
}

#[test]
fn content_string_assignment_and_reuse() {
    ShapeTest::new(
        r#"
let greeting = c"hi there"
let copy = greeting
print(copy)
"#,
    )
    .expect_run_ok()
    .expect_output("hi there");
}

// =====================================================================
// Interpolated content strings
// =====================================================================

#[test]
fn content_string_interpolation_brace() {
    ShapeTest::new(
        r#"
let name = "Alice"
let msg = c"Hello {name}!"
print(msg)
"#,
    )
    .expect_run_ok()
    .expect_output("Hello Alice!");
}

#[test]
fn content_string_interpolation_numeric() {
    ShapeTest::new(
        r#"
let x = 42
print(c"value is {x}")
"#,
    )
    .expect_run_ok()
    .expect_output("value is 42");
}

#[test]
fn content_string_multiple_interpolations() {
    ShapeTest::new(
        r#"
let a = "foo"
let b = "bar"
print(c"{a} and {b}")
"#,
    )
    .expect_run_ok()
    .expect_output("foo and bar");
}

#[test]
fn content_string_dollar_prefix_interpolation() {
    ShapeTest::new(
        r#"
let x = 10
print(c$"result: ${x}")
"#,
    )
    .expect_run_ok()
    .expect_output("result: 10");
}

#[test]
fn content_string_hash_prefix_interpolation() {
    ShapeTest::new(
        r#"
let cmd = "ls"
print(c#"run: #{cmd}")
"#,
    )
    .expect_run_ok()
    .expect_output("run: ls");
}

// =====================================================================
// Content string with print()
// =====================================================================

#[test]
fn content_string_print_directly() {
    ShapeTest::new(
        r#"
print(c"direct content output")
"#,
    )
    .expect_run_ok()
    .expect_output("direct content output");
}

#[test]
fn content_string_no_interpolation_plain() {
    ShapeTest::new(
        r#"
print(c"just plain text")
"#,
    )
    .expect_run_ok()
    .expect_output("just plain text");
}
