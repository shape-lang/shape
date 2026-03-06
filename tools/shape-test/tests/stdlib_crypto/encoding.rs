//! Tests for crypto encoding functions: base64_encode, base64_decode,
//! hex_encode, hex_decode.
//!
//! The crypto module is a stdlib module accessed as a global object.
//! The semantic analyzer does not recognize stdlib globals (TDD).

use shape_test::shape_test::ShapeTest;

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_base64_encode() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let encoded = crypto.base64_encode("Hello, World!")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_output("SGVsbG8sIFdvcmxkIQ==");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_base64_decode() {
    ShapeTest::new(
        r#"
        let decoded = crypto.base64_decode("SGVsbG8sIFdvcmxkIQ==")
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(Hello, World!)");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_base64_roundtrip() {
    ShapeTest::new(
        r#"
        let original = "Shape language rocks"
        let encoded = crypto.base64_encode(original)
        let decoded = crypto.base64_decode(encoded)
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(Shape language rocks)");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_hex_encode() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let hex = crypto.hex_encode("hello")
        print(hex)
    "#,
    )
    .with_stdlib()
    .expect_output("68656c6c6f");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_hex_decode() {
    ShapeTest::new(
        r#"
        let decoded = crypto.hex_decode("68656c6c6f")
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(hello)");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_hex_roundtrip() {
    ShapeTest::new(
        r#"
        let original = "test data"
        let encoded = crypto.hex_encode(original)
        let decoded = crypto.hex_decode(encoded)
        print(decoded)
    "#,
    )
    .with_stdlib()
    .expect_output("Ok(test data)");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_base64_encode_empty() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let encoded = crypto.base64_encode("")
        print(encoded)
    "#,
    )
    .with_stdlib()
    .expect_output("");
}
