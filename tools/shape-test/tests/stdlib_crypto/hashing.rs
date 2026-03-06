//! Tests for crypto hashing functions: crypto.sha256, crypto.hmac_sha256.
//!
//! The crypto module is a stdlib module accessed as a global object.
//! The semantic analyzer does not recognize stdlib globals, so these
//! tests are expected to fail at semantic analysis (TDD).

use shape_test::shape_test::ShapeTest;

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_sha256_basic() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let hash = crypto.sha256("hello")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_sha256_known_digest() {
    // TDD: crypto global not recognized by semantic analyzer
    // SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    ShapeTest::new(
        r#"
        let hash = crypto.sha256("hello")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_sha256_empty_string() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let hash = crypto.sha256("")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_hmac_sha256_basic() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let mac = crypto.hmac_sha256("hello", "secret")
        print(mac)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_hmac_sha256_produces_64_hex_chars() {
    // TDD: crypto global not recognized by semantic analyzer
    // HMAC-SHA256 always produces a 64-character hex string (32 bytes)
    ShapeTest::new(
        r#"
        let mac = crypto.hmac_sha256("data", "key")
        let len = mac.length()
        print(len)
    "#,
    )
    .with_stdlib()
    .expect_output("64");
}

// TDD: semantic analyzer doesn't recognize `crypto` as a global
#[test]
fn crypto_sha256_different_inputs_different_hashes() {
    // TDD: crypto global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let h1 = crypto.sha256("hello")
        let h2 = crypto.sha256("world")
        print(h1 != h2)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}
