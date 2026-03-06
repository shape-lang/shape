//! E2E tests for Python language runtime integration.
//!
//! Requires: cargo test -p shape-test --features e2e-python
//!
//! The Python extension provides `fn python name(...) { ... }` syntax
//! via the LanguageRuntimeVTable plugin system.

use shape_test::shape_test::ShapeTest;

#[cfg(feature = "e2e-python")]
#[test]
// TDD: Requires Python runtime extension loaded; not available in default test environment
fn python_basic_function_execution() {
    // foreign function syntax: fn python <name>(...) { <python_body> }
    ShapeTest::new(
        r#"
        fn python greet(name: string) -> string {
            return f"Hello, {name}!"
        }
        print(greet("Shape"))
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("Hello, Shape!");
}

#[cfg(feature = "e2e-python")]
#[test]
// TDD: Requires Python runtime extension loaded; not available in default test environment
fn python_value_roundtrip() {
    // Pass a number to Python, compute, return
    ShapeTest::new(
        r#"
        fn python double(x: number) -> number {
            return x * 2
        }
        let result = double(21.0)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("42");
}
