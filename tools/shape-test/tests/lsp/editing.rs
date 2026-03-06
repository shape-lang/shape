//! LSP editing tests: formatting, rename, code actions, code lens,
//! and format-on-type.

use shape_test::shape_test::{ShapeTest, pos, range};

// == Formatting (from lsp_editing) ===========================================

#[test]
fn format_function_preserves_body() {
    ShapeTest::new("function add(a, b) { return a + b; }").expect_format_preserves("return");
}

#[test]
fn format_nested_blocks_produces_indentation() {
    let code = "function test() {\nif (true) {\nlet x = 1;\n}\n}\n";
    ShapeTest::new(code).expect_format_has_indentation();
}

#[test]
fn format_preserves_content() {
    ShapeTest::new("let a = 1;\n\nlet b = 2;\n")
        .expect_format_preserves("let a")
        .expect_format_preserves("let b");
}

#[test]
fn format_on_type_after_newline_in_block() {
    ShapeTest::new("fn test() {\n")
        .at(pos(1, 0))
        .expect_format_on_type("\n");
}

// == Formatting: comment preservation (from lsp_presentation) ================

#[test]
fn format_preserves_line_comments() {
    let code = "let x = 1\n// middle comment\nlet y = 2\n";
    ShapeTest::new(code).expect_format_preserves("// middle comment");
}

#[test]
fn format_preserves_standalone_comments() {
    let code = "// header\nlet x = 1\n";
    ShapeTest::new(code).expect_format_preserves("// header");
}

#[test]
fn format_preserves_block_comments() {
    let code = "/* block */\nlet x = 1\n";
    ShapeTest::new(code).expect_format_preserves("/* block */");
}

#[test]
fn format_preserves_comments_in_function() {
    let code = "fn test() {\n    let x = 1\n    // inside comment\n    let y = 2\n}\n";
    ShapeTest::new(code).expect_format_preserves("// inside comment");
}

// == Formatting deep (from programs_lsp_completeness) ========================

#[test]
fn test_lsp_format_preserves_comments() {
    let code = "// This is a comment\nlet x = 1\n";
    ShapeTest::new(code).expect_format_preserves("// This is a comment");
}

#[test]
fn test_lsp_format_preserves_function_body() {
    let code = "fn test() {\n    let x = 1\n    return x\n}\n";
    ShapeTest::new(code).expect_format_has_indentation();
}

#[test]
fn test_lsp_format_on_type_no_crash() {
    ShapeTest::new("let x = 42\n")
        .at(pos(0, 10))
        .expect_format_on_type(";");
}

// == Rename (from lsp_editing) ===============================================

#[test]
fn rename_variable_updates_all_occurrences() {
    ShapeTest::new("let myVar = 42;\nlet x = myVar + 5;\nlet y = myVar * 2;\n")
        .at(pos(0, 5))
        .expect_rename_edits("newName", 3);
}

#[test]
fn rename_function_updates_definition_and_call_sites() {
    let code =
        "function myFunc(x) {\n    return x + 1;\n}\nlet a = myFunc(5);\nlet b = myFunc(10);\n";
    ShapeTest::new(code)
        .at(pos(0, 10))
        .expect_rename_edits("newFunc", 3);
}

#[test]
fn prepare_rename_on_keyword_returns_none() {
    ShapeTest::new("let x = 42;")
        .at(pos(0, 1))
        .expect_prepare_rename_none();
}

#[test]
fn prepare_rename_on_builtin_returns_none() {
    ShapeTest::new("let x = abs(-5);")
        .at(pos(0, 9))
        .expect_prepare_rename_none();
}

// == Code actions (from lsp_editing) =========================================

#[test]
fn code_actions_on_range_does_not_crash() {
    ShapeTest::new("let x = 42;\nlet y = 10;\n")
        .in_range(range(0, 0, 1, 11))
        .expect_code_actions_ok();
}

#[test]
fn code_actions_on_expression_does_not_crash() {
    ShapeTest::new("let x = 42 + 10;")
        .in_range(range(0, 8, 0, 15))
        .expect_code_actions_ok();
}

// == Code lens (from lsp_editing) ============================================

#[test]
fn code_lens_on_function_shows_reference_count() {
    let code = "function myFunc() {\n    return 1;\n}\nlet a = myFunc();\nlet b = myFunc();\n";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_at_line(0);
}

#[test]
fn code_lens_shows_reference_info() {
    let code = "function helper() {\n    return 1;\n}\nlet a = helper();\n";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_has_commands();
}
