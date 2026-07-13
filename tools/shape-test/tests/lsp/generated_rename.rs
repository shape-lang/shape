//! ADR-009 ticket D1 (slice S6) — rename semantics over GENERATED
//! declarations, answered from the compiler's SymbolId/provenance query
//! surface (Decision 68 identity-controlled rename).
//!
//! Two regimes:
//! - rejection row 5: rename on an EXPLICIT SOURCE BINDER (a name the
//!   expansion takes from source — the extend-method binder token in the
//!   generator, the extend target name) edits ONLY the source binder
//!   occurrences; ZERO edits land inside generated ranges; the expansion
//!   recomputes. Decoy text occurrences (comments, string literals) are
//!   never edited — rename is not a text scan for generated symbols.
//! - rejection row 4: rename on a WHOLLY GENERATOR-CONTROLLED name (a name
//!   computed by the generator, never written in source) is NEVER a text
//!   edit — prepare-rename declines and rename responds with the named
//!   generator-controlled report, linking the generator definition.
//!
//! No new binder syntax is built here (Decision 69 name policies are NOT
//! D1): "explicit source binder" means names the CURRENT
//! extend/materialization path takes from source.

use shape_test::shape_test::{ShapeTest, pos};

/// Zero-based lines:
///  1  annotation gen() {
///  3    comptime post(target, ctx) {          <- generator definition
///  5        method answer() -> int { 42 }    <- source binder token
/// 10  // decoy: the word answer appears in this comment
/// 11  let decoy = "answer"                    <- decoy string literal
/// 13  @gen()                                  <- application site
/// 14  type Point { id: int }
/// 17  let a = p.answer()                      <- call site (cursor here)
/// 18  let b = p.answer()                      <- second call site
const SOURCE_BINDER_PROGRAM: &str = r#"
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
let b = p.answer()
"#;

/// Rejection row 5: rename on a generated method whose name is an explicit
/// source binder (`method answer()` written in the generator) edits the
/// binder token (line 5) and the call sites (lines 17, 18) — and NOTHING
/// else: the decoy comment (10) and decoy string literal (11) are not
/// edited, proving the text-scan path is not answering for generated
/// symbols.
#[test]
fn rename_on_generated_method_edits_source_binder_and_call_sites_only() {
    ShapeTest::new(SOURCE_BINDER_PROGRAM)
        .at(pos(17, 11))
        .expect_rename_edits_include_lines("solution", &[5, 17, 18])
        .expect_rename_edits_exclude_line("solution", 10)
        .expect_rename_edits_exclude_line("solution", 11);
}

/// Rejection row 5 (generated-range half): ZERO rename edits land inside
/// the generated-declaration ranges reported by the compiler query surface
/// — the generated declaration is recomputed, never text-edited.
#[test]
fn rename_on_generated_method_never_edits_generated_ranges() {
    ShapeTest::new(SOURCE_BINDER_PROGRAM)
        .at(pos(17, 11))
        .expect_rename_edits_outside_generated_ranges("solution");
}

/// Rejection row 5 (extend target name): renaming the TARGET type `Point`
/// (an explicit source binder the expansion consumes) edits the type
/// declaration (line 14) and the constructor use (line 16); zero edits land
/// inside generated ranges — the generated `Point.answer` recomputes.
#[test]
fn rename_on_extend_target_name_edits_source_only_and_recomputes() {
    ShapeTest::new(SOURCE_BINDER_PROGRAM)
        .at(pos(14, 6))
        .expect_rename_edits_include_lines("Spot", &[14, 16])
        .expect_rename_edits_outside_generated_ranges("Spot");
}

/// Zero-based lines:
///  1  annotation gen() {
///  3    comptime post(target, ctx) {          <- generator definition
///  4      let suffix = "swer"
///  5      extend (f"... method an{suffix}() ...")  <- computed name
///  9  @gen()                                  <- application site
/// 13  let x = p.answer()                      <- call site (cursor here)
///
/// The generated method is `Point.answer`, but the name token `answer`
/// never appears inside the generator definition — the name is WHOLLY
/// GENERATOR-CONTROLLED (concatenated at expansion time).
const GENERATOR_CONTROLLED_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    let suffix = "swer"
    extend (f"extend {target.name} \{ method an{suffix}() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
let x = p.answer()
"#;

/// Rejection row 4 (prepare): prepare-rename on a wholly
/// generator-controlled generated name declines — the editor never even
/// offers a text-edit rename for it.
#[test]
fn prepare_rename_declines_on_generator_controlled_name() {
    // Test precondition: the name is computed — its token never appears in
    // the generator definition (only at the call site).
    let annotation_block = &GENERATOR_CONTROLLED_PROGRAM[..GENERATOR_CONTROLLED_PROGRAM
        .find("@gen()")
        .expect("application site")];
    assert!(
        !annotation_block.contains("answer"),
        "test precondition: the generated name must not appear in the generator"
    );
    ShapeTest::new(GENERATOR_CONTROLLED_PROGRAM)
        .at(pos(13, 11))
        .expect_prepare_rename_none();
}

/// Rejection row 4 (rename): rename on a wholly generator-controlled name
/// is NEVER a text edit — no workspace edit is produced, and the response
/// is the named generator-controlled report linking the generator
/// definition (line 3).
#[test]
fn rename_on_generator_controlled_name_is_a_report_not_a_text_edit() {
    ShapeTest::new(GENERATOR_CONTROLLED_PROGRAM)
        .at(pos(13, 11))
        .expect_rename_none("solution")
        .expect_rename_generator_controlled(
            "solution",
            shape_lsp::rename::GENERATOR_CONTROLLED_NAME_RENAME_REPORT,
            3,
        );
}

/// Zero-based lines:
/// 10  fn helper() -> int { 7 }                <- ordinary fn definition
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
/// generated declarations keep the existing rename behavior — the
/// generated-symbol classification answers only for generated symbols.
#[test]
fn ordinary_function_rename_is_unchanged_in_a_generating_document() {
    ShapeTest::new(MIXED_PROGRAM)
        .at(pos(17, 9))
        .expect_rename_edits_include_lines("assistant", &[10, 17]);
}
