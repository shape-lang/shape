//! LSP folding range tests: function bodies, type bodies, blocks, comments.
//! TDD: ShapeTest does not expose folding_ranges() — tests use semantic tokens
//! and document symbols as proxies to verify the document is well-structured
//! enough to support folding.

use shape_test::shape_test::ShapeTest;

// == Function body folds ======================================================

#[test]
fn function_body_produces_semantic_tokens() {
    // TDD: folding_ranges() not exposed; verify structure via semantic tokens
    let code = "\
function compute(x) {
    let a = x + 1;
    let b = a * 2;
    return b;
}
";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_document_symbols();
}

#[test]
fn multi_function_bodies_foldable() {
    // TDD: folding_ranges() not exposed; verify structure via semantic tokens
    let code = "\
function first() {
    return 1;
}
function second() {
    return 2;
}
";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_document_symbols();
}

// == Type body folds ==========================================================

#[test]
fn type_body_produces_tokens() {
    // TDD: folding_ranges() not exposed; verify structure via semantic tokens
    let code = "\
type Rectangle {
    width: int,
    height: int
}
";
    ShapeTest::new(code).expect_semantic_tokens();
}

#[test]
fn enum_body_produces_symbols() {
    // TDD: folding_ranges() not exposed; verify structure via document symbols
    let code = "\
enum Color {
    Red,
    Green,
    Blue
}
";
    ShapeTest::new(code).expect_document_symbols();
}

// == Block folds ==============================================================

#[test]
fn if_else_block_produces_tokens() {
    // TDD: folding_ranges() not exposed; verify structure via semantic tokens
    let code = "\
fn test(x: int) -> int {
    if x > 0 {
        return x;
    } else {
        return -x;
    }
}
";
    ShapeTest::new(code).expect_semantic_tokens();
}

// == Comment folds ============================================================

#[test]
fn consecutive_comments_produce_tokens() {
    // TDD: folding_ranges() not exposed; verify structure via semantic tokens
    let code = "\
// Line one
// Line two
// Line three
let x = 42;
";
    ShapeTest::new(code).expect_semantic_tokens();
}
