//! Async scope tests for structured concurrency.
//!
//! Covers:
//! - Basic async scope (cancellation boundary)
//! - Nested async scopes
//! - Scope with spawned tasks (async let inside scope)
//! - Scope variable capture from outer context
//!
//! `async scope { ... }` creates a structured concurrency boundary.
//! On scope exit, all pending tasks spawned within the scope are cancelled
//! in LIFO order. Must be inside an `async fn`.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic async scope
// =========================================================================

#[test]
fn basic_async_scope_returns_body_value() {
    let code = r#"
async fn work() {
    let result = async scope {
        42
    }
    print(result)
}

await work()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
fn async_scope_with_multiple_statements() {
    let code = r#"
async fn multi() {
    let result = async scope {
        let a = 10
        let b = 20
        a + b
    }
    print(result)
}

await multi()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("30");
}

// =========================================================================
// Nested async scopes
// =========================================================================

#[test]
fn nested_async_scopes() {
    let code = r#"
async fn nested() {
    let outer = async scope {
        let inner = async scope {
            "inner_value"
        }
        inner
    }
    print(outer)
}

await nested()
"#;

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("inner_value");
}

// =========================================================================
// Scope with async let inside
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_scope_with_async_let_inside() {
    // async let inside async scope spawns a task tracked by the scope.
    // On scope exit, pending tasks are cancelled.
    let code = r#"
async fn scoped_tasks() {
    let result = async scope {
        async let t = 99
        let v = await t
        v
    }
    print(result)
}

await scoped_tasks()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("99");
}

// =========================================================================
// Variable capture from outer context
// =========================================================================

#[test]
fn async_scope_captures_outer_variable() {
    let code = r#"
async fn capture() {
    let x = "hello"
    let result = async scope {
        x
    }
    print(result)
}

await capture()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("hello");
}
