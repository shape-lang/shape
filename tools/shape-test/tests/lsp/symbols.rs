//! LSP document symbol tests: document-level symbol discovery for functions,
//! types, enums, and nested structures.

use shape_test::shape_test::ShapeTest;

// == Document symbols =========================================================

#[test]
fn document_symbols_for_functions_and_types() {
    let code = "\
function add(a, b) { return a + b; }
type Point { x: int, y: int }
enum Color { Red, Green, Blue }
let PI = 3.14;
";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn document_symbols_empty_file_returns_none() {
    ShapeTest::new("").expect_no_document_symbols();
}

#[test]
fn document_symbols_single_function() {
    let code = "fn greet(name: string) -> string { return name; }\n";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn document_symbols_type_with_function() {
    // Document symbols tracks functions/types/enums at top level
    let code = "\
type Widget { id: int }
function create_widget() { return Widget { id: 1 }; }
";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn document_symbols_multiple_items() {
    let code = "\
function foo() { return 1; }
function bar() { return 2; }
type Config { name: string }
";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn document_symbols_enum_with_variants() {
    let code = "\
enum Direction {
    North,
    South,
    East,
    West
}
";
    ShapeTest::new(code).expect_document_symbols();
}
