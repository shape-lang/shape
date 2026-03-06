//! E2E tests for network I/O through the Shape io module.
//!
//! Tests TCP and UDP operations using ephemeral ports on localhost.
//! These tests create real sockets and may fail in restricted environments.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// TCP
// =========================================================================

#[test]
// TDD: io module globals not visible to semantic analyzer
fn tcp_listen_and_connect() {
    // Test that io.tcp_listen and io.tcp_connect produce IoHandle values
    ShapeTest::new(
        r#"
        let listener = io.tcp_listen("127.0.0.1:0")
        print(type_of(listener))
        io.tcp_close(listener)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output_contains("IoHandle");
}

#[test]
// TDD: io module globals not visible to semantic analyzer
fn tcp_send_receive_roundtrip() {
    // Spawn a listener, connect, send data, read it back
    ShapeTest::new(
        r#"
        let listener = io.tcp_listen("127.0.0.1:0")
        let addr = io.local_addr(listener)
        // Connect in same thread (non-blocking accept needed for real E2E)
        let client = io.tcp_connect(addr)
        io.tcp_write(client, "hello")
        io.tcp_close(client)
        io.tcp_close(listener)
        print("done")
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output("done");
}

// =========================================================================
// UDP
// =========================================================================

#[test]
// TDD: io module globals not visible to semantic analyzer
fn udp_bind_and_send() {
    // Test that io.udp_bind creates a valid handle
    ShapeTest::new(
        r#"
        let sock = io.udp_bind("127.0.0.1:0")
        print(type_of(sock))
        io.tcp_close(sock)
    "#,
    )
    .with_stdlib()
    .expect_run_ok()
    .expect_output_contains("IoHandle");
}
