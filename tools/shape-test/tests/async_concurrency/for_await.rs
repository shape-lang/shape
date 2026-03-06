//! For-await tests for async iteration.
//!
//! Covers:
//! - Basic `for await` iteration syntax
//! - `for await` with a collection
//! - `for await` with break
//!
//! Grammar: `for await x in stream { ... }`
//! The `is_async` flag on ForExpr/ForStatement distinguishes sync from async for.
//! Currently, for-await parses and compiles but the runtime treats the iterable
//! as a sync collection (no real async stream protocol yet). Tests verify the
//! syntax round-trips through parse -> compile -> execute.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic for await
// =========================================================================

#[test]
fn for_await_iterates_over_array() {
    // for await iterates over a sync array — the async path treats it
    // like a regular for loop since there is no async stream protocol yet.
    let code = r#"
async fn iterate() {
    for await x in [1, 2, 3] {
        print(x)
    }
}

await iterate()
"#;

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("1\n2\n3");
}

// =========================================================================
// For await with accumulation
// =========================================================================

#[test]
fn for_await_accumulates_values() {
    let code = r#"
async fn sum_stream() {
    var total = 0
    for await n in [10, 20, 30] {
        total = total + n
    }
    print(total)
}

await sum_stream()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("60");
}

// =========================================================================
// For await with break
// =========================================================================

#[test]
fn for_await_with_break() {
    let code = r#"
async fn early_exit() {
    for await x in [1, 2, 3, 4, 5] {
        if x == 3 {
            break
        }
        print(x)
    }
    print("done")
}

await early_exit()
"#;

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("1\n2\ndone");
}
