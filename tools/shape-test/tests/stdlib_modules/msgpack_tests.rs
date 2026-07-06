//! Integration tests for the `msgpack` stdlib module via Shape source code.
//!
//! msgpack::encode() returns Result<string> and msgpack::decode() returns Result<any>,
//! so the printed output includes the Ok() wrapper.

use shape_test::shape_test::ShapeTest;

#[test]
fn msgpack_encode_returns_result() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode("test")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn msgpack_encode_decode_string() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode("hello")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn msgpack_encode_decode_number() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode(42)
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn msgpack_encode_decode_bool() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode(true)
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn msgpack_encode_decode_array() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode([1, 2, 3])
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn msgpack_encode_bytes_returns_result() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let encoded = msgpack::encode_bytes("test")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn msgpack_encode_produces_hex_string() {
    // msgpack::encode returns Ok(hex_string), verify it runs
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let result = msgpack::encode("hello")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// --- WF-3A-tail regression: msgpack decode navigation (#17) ---
// Shares the json #16 root: decode returns Result<Json>, and before the fix the
// matched Ok payload stayed untyped so navigating the decoded Json enum
// (is_null / match Json::Int) failed. Assert encode -> decode -> navigate round-trips.
#[test]
fn msgpack_decode_navigates_int() {
    ShapeTest::new(
        r#"
        use std::core::msgpack
        let enc = msgpack::encode(42)
        match enc {
            Ok(hex) => {
                let dec = msgpack::decode(hex)
                match dec {
                    Ok(v) => {
                        print(v.is_null())
                        match v {
                            Json::Int(i) => print(i),
                            _ => print("not int"),
                        }
                    },
                    Err(e) => print(e),
                }
            },
            Err(e) => print(e),
        }
    "#,
    )
    .with_stdlib()
    .expect_output_contains("false")
    .expect_output_contains("42");
}
