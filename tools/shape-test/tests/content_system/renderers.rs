//! Content renderer integration tests.
//! Tests that print() on content values produces expected output.
//! The VM uses TerminalRenderer by default for content values.
//!
//! Since we can only observe the final rendered output through print(),
//! these tests verify the end-to-end rendering pipeline.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// Terminal rendering (default for print())
// =====================================================================

#[test]
fn render_plain_content_string() {
    // Plain content string with no styles should render as plain text
    ShapeTest::new(
        r#"
print(c"no styling")
"#,
    )
    .expect_run_ok()
    .expect_output("no styling");
}

#[test]
fn render_styled_content_preserves_text() {
    // Even with styles applied, the text content is preserved
    ShapeTest::new(
        r#"
let c = c"styled text"
print(c.bold().fg("red"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("styled text");
}

#[test]
fn render_interpolated_content_preserves_values() {
    ShapeTest::new(
        r#"
let x = 100
let y = 200
print(c"x={x}, y={y}")
"#,
    )
    .expect_run_ok()
    .expect_output("x=100, y=200");
}

#[test]
fn render_mixed_styled_and_plain_parts() {
    // A content string with both styled and unstyled interpolation parts
    ShapeTest::new(
        r#"
let label = "Status"
let value = "OK"
print(c"{label}: {value:fg(green)}")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("Status");
}

// =====================================================================
// Content toString() strips styles
// =====================================================================

#[test]
fn to_string_returns_plain_text() {
    // toString() on a content value returns a regular string with no styling
    ShapeTest::new(
        r#"
let c = c"hello"
let s = c.toString()
print(s)
"#,
    )
    .expect_run_ok()
    .expect_output("hello");
}

// =====================================================================
// Multiple print calls with content
// =====================================================================

#[test]
fn multiple_content_prints() {
    ShapeTest::new(
        r#"
print(c"line 1")
print(c"line 2")
print(c"line 3")
"#,
    )
    .expect_run_ok()
    .expect_output("line 1\nline 2\nline 3");
}

#[test]
fn content_and_regular_string_prints() {
    ShapeTest::new(
        r#"
print("regular string")
print(c"content string")
print("another regular")
"#,
    )
    .expect_run_ok()
    .expect_output("regular string\ncontent string\nanother regular");
}
