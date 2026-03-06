//! LSP code lens tests: reference count lenses on functions, types, and traits.

use shape_test::shape_test::ShapeTest;

// == Function reference count lens ============================================

#[test]
fn function_reference_count_lens_present() {
    let code = "\
function myFunc() { return 1; }
let a = myFunc();
let b = myFunc();
";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_at_line(0);
}

#[test]
fn function_lens_has_commands() {
    let code = "\
function helper() { return 42; }
let x = helper();
";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_has_commands();
}

// == Type reference count lens ================================================

#[test]
fn type_with_function_shows_lens() {
    // Code lens tracks functions; include a function that uses the type
    let code = "\
type Point { x: int, y: int }
function make_point() { return Point { x: 1, y: 2 }; }
let p = make_point();
let q = make_point();
";
    ShapeTest::new(code).expect_code_lens_not_empty();
}

// == Trait impl lens ==========================================================

#[test]
fn trait_definition_shows_lens() {
    // TDD: trait code lens (implementations count) may not be implemented
    let code = "\
trait Renderable {
    render(): string
}
impl Renderable for Widget {
    method render() { \"widget\" }
}
";
    ShapeTest::new(code).expect_code_lens_not_empty();
}

// == Multiple definitions =====================================================

#[test]
fn multiple_functions_each_get_lens() {
    let code = "\
function foo() { return 1; }
function bar() { return foo(); }
let x = bar();
";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_at_line(0)
        .expect_code_lens_at_line(1);
}
