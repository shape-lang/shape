//! Integration tests for newer crypto functions: sha512, sha1, md5,
//! random_bytes, ed25519 keypair/sign/verify.
//!
//! Basic crypto tests (sha256, hmac_sha256, base64, hex) are in stdlib_crypto/.

use shape_test::shape_test::ShapeTest;

#[test]
fn crypto_sha512_basic() {
    ShapeTest::new(
        r#"
        let hash = crypto.sha512("hello")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043");
}

#[test]
fn crypto_sha512_empty() {
    ShapeTest::new(
        r#"
        let hash = crypto.sha512("")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e");
}

#[test]
fn crypto_sha1_basic() {
    ShapeTest::new(
        r#"
        let hash = crypto.sha1("hello")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
}

#[test]
fn crypto_sha1_empty() {
    ShapeTest::new(
        r#"
        let hash = crypto.sha1("")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("da39a3ee5e6b4b0d3255bfef95601890afd80709");
}

#[test]
fn crypto_md5_basic() {
    ShapeTest::new(
        r#"
        let hash = crypto.md5("hello")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("5d41402abc4b2a76b9719d911017c592");
}

#[test]
fn crypto_md5_empty() {
    ShapeTest::new(
        r#"
        let hash = crypto.md5("")
        print(hash)
    "#,
    )
    .with_stdlib()
    .expect_output("d41d8cd98f00b204e9800998ecf8427e");
}

#[test]
fn crypto_random_bytes_length() {
    ShapeTest::new(
        r#"
        let bytes = crypto.random_bytes(16)
        print(bytes.length())
    "#,
    )
    .with_stdlib()
    .expect_output("32");
}

#[test]
fn crypto_random_bytes_zero() {
    ShapeTest::new(
        r#"
        let bytes = crypto.random_bytes(0)
        print(bytes.length())
    "#,
    )
    .with_stdlib()
    .expect_output("0");
}

#[test]
fn crypto_random_bytes_unique() {
    ShapeTest::new(
        r#"
        let a = crypto.random_bytes(32)
        let b = crypto.random_bytes(32)
        print(a != b)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn crypto_ed25519_keypair_generation() {
    ShapeTest::new(
        r#"
        let kp = crypto.ed25519_generate_keypair()
        let pk = kp.get("public_key")
        let sk = kp.get("secret_key")
        print(pk.length())
        print(sk.length())
    "#,
    )
    .with_stdlib()
    .expect_output("64\n64");
}

#[test]
fn crypto_ed25519_sign_produces_signature() {
    ShapeTest::new(
        r#"
        let kp = crypto.ed25519_generate_keypair()
        let sk = kp.get("secret_key")
        let sig = crypto.ed25519_sign("hello", sk)
        print(sig.length())
    "#,
    )
    .with_stdlib()
    .expect_output("128");
}

#[test]
fn crypto_ed25519_sign_verify_roundtrip() {
    ShapeTest::new(
        r#"
        let kp = crypto.ed25519_generate_keypair()
        let pk = kp.get("public_key")
        let sk = kp.get("secret_key")
        let msg = "test message"
        let sig = crypto.ed25519_sign(msg, sk)
        let valid = crypto.ed25519_verify(msg, sig, pk)
        print(valid)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}

#[test]
fn crypto_ed25519_verify_wrong_message() {
    ShapeTest::new(
        r#"
        let kp = crypto.ed25519_generate_keypair()
        let pk = kp.get("public_key")
        let sk = kp.get("secret_key")
        let sig = crypto.ed25519_sign("correct", sk)
        let valid = crypto.ed25519_verify("wrong", sig, pk)
        print(valid)
    "#,
    )
    .with_stdlib()
    .expect_output("false");
}

#[test]
fn crypto_sha512_different_inputs() {
    ShapeTest::new(
        r#"
        let h1 = crypto.sha512("hello")
        let h2 = crypto.sha512("world")
        print(h1 != h2)
    "#,
    )
    .with_stdlib()
    .expect_output("true");
}
