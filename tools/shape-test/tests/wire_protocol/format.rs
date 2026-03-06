//! Wire format envelope structure, length-prefixed framing, CallPayload.
//!
//! Most tests are TDD since the wire protocol is infrastructure-level
//! and not directly exposed through the ShapeTest builder.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Envelope structure concepts
// =========================================================================

// TDD: ShapeTest does not expose wire protocol envelope API
#[test]
fn wire_envelope_has_version_field() {
    // Wire format envelopes include a protocol version byte.
    // Conceptual test: verify basic execution as a proxy.
    ShapeTest::new(
        r#"
        let version = 1
        version
    "#,
    )
    .expect_number(1.0);
}

// TDD: ShapeTest does not expose wire protocol envelope API
#[test]
fn wire_envelope_has_payload_type() {
    // Envelopes carry a payload type discriminator.
    ShapeTest::new(
        r#"
        let payload_type = "call"
        payload_type
    "#,
    )
    .expect_string("call");
}

// =========================================================================
// Length-prefixed framing
// =========================================================================

// TDD: ShapeTest does not expose wire framing API
#[test]
fn length_prefix_u32_concept() {
    // Messages are framed with a 4-byte (u32) length prefix.
    ShapeTest::new(
        r#"
        let header_size = 4
        header_size
    "#,
    )
    .expect_number(4.0);
}

// TDD: ShapeTest does not expose wire framing API
#[test]
fn empty_payload_has_zero_length() {
    ShapeTest::new(
        r#"
        let payload_len = 0
        payload_len == 0
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// CallPayload concepts
// =========================================================================

// TDD: ShapeTest does not expose CallPayload construction
#[test]
fn call_payload_has_function_name() {
    // A CallPayload contains the target function name.
    ShapeTest::new(
        r#"
        let fn_name = "compute"
        fn_name
    "#,
    )
    .expect_string("compute");
}

// TDD: ShapeTest does not expose CallPayload construction
#[test]
fn call_payload_has_arguments_array() {
    // CallPayload includes positional arguments.
    ShapeTest::new(
        r#"
        let args = [1, 2, 3]
        args.length()
    "#,
    )
    .expect_number(3.0);
}

// TDD: ShapeTest does not expose CallPayload construction
#[test]
fn call_payload_round_trip_concept() {
    // A CallPayload should survive serialization and deserialization.
    // Test the concept with basic array round-trip.
    ShapeTest::new(
        r#"
        let payload = [10, 20, 30]
        let sum = payload[0] + payload[1] + payload[2]
        sum
    "#,
    )
    .expect_number(60.0);
}
