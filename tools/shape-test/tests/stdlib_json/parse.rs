//! Tests for json.parse() functionality.
//!
//! The json module is a stdlib module accessed as a global object.
//! The semantic analyzer does not recognize stdlib globals, so these
//! tests are expected to fail at semantic analysis (TDD).

use shape_test::shape_test::ShapeTest;

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_number() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("42")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_string_value() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("\"hello\"")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_boolean() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("true")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_null() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("null")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_array() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("[1, 2, 3]")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_object() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("{\"name\": \"test\", \"value\": 42}")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_nested_object() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("{\"outer\": {\"inner\": [1, 2]}}")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_parse_invalid_json_error() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let result = json.parse("{invalid json}")
        print(result)
    "#,
    )
    .with_stdlib()
    .expect_run_err_contains("parse");
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_is_valid_true() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let valid = json.is_valid("{\"key\": \"value\"}")
        print(valid)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: semantic analyzer doesn't recognize `json` as a global
#[test]
fn json_is_valid_false() {
    // TDD: json global not recognized by semantic analyzer
    ShapeTest::new(
        r#"
        let valid = json.is_valid("{not valid}")
        print(valid)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}
