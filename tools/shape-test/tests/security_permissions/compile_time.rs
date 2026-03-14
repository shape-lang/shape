//! Compile-time permission and capability tests.
//!
//! The Shape compiler checks import permissions when a PermissionSet is active.
//! Pure-computation modules (json, crypto, math, regex, log, testing) require
//! no permissions. IO-related modules require specific capabilities.
//!
//! These tests currently fail (TDD) because ShapeTest does not yet expose
//! set_permission_set(). The capability_tags unit tests in shape-runtime
//! cover the mapping logic directly.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Pure modules should always be importable
// =========================================================================

#[test]
fn pure_module_json_parses_without_permissions() {
    // json is a pure-computation module, no permissions needed
    ShapeTest::new(
        r#"
        from std::core::json use { parse, stringify }
        let data = parse("{\"key\": 42}")
        print(data.key)
    "#,
    )
    .with_stdlib()
    .expect_parse_ok();
}

#[test]
fn pure_module_math_parses_without_permissions() {
    ShapeTest::new(
        r#"
        from math use { abs, sqrt }
        let x = abs(-5)
        print(x)
    "#,
    )
    .with_stdlib()
    .expect_parse_ok();
}

// =========================================================================
// IO module should require permissions (compile-time check)
// =========================================================================

#[test]
fn io_import_denied_with_pure_permissions() {
    // With a pure PermissionSet, importing io.open should fail at compile time
    // because io.open requires FsRead capability.
    ShapeTest::new(
        r#"
        from std::core::io use { open }
        let f = open("/tmp/test.txt")
    "#,
    )
    .with_stdlib()
    .with_pure_permissions()
    .expect_run_err();
}

#[test]
fn net_connect_denied_with_pure_permissions() {
    // With a pure PermissionSet, importing io.tcp_connect should fail
    // because it requires NetConnect capability.
    ShapeTest::new(
        r#"
        from std::core::io use { tcp_connect }
        let conn = tcp_connect("127.0.0.1:8080")
    "#,
    )
    .with_stdlib()
    .with_pure_permissions()
    .expect_run_err();
}

#[test]
fn process_spawn_denied_with_pure_permissions() {
    // With a pure PermissionSet, importing io.spawn should fail
    // because it requires Process capability.
    ShapeTest::new(
        r#"
        from std::core::io use { spawn }
        let p = spawn("echo", ["hello"])
    "#,
    )
    .with_stdlib()
    .with_pure_permissions()
    .expect_run_err();
}
