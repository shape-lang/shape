//! Tests for the http stdlib module.
//!
//! All HTTP functions are async and require network access. These tests
//! are TDD since they need actual network connectivity and the semantic
//! analyzer doesn't recognize `http` as a global.

use shape_test::shape_test::ShapeTest;

// TDD: requires network access
#[test]
fn http_get_basic() {
    ShapeTest::new(
        r#"
        let response = http.get("https://httpbin.org/get")
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
        let response = http.post("https://httpbin.org/post", "hello")
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
        let response = http.put("https://httpbin.org/put", "data")
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
        let response = http.delete("https://httpbin.org/delete")
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
        let body = "{\"key\": \"value\"}"
        let response = http.post("https://httpbin.org/post", body)
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
        let response = http.get("not-a-valid-url")
        print(response)
    "#,
    )
    .with_stdlib()
    .expect_run_err();
}
