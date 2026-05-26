//! LSP document symbol tests: document-level symbol discovery for functions,
//! types, enums, and nested structures.

use shape_test::shape_test::ShapeTest;
use tower_lsp_server::ls_types::SymbolKind;

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

// == LSP-N §D regression flow #3: documentSymbol misses trait/type/impl =======
//
// Audit `v0.3-lsp-parity-audit.md` executive summary item #3:
// `extract_document_symbols`+`item_to_document_symbols`
// (`document_symbols.rs:47-126`) is missing arms for `Item::Trait` /
// `Item::StructType` / `Item::Impl`. Outline of a file with trait+type+impl
// shows only `main`. Currently red; LSP-D closes.

#[test]
#[should_panic] // LSP-D closes — Item::Trait arm currently missing
fn lsp_n_document_symbol_includes_trait() {
    // §D #3 (trait leg): trait Drawable must surface in the outline.
    let code = "\
trait Drawable { fn draw(self); }
fn main() { }
";
    ShapeTest::new(code).expect_document_symbol_named("Drawable");
}

#[test]
#[should_panic] // LSP-D closes — Item::StructType arm currently missing
fn lsp_n_document_symbol_includes_struct_type() {
    // §D #3 (type leg): `type Point` must surface as a STRUCT symbol.
    // The current dispatch covers TypeAlias only.
    let code = "\
type Point { x: int, y: int }
fn main() { }
";
    ShapeTest::new(code).expect_document_symbol_kind_count(SymbolKind::STRUCT, 1);
}

#[test]
#[should_panic] // LSP-D closes — Item::Impl arm currently missing
fn lsp_n_document_symbol_includes_impl() {
    // §D #3 (impl leg): impl blocks must surface in the outline (typically as
    // INTERFACE or class — at minimum the impl symbol must be discoverable
    // by name).
    let code = "\
type Point { x: int, y: int }
impl Point {
    fn origin() -> Point { Point { x: 0, y: 0 } }
}
";
    ShapeTest::new(code).expect_document_symbol_named("Point");
}
