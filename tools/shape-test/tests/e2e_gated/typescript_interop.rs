//! E2E tests for TypeScript language runtime integration.
//!
//! Requires: cargo test -p shape-test --features e2e-typescript
//!
//! The TypeScript extension provides `fn typescript name(...) { ... }` syntax
//! via deno_core V8 embedding and the LanguageRuntimeVTable plugin system.

use shape_test::shape_test::ShapeTest;

#[cfg(feature = "e2e-typescript")]
#[test]
// TDD: Requires TypeScript runtime extension loaded; not available in default test environment
fn typescript_basic_function_execution() {
    // foreign function syntax: fn typescript <name>(...) { <typescript_body> }
    ShapeTest::new(
        r#"
        fn typescript greet(name: string) -> string {
            return `Hello, ${name}!`;
        }
        print(greet("Shape"))
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("Hello, Shape!");
}

#[cfg(feature = "e2e-typescript")]
#[test]
// TDD: Requires TypeScript runtime extension loaded; not available in default test environment
fn typescript_value_roundtrip() {
    // Pass a number to TypeScript, compute, return
    ShapeTest::new(
        r#"
        fn typescript triple(x: number) -> number {
            return x * 3;
        }
        let result = triple(14.0)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("42");
}
