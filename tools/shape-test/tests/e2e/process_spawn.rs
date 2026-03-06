//! E2E tests for process spawning through the Shape io module.
//!
//! Tests subprocess creation, stdout capture, and exit code checking.
//! These tests spawn real OS processes.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Process execution
// =========================================================================

#[test]
// TDD: io module globals not visible to semantic analyzer
fn exec_echo_captures_stdout() {
    // io.exec runs a command to completion and returns an object with stdout
    ShapeTest::new(
        r#"
        let result = io.exec("echo", ["hello"])
        print(result.stdout)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output_contains("hello");
}

#[test]
// TDD: io module globals not visible to semantic analyzer
fn exec_returns_exit_code() {
    // Successful command should return status 0
    ShapeTest::new(
        r#"
        let result = io.exec("true")
        print(result.status)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("0");
}

#[test]
// TDD: io module globals not visible to semantic analyzer
fn spawn_and_read_stdout() {
    // io.spawn creates a child process handle, then read from it
    ShapeTest::new(
        r#"
        let proc = io.spawn("echo", ["world"])
        let line = io.process_read(proc)
        print(line)
        io.process_wait(proc)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output_contains("world");
}
