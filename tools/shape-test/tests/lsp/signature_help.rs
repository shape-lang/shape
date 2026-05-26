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

// == LSP-N §D regression flow #2: signatureHelp returns null mid-call =========
//
// Audit `v0.3-lsp-parity-audit.md` executive summary item #2: signatureHelp
// returns null for both stdlib calls (`distance(`) and user-method calls
// (`u.hello(`) mid-typing. Currently red; LSP-F closes.

#[test]
fn lsp_n_signature_help_for_user_function_call() {
    // §D #2 (stdlib leg analogue using a typed user fn at module scope):
    // `distance(` mid-typing must surface a signature. PASSES today at HEAD
    // 7813a652 — single-file characterization does not reproduce the §D
    // regression (which used the multi-file editor fixture
    // `main.shape::let d = distance(p,q)`). Regression-prevention coverage.
    let code = "\
type Point { x: number, y: number }
fn distance(a: Point, b: Point) -> number { 0.0 }
let p = Point { x: 0.0, y: 0.0 }
let q = Point { x: 1.0, y: 1.0 }
let d = distance(
";
    ShapeTest::new(code)
        .at(pos(4, 17))
        .expect_signature_help();
}

#[test]
fn lsp_n_signature_help_for_user_method_call() {
    // §D #2 (user-method leg): `u.hello(` mid-typing must surface a signature.
    // Mirrors the audit's `traits.shape` fixture (`trait Greet` + `impl Greet
    // for User { fn hello(...) }`). LSP-F closes the prior `null` return.
    //
    // Notes:
    //   - Shape requires `impl Trait for Type` (inherent `impl Type { ... }`
    //     does not parse), and trait/impl methods take an implicit `self`
    //     (explicit `self` is a semantic error). The cursor sits inside an
    //     incomplete call (`u.hello(`) which by itself is a parse error;
    //     `signature_help` recovers by stripping the dangling line before
    //     re-parsing for type / method lookup.
    let code = "\
trait Greet { fn hello(greeting: string) -> string; }
type User { name: string }
impl Greet for User {
    fn hello(greeting: string) -> string { greeting }
}
let u = User { name: \"a\" }
let s = u.hello(
";
    ShapeTest::new(code)
        .at(pos(6, 16))
        .expect_signature_help();
}
