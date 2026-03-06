//! Content builder tests.
//! Tests for Content.text(), Content.table(), Content.chart(), Content.code(),
//! Content.kv(), Content.fragment() static constructors.
//!
//! NOTE: Content.text() etc. are NOT currently exposed as callable builtins
//! at the Shape language level. The content system is accessed via c"..." syntax
//! and method chaining. These tests currently fail (TDD) until the builder
//! namespace is registered in the compiler/VM.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// Content builders via c"..." and method calls (working features)
// =====================================================================

#[test]
fn content_string_used_as_function_arg() {
    // Content values can be passed to functions
    ShapeTest::new(
        r#"
fn show(msg) {
    print(msg)
}
show(c"from function")
"#,
    )
    .expect_run_ok()
    .expect_output("from function");
}

#[test]
fn content_string_in_array() {
    // Content values can be stored in arrays
    ShapeTest::new(
        r#"
let items = [c"one", c"two", c"three"]
print(items.length)
"#,
    )
    .expect_run_ok()
    .expect_output("3");
}

#[test]
fn content_styled_in_function() {
    ShapeTest::new(
        r#"
fn make_header(text) {
    return c"{text:bold}"
}
print(make_header("Title"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("Title");
}

#[test]
fn content_string_concatenation_via_interpolation() {
    // Multiple content parts combined through interpolation
    ShapeTest::new(
        r#"
let prefix = "Hello"
let suffix = "World"
print(c"{prefix} {suffix}")
"#,
    )
    .expect_run_ok()
    .expect_output("Hello World");
}

#[test]
fn content_string_with_expression_interpolation() {
    ShapeTest::new(
        r#"
let x = 10
let y = 20
print(c"sum: {x + y}")
"#,
    )
    .expect_run_ok()
    .expect_output("sum: 30");
}

// =====================================================================
// Content.text() / Content.table() etc. (not yet exposed in language)
// =====================================================================

#[test]
// TDD: Content.text() not available as Shape-level builtin
fn content_builder_text() {
    ShapeTest::new(
        r#"
let node = Content.text("hello")
print(node)
"#,
    )
    .expect_run_ok()
    .expect_output("hello");
}

#[test]
// TDD: Content.table() not available as Shape-level builtin
fn content_builder_table() {
    ShapeTest::new(
        r#"
let t = Content.table(["Name", "Value"], [["a", "1"], ["b", "2"]])
print(t)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("Name");
}

#[test]
// TDD: Content.code() not available as Shape-level builtin
fn content_builder_code() {
    ShapeTest::new(
        r#"
let c = Content.code("rust", "fn main() {}")
print(c)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("fn main()");
}

#[test]
// TDD: Content.kv() not available as Shape-level builtin
fn content_builder_kv() {
    ShapeTest::new(
        r#"
let kv = Content.kv([["name", "Alice"], ["age", "30"]])
print(kv)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("name");
}

#[test]
// TDD: Content.fragment() not available as Shape-level builtin
fn content_builder_fragment() {
    ShapeTest::new(
        r#"
let a = Content.text("hello ")
let b = Content.text("world")
let f = Content.fragment([a, b])
print(f)
"#,
    )
    .expect_run_ok()
    .expect_output("hello world");
}
