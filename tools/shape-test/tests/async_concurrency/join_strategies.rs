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

fn join_all_two_ms(sleep_ms: &str) -> u128 {
    let code = format!(
        r#"
use std::core::time

async fn work() -> int {{
    await time::sleep({sleep_ms})
    21
}}

async fn gather() {{
    let result = await join all {{
        work(),
        work()
    }}
    print(result[0] + result[1])
}}

await gather()
"#
    );

    let start = std::time::Instant::now();
    ShapeTest::new(&code).with_stdlib().expect_output("42");
    start.elapsed().as_millis()
}

fn join_race_two_ms(slow_ms: &str, fast_ms: &str) -> u128 {
    let code = format!(
        r#"
use std::core::time

async fn slow() -> int {{
    await time::sleep({slow_ms})
    2
}}

async fn fast() -> int {{
    await time::sleep({fast_ms})
    1
}}

async fn choose() {{
    let winner = await join race {{
        slow(),
        fast()
    }}
    print(winner)
}}

await choose()
"#
    );

    let start = std::time::Instant::now();
    ShapeTest::new(&code).with_stdlib().expect_output("1");
    start.elapsed().as_millis()
}

fn join_any_two_ms(slow_ms: &str, fast_ms: &str) -> u128 {
    let code = format!(
        r#"
use std::core::time

async fn slow() -> int {{
    await time::sleep({slow_ms})
    9
}}

async fn fast() -> int {{
    await time::sleep({fast_ms})
    7
}}

async fn choose() {{
    let winner = await join any {{
        slow(),
        fast()
    }}
    print(winner)
}}

await choose()
"#
    );

    let start = std::time::Instant::now();
    ShapeTest::new(&code).with_stdlib().expect_output("7");
    start.elapsed().as_millis()
}

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
    print(result[0] + result[1])
}

await compute()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("10");
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
    print(r[0] + r[1] + r[2])
}

await gather()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("60");
}

#[test]
fn join_all_mixed_carriers_reports_clear_error() {
    let code = r#"
async fn gather() {
    let result = await join all {
        1,
        "two"
    }
    print(result)
}

await gather()
"#;

    ShapeTest::new(code)
        .expect_run_err_contains("join all cannot materialize mixed result carriers");
}

#[test]
fn join_all_two_one_second_tasks_overlap() {
    let _ = join_all_two_ms("0.0");

    let baseline = join_all_two_ms("0.0");
    let with_sleep = join_all_two_ms("1000.0");
    let sleep_contribution = with_sleep.saturating_sub(baseline);
    eprintln!(
        "join_all overlap timing: baseline={baseline}ms with_sleep={with_sleep}ms \
         sleep_contribution={sleep_contribution}ms"
    );

    assert!(
        sleep_contribution < 1500,
        "join all did not overlap: two 1s branches added {sleep_contribution}ms over the \
         {baseline}ms zero-sleep baseline (with_sleep={with_sleep}ms). Expected < 1500ms; \
         a serial regression adds ~2000ms."
    );
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

    ShapeTest::new(code).expect_run_ok().expect_output("alpha");
}

#[test]
fn join_race_returns_fastest_branch_without_waiting_for_loser() {
    let _ = join_race_two_ms("50.0", "0.0");

    let baseline = join_race_two_ms("50.0", "0.0");
    let with_sleep = join_race_two_ms("1000.0", "100.0");
    let race_contribution = with_sleep.saturating_sub(baseline);
    eprintln!(
        "join_race timing: baseline={baseline}ms with_sleep={with_sleep}ms \
         race_contribution={race_contribution}ms"
    );

    assert!(
        race_contribution < 600,
        "join race waited for the slow branch: winner run added {race_contribution}ms over \
         the {baseline}ms short baseline (with_sleep={with_sleep}ms). Expected < 600ms; \
         waiting for the loser adds ~1000ms."
    );
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

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("success_a");
}

#[test]
fn join_any_returns_first_success_without_waiting_for_slower_success() {
    let _ = join_any_two_ms("50.0", "0.0");

    let baseline = join_any_two_ms("50.0", "0.0");
    let with_sleep = join_any_two_ms("1000.0", "100.0");
    let any_contribution = with_sleep.saturating_sub(baseline);
    eprintln!(
        "join_any timing: baseline={baseline}ms with_sleep={with_sleep}ms \
         any_contribution={any_contribution}ms"
    );

    assert!(
        any_contribution < 600,
        "join any waited for the slower successful branch: winner run added \
         {any_contribution}ms over the {baseline}ms short baseline \
         (with_sleep={with_sleep}ms). Expected < 600ms; waiting for the slow \
         success adds ~1000ms."
    );
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

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("[TaskGroup:Settle(2)]");
}
