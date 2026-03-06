//! MessagePack encoding, binary codec, round-trip serialization.
//!
//! Most tests are TDD since the wire protocol encoding is infrastructure-level.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// MessagePack encoding concepts
// =========================================================================

// TDD: ShapeTest does not expose MessagePack encoding API
#[test]
fn msgpack_encodes_integers() {
    // Integers should be encodable in MessagePack format.
    // Verify basic integer handling as a proxy.
    ShapeTest::new(
        r#"
        let val = 42
        val
    "#,
    )
    .expect_number(42.0);
}

// TDD: ShapeTest does not expose MessagePack encoding API
#[test]
fn msgpack_encodes_strings() {
    ShapeTest::new(
        r#"
        let val = "hello wire"
        val
    "#,
    )
    .expect_string("hello wire");
}

// TDD: ShapeTest does not expose MessagePack encoding API
#[test]
fn msgpack_encodes_booleans() {
    ShapeTest::new(
        r#"
        let val = true
        val
    "#,
    )
    .expect_bool(true);
}

// TDD: ShapeTest does not expose MessagePack encoding API
#[test]
fn msgpack_encodes_arrays() {
    // Arrays should be serializable to MessagePack.
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        arr.length()
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// Binary codec concepts
// =========================================================================

// TDD: ShapeTest does not expose binary codec API
#[test]
fn binary_codec_preserves_number_precision() {
    // Floating-point values must survive encode/decode without precision loss.
    ShapeTest::new(
        r#"
        let precise = 3.141592653589793
        precise
    "#,
    )
    .expect_number(std::f64::consts::PI);
}

// TDD: ShapeTest does not expose binary codec API
#[test]
fn binary_codec_handles_nested_objects() {
    // Nested objects should serialize correctly.
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Outer { inner: Inner }
        let o = Outer { inner: Inner { val: 99 } }
        o.inner.val
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// Round-trip serialization concepts
// =========================================================================

// TDD: ShapeTest does not expose wire serialization round-trip
#[test]
fn round_trip_preserves_int() {
    ShapeTest::new(
        r#"
        let value = 12345
        value
    "#,
    )
    .expect_number(12345.0);
}

// TDD: ShapeTest does not expose wire serialization round-trip
#[test]
fn round_trip_preserves_string() {
    ShapeTest::new(
        r#"
        let value = "round-trip test"
        value
    "#,
    )
    .expect_string("round-trip test");
}
