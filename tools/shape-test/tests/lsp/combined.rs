//! Combined LSP + runtime tests: verifying that LSP features (hover, inlay hints,
//! semantic tokens, diagnostics) work correctly alongside runtime execution.

use shape_test::shape_test::{ShapeTest, pos};

#[test]
fn test_lsp_combined_hover_completions_run() {
    let code = "type Point { x: int, y: int }\nlet p = Point { x: 3, y: 4 }\np.x + p.y\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_hover_contains("Point")
        .at(pos(2, 2))
        .expect_hover_contains("Property")
        .expect_number(7.0);
}

#[test]
fn test_lsp_combined_function_hover_and_run() {
    let code = "fn factorial(n: int) -> int {\n  if n <= 1 { return 1 }\n  return n * factorial(n - 1)\n}\nfactorial(5)\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("factorial")
        .expect_number(120.0);
}

#[test]
fn test_lsp_combined_inlay_and_run() {
    let code = "let x = 10\nlet y = 20\nx + y\n";
    ShapeTest::new(code)
        .expect_type_hint_label(": int")
        .expect_number(30.0);
}

#[test]
fn test_lsp_combined_string_hover_and_run() {
    let code = "let greeting = \"hello\"\ngreeting\n";
    ShapeTest::new(code)
        .at(pos(0, 5))
        .expect_hover_contains("string")
        .expect_string("hello");
}

#[test]
fn test_lsp_combined_diagnostics_and_error() {
    let code = "fn bad(c) {\n  c = c + 1\n  return c\n}\nlet obj = { x: 1 }\nbad(obj)\nbad(1)\n";
    ShapeTest::new(code).expect_semantic_diagnostic_contains("Could not solve type constraints");
}

#[test]
fn test_lsp_combined_enum_match_run() {
    let code = "enum Shape { Circle(number), Square(number) }\nlet s = Shape::Circle(5.0)\nmatch s {\n  Shape::Circle(r) => r * r * 3.14\n  Shape::Square(side) => side * side\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_number(78.5);
}

#[test]
fn test_lsp_combined_all_features_simple() {
    let code = "let x = 42\nlet y = x + 8\ny\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("int")
        .expect_type_hint_label(": int")
        .expect_semantic_tokens()
        .expect_no_semantic_diagnostics()
        .expect_number(50.0);
}

#[test]
fn test_lsp_combined_closure_inlay_and_run() {
    let code = "let double = |x| x * 2\ndouble(21)\n";
    ShapeTest::new(code)
        .expect_inlay_hints_not_empty()
        .expect_number(42.0);
}

#[test]
fn test_lsp_combined_module_hover_and_parse() {
    let code = "mod math {\n  fn add(a: int, b: int) -> int {\n    return a + b\n  }\n}\nmath.add(10, 32)\n";
    ShapeTest::new(code)
        .at(pos(5, 1))
        .expect_hover_contains("math")
        .expect_parse_ok();
}

#[test]
fn test_lsp_combined_output_and_diagnostics() {
    let code = "let msg = \"hello world\"\nprint(msg)\nmsg\n";
    ShapeTest::new(code)
        .at(pos(0, 5))
        .expect_hover_contains("string")
        .expect_no_semantic_diagnostics()
        .expect_output("hello world")
        .expect_string("hello world");
}
