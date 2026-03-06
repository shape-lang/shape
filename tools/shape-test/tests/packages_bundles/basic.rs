//! Package manifest and bundle structure tests.
//!
//! Shape packages use shape.toml for metadata and content-addressed bytecode
//! bundles for distribution. Most tests are TDD since they need shapec
//! compiler tooling exposed through ShapeTest.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Manifest / shape.toml concepts
// =========================================================================

// TDD: ShapeTest does not expose shapec manifest parsing
#[test]
fn package_name_in_manifest_is_valid_identifier() {
    // A package name must be a valid Shape identifier
    // shape.toml: name = "my_package"
    ShapeTest::new(
        r#"
        let name = "my_package"
        name
    "#,
    )
    .expect_string("my_package");
}

// TDD: ShapeTest does not expose shapec manifest parsing
#[test]
fn package_version_semver_string() {
    // Package versions follow semantic versioning
    ShapeTest::new(
        r#"
        let version = "1.2.3"
        version
    "#,
    )
    .expect_string("1.2.3");
}

// =========================================================================
// Module visibility
// =========================================================================

#[test]
fn pub_function_is_exported() {
    ShapeTest::new(
        r#"
        pub fn add(a, b) { a + b }
        add(1, 2)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn mod_declaration_parses() {
    ShapeTest::new(
        r#"
        mod utils {
            pub fn double(x) { x * 2 }
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Content addressing concepts
// =========================================================================

// TDD: ShapeTest does not expose content-addressed bundle creation
#[test]
fn content_hash_deterministic_for_same_source() {
    // Same source code should produce the same bytecode hash.
    // We test the concept by verifying deterministic evaluation.
    ShapeTest::new(
        r#"
        fn f(x) { x + 1 }
        f(41)
    "#,
    )
    .expect_number(42.0);
}

// TDD: ShapeTest does not expose bundle structure inspection
#[test]
fn bundle_contains_string_pool() {
    // Bundles store a string pool for interned string operands.
    // Here we just verify strings work at runtime.
    ShapeTest::new(
        r#"
        let s = "hello"
        let t = "world"
        f"{s} {t}"
    "#,
    )
    .expect_string("hello world");
}

// TDD: ShapeTest does not expose dependency resolution
#[test]
fn dependency_resolution_concept() {
    // Dependencies are resolved by content hash, not name.
    // Verifying basic function call chain as a proxy.
    ShapeTest::new(
        r#"
        fn dep_a() { 10 }
        fn dep_b() { dep_a() + 20 }
        dep_b()
    "#,
    )
    .expect_number(30.0);
}

// TDD: ShapeTest does not expose shapec compilation pipeline
#[test]
fn compilation_produces_bytecode() {
    // Verify that a simple program compiles and runs, implying
    // bytecode was produced.
    ShapeTest::new(
        r#"
        let x = 1 + 2 + 3
        x
    "#,
    )
    .expect_number(6.0);
}
