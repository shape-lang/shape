//! Async let tests for task spawning.
//!
//! Covers:
//! - Basic async let (spawn + bind future)
//! - Multiple async let bindings
//! - Awaiting a spawned task
//! - Async let with computed expressions
//!
//! `async let x = expr` spawns a task, binds the future handle to `x`.
//! `await x` resolves the future. Must be inside an `async fn`.
//!
//! Known limitation: The semantic analyzer does not track variable bindings
//! from `async let`, so code like `async let x = 42; await x` produces
//! "Undefined variable: 'x'" at the semantic analysis phase. The compiler
//! and VM handle it correctly. These tests currently fail (TDD) until
//! the semantic analyzer is updated.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic async let
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_let_spawns_and_awaits() {
    let code = r#"
async fn spawn_one() {
    async let x = 42
    let result = await x
    print(result)
}

await spawn_one()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_let_with_expression() {
    let code = r#"
async fn compute() {
    async let total = 10 + 20 + 30
    let result = await total
    print(result)
}

await compute()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("60");
}

// =========================================================================
// Multiple async let bindings
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn multiple_async_let_bindings() {
    let code = r#"
async fn multi() {
    async let a = 1
    async let b = 2
    async let c = 3
    let va = await a
    let vb = await b
    let vc = await c
    print(va + vb + vc)
}

await multi()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("6");
}

// =========================================================================
// Async let with string values
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_let_with_string_value() {
    let code = r#"
async fn fetch_name() {
    async let name = "Shape"
    let result = await name
    print(f"Hello, {result}!")
}

await fetch_name()
"#;

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Hello, Shape!");
}

// =========================================================================
// Async let requires async function context
// =========================================================================

#[test]
// TDD: Semantic analyzer reports "Undefined variable" before compiler can emit the async error
fn async_let_outside_async_fn_is_compile_error() {
    // The compiler should reject `async let` outside an async function,
    // but the semantic analyzer currently errors first with "Undefined variable".
    let code = r#"
fn sync_fn() {
    async let x = 42
    x
}

sync_fn()
"#;

    ShapeTest::new(code).expect_run_err_contains("async");
}
