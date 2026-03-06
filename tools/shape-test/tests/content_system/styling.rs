//! Content styling tests.
//! Covers method-based styling (.bold(), .fg(), .bg(), .italic(), etc.),
//! inline format spec styling (c"{val:bold}"), and style chaining.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// Inline format spec styling (c"{val:bold}", c"{val:fg(red)}")
// =====================================================================

#[test]
fn content_inline_bold_style() {
    // Inline bold style spec: the plain text is preserved
    ShapeTest::new(
        r#"
let name = "World"
print(c"Hello {name:bold}")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("World");
}

#[test]
fn content_inline_fg_color() {
    ShapeTest::new(
        r#"
let msg = "error"
print(c"Status: {msg:fg(red)}")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("error");
}

#[test]
fn content_inline_multiple_styles() {
    ShapeTest::new(
        r#"
let val = "important"
print(c"{val:fg(green), bold, underline}")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("important");
}

// =====================================================================
// Method-based styling
// =====================================================================

#[test]
fn content_method_bold() {
    ShapeTest::new(
        r#"
let c = c"hello"
let styled = c.bold()
print(styled)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("hello");
}

#[test]
fn content_method_italic() {
    ShapeTest::new(
        r#"
let c = c"text"
print(c.italic())
"#,
    )
    .expect_run_ok()
    .expect_output_contains("text");
}

#[test]
fn content_method_fg_color() {
    ShapeTest::new(
        r#"
let c = c"warning"
print(c.fg("yellow"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("warning");
}

#[test]
fn content_method_bg_color() {
    ShapeTest::new(
        r#"
let c = c"highlighted"
print(c.bg("blue"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("highlighted");
}

#[test]
fn content_method_underline() {
    ShapeTest::new(
        r#"
let c = c"link"
print(c.underline())
"#,
    )
    .expect_run_ok()
    .expect_output_contains("link");
}

#[test]
fn content_method_dim() {
    ShapeTest::new(
        r#"
let c = c"faded"
print(c.dim())
"#,
    )
    .expect_run_ok()
    .expect_output_contains("faded");
}

// =====================================================================
// Style chaining
// =====================================================================

#[test]
fn content_method_chaining_bold_italic_fg() {
    ShapeTest::new(
        r#"
let c = c"styled"
print(c.bold().italic().fg("cyan"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("styled");
}

#[test]
fn content_to_string_method() {
    ShapeTest::new(
        r#"
let c = c"hello world"
let s = c.toString()
print(s)
"#,
    )
    .expect_run_ok()
    .expect_output("hello world");
}
