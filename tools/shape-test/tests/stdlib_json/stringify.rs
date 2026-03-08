//! Tests for json.stringify() functionality.
//!
//! The json module is a stdlib module accessed as a global object.
//! The semantic analyzer does not recognize stdlib globals, so these
//! tests are expected to fail at semantic analysis (TDD).

use shape_test::shape_test::ShapeTest;

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_stringify_number() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.stringify(42)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_stringify_string() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.stringify("hello")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_stringify_boolean() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.stringify(true)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_stringify_null() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.stringify(None)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_stringify_array() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let result = json.stringify(arr)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_stringify_pretty() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.stringify(42, true)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_roundtrip() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let original = "{\"name\":\"test\",\"value\":42}"
        let parsed = json.parse(original)
        let back = json.stringify(parsed)
        print(back)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}
