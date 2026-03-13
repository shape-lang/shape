//! Tests for crypto encoding functions: base64_encode, base64_decode,
//! hex_encode, hex_decode.
//!
//! The crypto module is a stdlib module imported via `use std::core::crypto`.

use shape_test::shape_test::ShapeTest;

#[test]
fn crypto_base64_encode() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let encoded = crypto::base64_encode("Hello, World!")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_output("SGVsbG8sIFdvcmxkIQ==");
}

#[test]
fn crypto_base64_decode() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let decoded = crypto::base64_decode("SGVsbG8sIFdvcmxkIQ==")
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(Hello, World!)");
}

#[test]
fn crypto_base64_roundtrip() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let original = "Shape language rocks"
        let encoded = crypto::base64_encode(original)
        let decoded = crypto::base64_decode(encoded)
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(Shape language rocks)");
}

#[test]
fn crypto_hex_encode() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let hex = crypto::hex_encode("hello")
        print(hex)
    "#,
    )
    .with_stdlib()
    .expect_output("68656c6c6f");
}

#[test]
fn crypto_hex_decode() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let decoded = crypto::hex_decode("68656c6c6f")
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(hello)");
}

#[test]
fn crypto_hex_roundtrip() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let original = "test data"
        let encoded = crypto::hex_encode(original)
        let decoded = crypto::hex_decode(encoded)
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(test data)");
}

#[test]
fn crypto_base64_encode_empty() {
    ShapeTest::new(
        r#"
        use std::core::crypto
        let encoded = crypto::base64_encode("")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_output("");
}
