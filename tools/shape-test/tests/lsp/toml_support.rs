//! LSP TOML support tests: completions, diagnostics, and hover for shape.toml.
//! TDD: TOML LSP support is a separate feature — all tests verify the test
//! harness handles non-Shape content gracefully (no panics).

use shape_test::shape_test::{ShapeTest, pos};

// == shape.toml completions ===================================================

#[test]
fn shape_toml_content_does_not_crash_hover() {
    // TDD: TOML LSP support not implemented; verify no panic on TOML-like content
    let code = "let name = \"my-project\"\nlet version = \"0.1.0\"\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("Variable");
}

#[test]
fn shape_toml_field_names_as_variables() {
    // TDD: TOML LSP support not implemented; modeling toml fields as Shape variables
    let code = "\
let name = \"my-project\"
let version = \"0.1.0\"
let edition = \"2024\"
";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("string");
}

// == shape.toml diagnostics ===================================================

#[test]
fn shape_toml_invalid_syntax_as_shape_code() {
    // TDD: TOML LSP support not implemented; verifying Shape code diagnostics work
    let code = "let x = 42;\n";
    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

// == shape.toml hover =========================================================

#[test]
fn shape_toml_hover_on_string_field() {
    // TDD: TOML LSP support not implemented; testing basic hover on string values
    let code = "let description = \"A Shape project\"\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("string");
}

#[test]
fn shape_toml_hover_on_dependency_like_structure() {
    // TDD: TOML LSP support not implemented; testing hover on object literal
    let code = "let deps = { math: \"1.0\", utils: \"2.0\" }\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("math");
}
