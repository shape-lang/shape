//! Cross-feature integration tests.
//!
//! Tests that combine multiple LSP features and runtime assertions
//! to verify they work correctly together on the same code.

use shape_test::shape_test::{ShapeTest, pos, range};

// -- LSP feature combinations -----------------------------------------------

#[test]
fn hover_and_completions_work_together() {
    let code = "let x = 42;\nlet y = x + 1;";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("x")
        .at(pos(1, 8))
        .expect_completions_not_empty();
}

#[test]
fn semantic_tokens_and_hover_on_function() {
    let code = "function add(a, b) { return a + b; }";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .at(pos(0, 9))
        .expect_hover_contains("add");
}

#[test]
fn semantic_tokens_and_hover_on_trait() {
    let code = "trait Printable {\n    display(self): string\n}";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn definition_and_references_consistent() {
    // If go-to-def works, references should also work from the same position
    let code = "let x = 42;\nlet y = x + 1;\nlet z = x * 2;";
    ShapeTest::new(code)
        .at(pos(1, 8))
        .expect_definition()
        .expect_references_min(2);
}

#[test]
fn hover_definition_and_rename_on_variable() {
    let code = "let myVar = 42;\nlet y = myVar + 1;";
    ShapeTest::new(code)
        .at(pos(0, 5))
        .expect_hover_contains("myVar")
        .expect_definition()
        .expect_rename_edits("newVar", 2);
}

// -- LSP + runtime combined -------------------------------------------------

#[test]
fn lsp_and_runtime_on_arithmetic() {
    ShapeTest::new("let x = 10 + 32\nx\n")
        .at(pos(0, 4))
        .expect_hover_contains("Variable")
        .expect_semantic_tokens()
        .expect_run_ok()
        .expect_number(42.0);
}

#[test]
fn lsp_and_runtime_on_function_call() {
    let code = "function double(n) { return n * 2; }\ndouble(21)\n";
    ShapeTest::new(code)
        .at(pos(0, 9))
        .expect_hover_contains("double")
        .expect_semantic_tokens()
        .expect_run_ok()
        .expect_number(42.0);
}

#[test]
fn lsp_and_runtime_on_string() {
    ShapeTest::new("let s = \"hello\"\ns\n")
        .at(pos(0, 4))
        .expect_hover_contains("Variable")
        .expect_run_ok()
        .expect_string("hello");
}

#[test]
fn lsp_and_runtime_on_bool() {
    ShapeTest::new("let b = 1 < 2\nb\n")
        .at(pos(0, 4))
        .expect_hover_contains("Variable")
        .expect_run_ok()
        .expect_bool(true);
}

#[test]
fn lsp_and_runtime_on_output() {
    ShapeTest::new("print(\"hello world\")")
        .expect_semantic_tokens()
        .expect_run_ok()
        .expect_output("hello world");
}

// -- Formatting + other features --------------------------------------------

#[test]
fn formatting_and_semantic_tokens() {
    let code = "function test() {\n    let x = 1;\n    return x;\n}\n";
    ShapeTest::new(code)
        .expect_format_preserves("return")
        .expect_semantic_tokens();
}

#[test]
fn code_actions_and_code_lens() {
    let code = "function myFunc() {\n    return 1;\n}\nlet a = myFunc();\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 2, 1))
        .expect_code_actions_ok()
        .expect_code_lens_not_empty();
}

// -- Complex programs -------------------------------------------------------

#[test]
fn full_program_lsp_coverage() {
    let code = r#"let x = 42;
const PI = 3.14;
function add(a, b) { return a + b; }
let result = add(x, 10);
print(result);
"#;
    ShapeTest::new(code)
        .expect_parse_ok()
        .expect_semantic_tokens()
        .expect_document_symbols()
        .at(pos(2, 9))
        .expect_hover_contains("add")
        .at(pos(3, 13))
        .expect_definition()
        .expect_run_ok()
        .expect_output("52");
}

#[test]
fn trait_program_parses_and_has_tokens() {
    let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    method select(cols) { self }\n}\n";
    ShapeTest::new(code)
        .expect_parse_ok()
        .expect_semantic_tokens();
}

// -- Edge cases -------------------------------------------------------------

#[test]
fn empty_program_has_no_symbols() {
    ShapeTest::new("").expect_no_document_symbols();
}

#[test]
fn single_expression_runs_ok() {
    ShapeTest::new("42").expect_run_ok().expect_number(42.0);
}

#[test]
fn multiline_output_integration() {
    ShapeTest::new("print(1)\nprint(2)\nprint(3)")
        .expect_run_ok()
        .expect_output("1\n2\n3");
}
