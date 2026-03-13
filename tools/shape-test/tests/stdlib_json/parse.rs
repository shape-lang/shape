//! Tests for json::parse() functionality.
//!
//! The json module is a stdlib module imported via `use std::core::json`.

use shape_test::shape_test::ShapeTest;

#[test]
fn json_parse_number() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("42")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_string_value() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("\"hello\"")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_boolean() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("true")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_null() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("null")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_array() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("[1, 2, 3]")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_object() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("{\"name\": \"test\", \"value\": 42}")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_nested_object() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("{\"outer\": {\"inner\": [1, 2]}}")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_parse_invalid_json_error() {
    ShapeTest::new(
        r#"
        use std::core::json
        let result = json::parse("{invalid json}")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_err_contains("parse");
}

#[test]
fn json_is_valid_true() {
    ShapeTest::new(
        r#"
        use std::core::json
        let valid = json::is_valid("{\"key\": \"value\"}")
        print(valid)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

#[test]
fn json_is_valid_false() {
    ShapeTest::new(
        r#"
        use std::core::json
        let valid = json::is_valid("{not valid}")
        print(valid)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}
