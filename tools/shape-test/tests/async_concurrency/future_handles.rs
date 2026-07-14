//! Static Future<T> surface tests for async let handles.

use shape_test::shape_test::ShapeTest;

#[test]
fn async_let_handle_can_be_annotated_as_future_and_awaited() {
    let code = r#"
async fn run() {
    async let f = 41
    let h: Future<int> = f
    let n: int = await h
    print(n + 1)
}

await run()
"#;

    ShapeTest::new(code).expect_output("42");
}

#[test]
fn async_let_handle_does_not_assign_to_payload_type() {
    let code = r#"
async fn run() {
    async let f = 41
    let n: int = f
    print(n)
}

await run()
"#;

    ShapeTest::new(code).expect_run_err_contains_any(&["Future", "TypeMismatch"]);
}

#[test]
fn awaited_payload_does_not_assign_to_future_type() {
    let code = r#"
async fn run() {
    async let f = 41
    let h: Future<int> = await f
    print(await h)
}

await run()
"#;

    ShapeTest::new(code).expect_run_err_contains_any(&["Future", "TypeMismatch"]);
}
