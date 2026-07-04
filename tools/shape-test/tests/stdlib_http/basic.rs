//! Tests for the http stdlib module.
//!
//! All HTTP functions are async and require network access. These tests
//! use `use std::core::http` to import the http module.

use shape_test::shape_test::ShapeTest;

// TDD: requires network access
#[test]
fn http_get_basic() {
    ShapeTest::new(
        r#"
        use std::core::http
        let response = http::get("https://httpbin.org/get", HashMap())
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: requires network access
#[test]
fn http_post_basic() {
    ShapeTest::new(
        r#"
        use std::core::http
        let response = http::post_text("https://httpbin.org/post", "hello", HashMap())
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: requires network access
#[test]
fn http_put_basic() {
    ShapeTest::new(
        r#"
        use std::core::http
        let response = http::put_text("https://httpbin.org/put", "data", HashMap())
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: requires network access
#[test]
fn http_delete_basic() {
    ShapeTest::new(
        r#"
        use std::core::http
        let response = http::delete("https://httpbin.org/delete", HashMap())
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: requires network access
#[test]
fn http_post_with_json_body() {
    ShapeTest::new(
        r#"
        use std::core::http
        let body = { key: "value" }
        let response = http::post_json("https://httpbin.org/post", body, HashMap())
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_ok();
}

// TDD: invalid URL should produce an error
#[test]
fn http_get_with_invalid_url() {
    ShapeTest::new(
        r#"
        use std::core::http
        let response = http::get("not-a-valid-url", HashMap())
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_err();
}
