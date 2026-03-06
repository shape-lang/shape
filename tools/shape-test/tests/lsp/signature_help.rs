//! LSP signature help tests: parameter hints inside function call parentheses.

use shape_test::shape_test::{ShapeTest, pos};

// == Basic function signature =================================================

#[test]
fn basic_function_signature_shows_help() {
    let code = "fn greet(name: string) -> string { return name; }\nlet x = greet(\n";
    ShapeTest::new(code)
        .at(pos(1, 14))
        .expect_signature_help_if_available();
}

#[test]
fn basic_builtin_signature_shows_help() {
    ShapeTest::new("let x = abs(")
        .at(pos(0, 12))
        .expect_signature_help();
}

// == Multi-parameter active parameter tracking ================================

#[test]
fn multi_param_first_position() {
    let code = "function add(a, b) { return a + b; }\nadd(\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_signature_help_if_available();
}

#[test]
fn multi_param_second_position_after_comma() {
    let code = "function add(a, b) { return a + b; }\nadd(1, \n";
    ShapeTest::new(code)
        .at(pos(1, 7))
        .expect_active_parameter_min(1);
}

// == Method signature =========================================================

#[test]
fn method_signature_on_module_function() {
    let code = "mod csv { fn load(path: string) { path } }\ncsv.load(";
    ShapeTest::new(code).at(pos(1, 9)).expect_signature_help();
}

// == Nested calls =============================================================

#[test]
fn nested_calls_inner_function_shows_signature() {
    let code = "function foo(x) { return x; }\nlet y = foo(abs(\n";
    ShapeTest::new(code).at(pos(1, 16)).expect_signature_help();
}
