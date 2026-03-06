//! LSP navigation tests: go-to-definition, find references, rename (scope-aware),
//! and prepare-rename safety checks.

use shape_test::shape_test::{ShapeTest, pos};

// == Go-to-definition (from lsp_analysis) ====================================

#[test]
fn goto_def_on_function_call_jumps_to_definition() {
    let code = "function myFunc(x, y) {\n    return x + y;\n}\n\nlet result = myFunc(1, 2);\n";
    ShapeTest::new(code).at(pos(4, 15)).expect_definition();
}

#[test]
fn goto_def_on_variable_usage_jumps_to_let() {
    ShapeTest::new("let myVar = 42;\nlet x = myVar + 5;\n")
        .at(pos(1, 10))
        .expect_definition();
}

// == Find references: simple (from lsp_analysis) ============================

#[test]
fn find_references_on_function_returns_all_call_sites() {
    let code = "function myFunc(x, y) {\n    return x + y;\n}\n\nlet a = myFunc(1, 2);\nlet b = myFunc(3, 4);\n";
    ShapeTest::new(code).at(pos(0, 10)).expect_references_min(2);
}

#[test]
fn find_references_on_variable_returns_all_usages() {
    ShapeTest::new("let myVar = 42;\nlet x = myVar + 5;\nlet y = myVar * 2;\n")
        .at(pos(0, 5))
        .expect_references_min(2);
}

// == Find references: scope-aware (from lsp_references) ======================

#[test]
fn find_references_simple_variable() {
    ShapeTest::new("let x = 1;\nlet y = x + 2;\nlet z = x * 3;")
        .at(pos(0, 4))
        .expect_references_min(3);
}

#[test]
fn find_references_function_name() {
    ShapeTest::new("function foo() { return 1; }\nlet x = foo();")
        .at(pos(0, 9))
        .expect_references_min(2);
}

#[test]
fn find_references_single_use() {
    ShapeTest::new("let x = 42;\nlet y = x;")
        .at(pos(0, 4))
        .expect_references_min(2);
}

#[test]
fn find_references_from_usage_site() {
    ShapeTest::new("let x = 1;\nlet y = x + 2;")
        .at(pos(1, 8))
        .expect_references_min(2);
}

#[test]
fn find_references_respects_scope() {
    let code = "let x = 1;\nfunction foo() {\n    let x = 2;\n    return x;\n}\nlet y = x;";
    ShapeTest::new(code).at(pos(2, 8)).expect_references_min(2);
}

#[test]
fn find_references_outer_variable_across_function() {
    let code = "let x = 1;\nfunction foo() {\n    let x = 2;\n    return x;\n}\nlet y = x;";
    ShapeTest::new(code).at(pos(0, 4)).expect_references_min(2);
}

#[test]
fn find_references_function_parameter() {
    let code = "function add(a, b) {\n    return a + b;\n}\n";
    ShapeTest::new(code).at(pos(0, 13)).expect_references_min(2);
}

#[test]
fn find_references_multiple_functions() {
    let code =
        "function a() { return 1; }\nfunction b() { return a(); }\nlet x = a();\nlet y = b();\n";
    ShapeTest::new(code).at(pos(0, 9)).expect_references_min(3);
}

// == Rename: scope-aware (from lsp_references) ===============================

#[test]
fn rename_simple_variable() {
    ShapeTest::new("let x = 1;\nlet y = x + 2;")
        .at(pos(0, 4))
        .expect_rename_edits("foo", 2);
}

#[test]
fn rename_variable_multiple_uses() {
    ShapeTest::new("let x = 1;\nlet y = x + 2;\nlet z = x * 3;")
        .at(pos(0, 4))
        .expect_rename_edits("renamed", 3);
}

#[test]
fn rename_function_name() {
    ShapeTest::new("function foo() { return 1; }\nlet x = foo();")
        .at(pos(0, 9))
        .expect_rename_edits("bar", 2);
}

#[test]
fn rename_does_not_affect_other_scope() {
    let code = "let x = 1;\nfunction foo() {\n    let x = 2;\n    return x;\n}\nlet y = x;";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_rename_edits("z", 2);
}

#[test]
fn rename_inner_variable_scoped() {
    let code = "let x = 1;\nfunction foo() {\n    let x = 2;\n    return x;\n}\nlet y = x;";
    ShapeTest::new(code)
        .at(pos(2, 8))
        .expect_rename_edits("inner", 2);
}

// == Rename: safety checks (from lsp_references) ============================

#[test]
fn prepare_rename_on_keyword_is_none() {
    ShapeTest::new("let x = 42;")
        .at(pos(0, 1))
        .expect_prepare_rename_none();
}

#[test]
fn prepare_rename_on_builtin_is_none() {
    ShapeTest::new("let x = abs(-5);")
        .at(pos(0, 9))
        .expect_prepare_rename_none();
}

// == Navigation deep (from programs_lsp_completeness) ========================

#[test]
fn test_lsp_nav_goto_def_variable() {
    let code = "let myVar = 42\nlet x = myVar + 1\n";
    ShapeTest::new(code).at(pos(1, 10)).expect_definition();
}

#[test]
fn test_lsp_nav_goto_def_function_call() {
    let code = "fn compute(x: int) -> int { return x * 2 }\nlet r = compute(5)\n";
    ShapeTest::new(code).at(pos(1, 10)).expect_definition();
}

#[test]
fn test_lsp_nav_goto_def_type_reference() {
    let code = "type Widget { id: int }\nlet w = Widget { id: 1 }\n";
    ShapeTest::new(code).at(pos(1, 10)).expect_definition();
}

#[test]
fn test_lsp_nav_goto_def_trait_in_impl() {
    let code = "trait Render {\n  draw(): any\n}\nimpl Render for Canvas {\n  method draw() { \"drawn\" }\n}\n";
    ShapeTest::new(code).at(pos(3, 5)).expect_definition();
}

#[test]
fn test_lsp_nav_references_variable() {
    let code = "let val = 10\nlet a = val + 1\nlet b = val * 2\n";
    ShapeTest::new(code).at(pos(0, 5)).expect_references_min(2);
}

#[test]
fn test_lsp_nav_references_function() {
    let code = "fn helper(x: int) -> int { return x }\nlet a = helper(1)\nlet b = helper(2)\nlet c = helper(3)\n";
    ShapeTest::new(code).at(pos(0, 4)).expect_references_min(3);
}

#[test]
fn test_lsp_nav_rename_variable() {
    let code = "let counter = 0\nlet a = counter + 1\nlet b = counter * 2\n";
    ShapeTest::new(code)
        .at(pos(0, 5))
        .expect_rename_edits("cnt", 3);
}

#[test]
fn test_lsp_nav_rename_function() {
    let code = "fn calc(x: int) -> int { return x }\nlet a = calc(1)\nlet b = calc(2)\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_rename_edits("compute", 3);
}
