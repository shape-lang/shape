//! LSP code action tests: quickfix, organize imports, refactor suggestions.

use shape_test::shape_test::{ShapeTest, range};

// == Quickfix actions =========================================================

#[test]
fn quickfix_actions_on_code_range() {
    // TDD: quickfix actions may not be fully implemented for all diagnostics
    let code = "let x = 42;\nlet y = x + 1;\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 1, 14))
        .expect_code_actions_ok();
}

#[test]
fn quickfix_on_single_expression() {
    let code = "let result = 10 + 20;\n";
    ShapeTest::new(code)
        .in_range(range(0, 13, 0, 20))
        .expect_code_actions_ok();
}

// == Organize imports =========================================================

#[test]
fn organize_imports_does_not_crash() {
    // TDD: organize imports not fully implemented; verifies no crash on range with imports
    let code = "let x = 1;\nlet y = 2;\nlet z = x + y;\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 2, 14))
        .expect_code_actions_ok();
}

// == Refactor extract =========================================================

#[test]
fn refactor_extract_on_block_does_not_crash() {
    // TDD: extract-function refactoring not yet implemented
    let code = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = a + b;\n}\n";
    ShapeTest::new(code)
        .in_range(range(1, 4, 3, 18))
        .expect_code_actions_ok();
}

#[test]
fn code_actions_on_function_definition() {
    let code = "function helper() {\n    return 42;\n}\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 2, 1))
        .expect_code_actions_ok();
}
