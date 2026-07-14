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

fn unawaited_scope_child_ms(sleep_ms: &str) -> u128 {
    let code = format!(
        r#"
use std::core::time

async fn scoped() {{
    let result = async scope {{
        async let sleeper = time::sleep({sleep_ms})
        7
    }}
    print(result)
}}

await scoped()
"#
    );

    let start = std::time::Instant::now();
    ShapeTest::new(&code).with_stdlib().expect_output("7");
    start.elapsed().as_millis()
}

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

#[test]
fn async_scope_exit_cancels_unawaited_child_without_waiting() {
    let _ = unawaited_scope_child_ms("0.0");

    let baseline = unawaited_scope_child_ms("0.0");
    let with_sleep = unawaited_scope_child_ms("1000.0");
    let cancellation_contribution = with_sleep.saturating_sub(baseline);
    eprintln!(
        "async_scope cancellation timing: baseline={baseline}ms with_sleep={with_sleep}ms \
         cancellation_contribution={cancellation_contribution}ms"
    );

    assert!(
        cancellation_contribution < 500,
        "async scope exit waited for an unawaited child: pending 1s child added \
         {cancellation_contribution}ms over the {baseline}ms zero-sleep baseline \
         (with_sleep={with_sleep}ms). Expected < 500ms; waiting for the child adds ~1000ms."
    );
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
