//! ADR-009 ticket D1 (slice S5) — LSP navigation, references, and
//! workspace/document symbols over GENERATED declarations, answered from
//! the compiler's SymbolId/provenance query surface (Decision 66 closing
//! rule: the LSP consumes compiler query results — never a text scan,
//! never a second evaluator; Decision 68 LSP behaviors 1/3/5).
//!
//! The negative tests are rejection row 6: decoy text occurrences of a
//! generated symbol's name (comments, string literals) must NOT be served,
//! proving the text-scan path is not answering for generated symbols; and
//! generated-symbol queries succeed for qualified names that never appear
//! as plain text in the document.
//!
//! Rename over generated symbols is slice S6.

use shape_test::shape_test::{ShapeTest, pos};

/// Zero-based lines:
///  1  annotation gen() {
///  3    comptime post(target, ctx) {          <- generator definition
///  5      method answer() -> int { 42 }
/// 10  @gen()                                  <- application site
/// 11  type Point { id: int }
/// 14  let a = p.answer()                      <- call site (cursor here)
/// 15  let b = p.answer()                      <- second call site
const GENERATED_METHOD_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
let b = p.answer()
"#;

/// Decision 68 LSP behavior 1: go-to-definition on a generated-method CALL
/// SITE opens the checked generated declaration (anchored at the
/// application line until D2 virtual documents) and links the source
/// application (line 10) + the generator definition (line 3).
#[test]
fn goto_definition_on_generated_method_call_site_links_application_and_generator() {
    ShapeTest::new(GENERATED_METHOD_PROGRAM)
        .at(pos(14, 11))
        .expect_definition_includes_lines(&[10, 3]);
}

/// Decision 68 LSP behavior 3: references on a generated method include
/// every call site (lines 14 and 15) AND the application site (line 10),
/// resolved via the compiler-issued SymbolId.
#[test]
fn references_on_generated_method_include_call_sites_and_application_site() {
    ShapeTest::new(GENERATED_METHOD_PROGRAM)
        .at(pos(14, 11))
        .expect_references_include_lines(&[14, 15, 10]);
}

/// Decision 68 LSP behavior 5: workspace symbols include generated symbols
/// via SymbolId. The qualified declaration name `Point.answer` NEVER
/// appears as plain text in the document (negative half of rejection row
/// 6: the answer cannot come from a text scan).
#[test]
fn workspace_symbols_include_generated_symbol_under_its_qualified_name() {
    assert!(
        !GENERATED_METHOD_PROGRAM.contains("Point.answer"),
        "test precondition: the qualified generated name must not appear as plain text"
    );
    ShapeTest::new(GENERATED_METHOD_PROGRAM)
        .expect_workspace_symbol_named("Point.answer", "Point.answer");
}

/// Document symbols (outline) include the generated declaration under its
/// qualified name, anchored at the application site.
#[test]
fn document_symbols_include_generated_symbol() {
    ShapeTest::new(GENERATED_METHOD_PROGRAM).expect_document_symbol_named("Point.answer");
}

/// Zero-based lines:
/// 10  // decoy: the word answer appears in this comment
/// 11  let decoy = "answer"                    <- decoy string literal
/// 13  @gen()                                  <- application site
/// 17  let a = p.answer()                      <- call site (cursor here)
const DECOY_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

// decoy: the word answer appears in this comment
let decoy = "answer"

@gen()
type Point { id: int }

let p = Point { id: 1 }
let a = p.answer()
"#;

/// Rejection row 6 (references): decoy occurrences of the generated
/// method's name in a comment (line 10) and a string literal (line 11) are
/// NOT references — the generated symbol's references are answered from
/// the compiler query surface (call site line 17 + application line 13),
/// never from a text scan.
#[test]
fn references_on_generated_method_exclude_comment_and_string_decoys() {
    ShapeTest::new(DECOY_PROGRAM)
        .at(pos(17, 11))
        .expect_references_include_lines(&[17, 13])
        .expect_references_exclude_line(10)
        .expect_references_exclude_line(11);
}

/// Rejection row 6 (definition): the decoy lines are never served as
/// definition locations for the generated method; definition opens the
/// checked declaration at the application line (13) and links the
/// generator (3).
#[test]
fn goto_definition_on_generated_method_excludes_decoy_lines() {
    ShapeTest::new(DECOY_PROGRAM)
        .at(pos(17, 11))
        .expect_definition_includes_lines(&[13, 3])
        .expect_definition_excludes_line(10)
        .expect_definition_excludes_line(11);
}

/// Zero-based lines:
/// 10  fn helper() -> int { 7 }                <- ordinary fn definition
/// 12  @gen()
/// 13  type Point { id: int }
/// 16  let x = p.answer()                      <- generated call site
/// 17  let h = helper()                        <- ordinary call site (cursor)
const MIXED_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

fn helper() -> int { 7 }

@gen()
type Point { id: int }

let p = Point { id: 1 }
let x = p.answer()
let h = helper()
"#;

/// Control: ordinary (hand-written) symbols in a document that ALSO holds
/// generated declarations keep their existing text/scope navigation —
/// the generated-symbol path answers only for generated symbols.
#[test]
fn ordinary_function_navigation_is_unchanged_in_a_generating_document() {
    ShapeTest::new(MIXED_PROGRAM)
        .at(pos(17, 9))
        .expect_definition_includes_lines(&[10]);
}
