//! Stress tests for string interpolation (f-strings) and template patterns.

use shape_test::shape_test::ShapeTest;

// ========================================================================
// 21. String Interpolation (f-strings)
// ========================================================================

/// Verifies basic f-string interpolation with variable.
#[test]
fn test_interpolation_basic() {
    ShapeTest::new(
        r#"fn test() -> string {
            let name = "world"
            f"hello {name}"
        }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies f-string interpolation with expression.
#[test]
fn test_interpolation_expression() {
    ShapeTest::new(
        r#"fn test() -> string {
            let x = 5
            f"value is {x + 1}"
        }
test()"#,
    )
    .expect_string("value is 6");
}

/// Verifies f-string interpolation with multiple variables.
#[test]
fn test_interpolation_multiple() {
    ShapeTest::new(
        r#"fn test() -> string {
            let a = "hello"
            let b = "world"
            f"{a} {b}"
        }
test()"#,
    )
    .expect_string("hello world");
}

/// Verifies empty f-string template.
#[test]
fn test_interpolation_empty_template() {
    ShapeTest::new(
        r#"fn test() -> string {
            let x = "nothing"
            f""
        }
test()"#,
    )
    .expect_string("");
}
