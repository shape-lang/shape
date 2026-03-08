//! Runtime permission gating tests.
//!
//! RuntimePolicy governs what a Shape program can access at execution time:
//! - allowed_paths / read_only_paths for filesystem access
//! - allowed_hosts for network connections
//! - memory_limit, time_limit, output_limit for resource bounds
//!
//! These tests currently fail (TDD) because ShapeTest does not yet expose
//! RuntimePolicy configuration. The runtime_policy unit tests in shape-runtime
//! cover the policy logic directly.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Filesystem path restrictions
// =========================================================================

#[test]
// TDD: ShapeTest does not expose RuntimePolicy
fn read_only_path_blocks_writes() {
    // A RuntimePolicy with read_only_paths should allow reads but deny writes
    // to those paths.
    ShapeTest::new(
        r#"
        let f = io.open("/protected/data.txt", "w")
        io.write(f, "should fail")
    "#,
    )
    .with_stdlib()
    .expect_run_err();
}

#[test]
// TDD: ShapeTest does not expose RuntimePolicy
fn allowed_paths_restricts_access() {
    // A RuntimePolicy with specific allowed_paths should deny access to
    // files outside those paths.
    ShapeTest::new(
        r#"
        let f = io.open("/etc/passwd", "r")
        let content = io.read(f)
    "#,
    )
    .with_stdlib()
    .expect_run_err();
}

// =========================================================================
// Network host restrictions
// =========================================================================

#[test]
// TDD: ShapeTest does not expose RuntimePolicy
fn allowed_hosts_blocks_unauthorized_connections() {
    // A RuntimePolicy with allowed_hosts should deny connections to
    // hosts not in the allowed list.
    ShapeTest::new(
        r#"
        let conn = io.tcp_connect("evil.example.com:443")
    "#,
    )
    .with_stdlib()
    .expect_run_err();
}
