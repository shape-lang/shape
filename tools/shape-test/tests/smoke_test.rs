//! Smoke test: verify ShapeTest can chain LSP + runtime assertions on the same code.

use shape_test::shape_test::{ShapeTest, pos};

#[test]
fn lsp_and_runtime_combined() {
    ShapeTest::new("let x = 1 + 2\nx\n")
        .at(pos(0, 4))
        .expect_hover_contains("Variable")
        .expect_semantic_tokens()
        .expect_run_ok()
        .expect_number(3.0);
}

#[test]
fn function_hover_and_execution() {
    let code = "function double(n) { return n * 2; }\ndouble(21)\n";
    ShapeTest::new(code)
        .at(pos(0, 9))
        .expect_hover_contains("double")
        .expect_run_ok()
        .expect_number(42.0);
}

#[test]
fn parse_error_detected() {
    ShapeTest::new("let = ;").expect_parse_err();
}

#[test]
fn bool_result() {
    ShapeTest::new("1 < 2").expect_run_ok().expect_bool(true);
}

#[test]
fn output_capture_single_line() {
    ShapeTest::new("print(\"Hello, Shape!\")").expect_output("Hello, Shape!");
}

#[test]
fn output_capture_multiline() {
    ShapeTest::new("print(1 + 2)\nprint(4 + 5)").expect_output("3\n9");
}

#[test]
fn output_contains_substring() {
    ShapeTest::new("print(\"hello world\")").expect_output_contains("world");
}

#[test]
fn typed_object_property_assignment() {
    ShapeTest::new("let a = { x: 10 }\na.y = 2\nprint(a.y)")
        .expect_run_ok()
        .expect_output("2");
}
