//! LSP foreign language delegation tests: Python and TypeScript body delegation.
//! TDD: Foreign LSP delegation not implemented — tests verify that code containing
//! foreign-style constructs does not crash the LSP and basic Shape features still work.

use shape_test::shape_test::{ShapeTest, pos};

// == Python body delegation ===================================================

#[test]
fn python_function_shape_wrapper_hover() {
    // TDD: foreign LSP delegation not implemented; testing Shape function wrapper hover
    let code = "\
fn py_transform(data: any) -> any {
    return data
}
let result = py_transform([1, 2, 3]);
";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("py_transform");
}

#[test]
fn python_function_wrapper_definition() {
    // TDD: foreign LSP delegation not implemented; testing go-to-definition
    let code = "\
fn py_compute(x: int) -> int {
    return x * 2
}
let val = py_compute(10);
";
    ShapeTest::new(code).at(pos(3, 12)).expect_definition();
}

#[test]
fn python_function_wrapper_completions() {
    // TDD: foreign LSP delegation not implemented; testing completions
    let code = "\
fn py_analyze(data: any) -> any { return data }
py_a
";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_completion("py_analyze");
}

// == TypeScript body delegation ===============================================

#[test]
fn typescript_function_shape_wrapper_hover() {
    // TDD: foreign LSP delegation not implemented; testing Shape function wrapper hover
    let code = "\
fn ts_format(template: string) -> string {
    return template
}
let msg = ts_format(\"hello\");
";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("ts_format");
}

#[test]
fn typescript_function_wrapper_references() {
    // TDD: foreign LSP delegation not implemented; testing find-references
    let code = "\
fn ts_render(html: string) -> string { return html }
let a = ts_render(\"<div>\");
let b = ts_render(\"<span>\");
";
    ShapeTest::new(code).at(pos(0, 4)).expect_references_min(2);
}

#[test]
fn typescript_function_wrapper_rename() {
    // TDD: foreign LSP delegation not implemented; testing rename
    let code = "\
fn ts_render(html: string) -> string { return html }
let a = ts_render(\"<div>\");
let b = ts_render(\"<span>\");
";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_rename_edits("ts_display", 3);
}

// == Foreign-target annotation rejections surface in the editor ================

/// `#74 INTERIM REJECTION` (ADR-009 E4-D5, slice S2) — the foreign-target
/// comptime rejection must reach the EDITOR, not just `shape run`. The
/// compiler's `SemanticError` flows through `shape-lsp`'s `analysis.rs`
/// (`compile_in_place` under `RecoverAll` → `error_to_diagnostic_with_uri`),
/// so the squiggle lands on the `@application` line — line 6, 0-based.
#[test]
fn comptime_annotation_on_extern_c_fn_surfaces_lsp_diagnostic_at_the_application_line() {
    let code = "\
annotation marked() on function {
  comptime post(target, ctx) {
    1
  }
}

@marked()
extern \"C\" fn labs(x: int) -> int from \"c\"

print(\"unreachable\")
";
    ShapeTest::new(code).expect_semantic_diagnostic_at_line_contains(6, "see issue #74");
}
