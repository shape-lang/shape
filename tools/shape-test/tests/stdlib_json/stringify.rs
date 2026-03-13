//! Tests for json::stringify() functionality.
//!
//! The json module is a stdlib module imported via `use std::core::json`.

use shape_test::shape_test::ShapeTest;

#[test]
fn json_stringify_number() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::stringify(42)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_stringify_string() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::stringify("hello")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_stringify_boolean() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::stringify(true)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_stringify_null() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::stringify(None)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_stringify_array() {
    ShapeTest::new(
        r#"
        use std::core::json
        let arr = [1, 2, 3]
        let result = json::stringify(arr)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_stringify_pretty() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::stringify(42, true)
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_roundtrip() {
    ShapeTest::new(
        r#"
        use std::core::json
        let original = "{\"name\":\"test\",\"value\":42}"
        let parsed = json::parse(original)
        let back = json::stringify(parsed)
        print(back)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}
