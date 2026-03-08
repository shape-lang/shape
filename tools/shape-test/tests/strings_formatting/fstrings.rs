//! Format string (f-string) tests.
//!
//! Covers: f"...", f$"...", f#"..." interpolation.

use shape_test::shape_test::ShapeTest;

#[test]
fn fstring_basic_variable() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"hello {name}"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn fstring_integer_interpolation() {
    ShapeTest::new(
        r#"
        let x = 42
        f"value is {x}"
    "#,
    )
    .expect_string("value is 42");
}

#[test]
fn fstring_expression_interpolation() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 4
        f"sum is {a + b}"
    "#,
    )
    .expect_string("sum is 7");
}

#[test]
fn fstring_multiple_interpolations() {
    ShapeTest::new(
        r#"
        let first = "John"
        let last = "Doe"
        f"{first} {last}"
    "#,
    )
    .expect_string("John Doe");
}

#[test]
fn fstring_with_string_method() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"HELLO {name.toUpperCase()}"
    "#,
    )
    .expect_string("HELLO WORLD");
}

#[test]
fn fstring_empty_interpolation_at_edges() {
    ShapeTest::new(
        r#"
        let x = "edge"
        f"{x} test"
    "#,
    )
    .expect_string("edge test");
}

// f$ uses ${} for interpolation — bare {} is literal text
#[test]
fn fstring_dollar_literal_braces() {
    ShapeTest::new(
        r#"
        let x = 10
        f$"value: {x}"
    "#,
    )
    .expect_string("value: {x}");
}

#[test]
fn fstring_dollar_interpolation() {
    ShapeTest::new(
        r#"
        let x = 10
        f$"value: ${x}"
    "#,
    )
    .expect_string("value: 10");
}

// f# uses #{} for interpolation — bare {} is literal text
#[test]
fn fstring_hash_literal_braces() {
    ShapeTest::new(
        r#"
        let x = 5
        f#"raw {x}"
    "#,
    )
    .expect_string("raw {x}");
}

#[test]
fn fstring_hash_interpolation() {
    ShapeTest::new(
        r##"
        let x = 5
        f#"raw #{x}"
    "##,
    )
    .expect_string("raw 5");
}

// =========================================================================
// Regression: nested strings inside {}-interpolation blocks
// Bug: f"text: {fn("arg")}" caused the grammar to terminate the f-string at
// the inner `"`, producing a bad parse error with wrong line attribution.
// =========================================================================

#[test]
fn fstring_nested_string_literal_in_interpolation() {
    // A string literal inside {} should be parsed as part of the expression,
    // not terminate the outer f-string.
    ShapeTest::new(
        r#"
        f"value: {"nested"}"
    "#,
    )
    .expect_parse_ok()
    .expect_string("value: nested");
}

#[test]
fn fstring_nested_string_in_function_call() {
    // The exact pattern from the error-handling playground example.
    ShapeTest::new(
        r#"
        let result = Err("oops")
        f"Err: {result}"
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn fstring_nested_string_as_call_arg() {
    // Nested string used as a direct argument inside the interpolation.
    ShapeTest::new(
        r#"
        fn greet(name: str) -> str { f"Hello, {name}!" }
        f"msg: {greet("world")}"
    "#,
    )
    .expect_parse_ok()
    .expect_string("msg: Hello, world!");
}

#[test]
fn fstring_nested_err_with_string_arg() {
    // Regression: this exact snippet produced a bad error and wrong line number.
    ShapeTest::new(
        r#"
        print(f"Err: {Err("oops")}")
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn cstring_nested_string_in_interpolation() {
    // c-strings share the same grammar fix — nested quotes should work there too.
    ShapeTest::new(
        r#"
        c"label: {"value"}"
    "#,
    )
    .expect_parse_ok();
}
