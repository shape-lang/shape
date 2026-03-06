//! Join strategy tests for async concurrency.
//!
//! Covers:
//! - `await join all { ... }` — wait for all branches
//! - `await join race { ... }` — first to complete wins
//! - `await join any { ... }` — first success wins
//! - `await join settle { ... }` — all complete, results collected
//!
//! Note: `await join` must be inside an `async fn`. The VM uses a cooperative
//! task scheduler — sync expressions resolve immediately via the sync shortcut
//! in `op_await`, so these tests verify parsing, compilation, and the spawn/join
//! opcode pipeline with immediately-resolved values.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// join all — all branches complete, results collected as tuple
// =========================================================================

#[test]
fn join_all_two_branches() {
    let code = r#"
async fn compute() {
    let result = await join all {
        1 + 2,
        3 + 4
    }
    print(result)
}

await compute()
"#;

    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn join_all_three_sync_expressions() {
    let code = r#"
async fn gather() {
    let r = await join all {
        10,
        20,
        30
    }
    print(r)
}

await gather()
"#;

    ShapeTest::new(code).expect_run_ok();
}

// =========================================================================
// join race — first to complete wins
// =========================================================================

#[test]
fn join_race_returns_first_completed() {
    let code = r#"
async fn fastest() {
    let winner = await join race {
        "alpha",
        "beta"
    }
    print(winner)
}

await fastest()
"#;

    ShapeTest::new(code).expect_run_ok();
}

// =========================================================================
// join any — first success wins (skips errors)
// =========================================================================

#[test]
fn join_any_returns_first_success() {
    let code = r#"
async fn first_ok() {
    let ok = await join any {
        "success_a",
        "success_b"
    }
    print(ok)
}

await first_ok()
"#;

    ShapeTest::new(code).expect_run_ok();
}

// =========================================================================
// join settle — all complete, individual results preserved
// =========================================================================

#[test]
fn join_settle_collects_all_results() {
    let code = r#"
async fn collect_all() {
    let results = await join settle {
        100,
        200
    }
    print(results)
}

await collect_all()
"#;

    ShapeTest::new(code).expect_run_ok();
}
