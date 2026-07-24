//! ADR-009 ticket D1 (slice S4) — provenance-carrying diagnostics for
//! generated declarations, end-to-end through the LSP diagnostic bridge.
//!
//! Rejection row 7 (Decision 66 item 4 / Decision 68 LSP behavior 2): a
//! diagnostic raised inside a GENERATED method body must carry the
//! generated node, the application site, and the generator definition as
//! navigable locations — mapped to LSP `relatedInformation`, asserted here
//! as related LOCATIONS (message fragment + resolved line), not rendered
//! strings.
//!
//! Navigation/references/rename over generated symbols are slice S5; this
//! module covers the S4 diagnostics surface.

use shape_test::shape_test::ShapeTest;

/// The generated method body calls an undefined function. The surfaced
/// diagnostic names the generated declaration and links all three
/// provenance locations:
/// - the generated node / checked declaration (anchors at the application
///   line until D2 virtual documents),
/// - the application site (`@bad_gen()`, line 10 zero-based),
/// - the generator definition (the `comptime post` handler, line 3
///   zero-based).
#[test]
fn generated_method_body_error_links_generated_node_application_and_generator() {
    let code = r#"
annotation bad_gen() on type {
  comptime post(target, ctx) {
    extend target {
      method broken() -> int { missing_helper() }
    }
  }
}

@bad_gen()
type Point { id: int }
"#;
    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("error in generated declaration `Point.broken`")
        .expect_semantic_diagnostic_related_locations(
            "error in generated declaration `Point.broken`",
            &[
                ("in generated declaration `Point.broken`", 9),
                ("generated from this application site", 9),
                ("generator defined here", 2),
            ],
        );
}

/// The generated-node note carries the structured node path (Decision 68
/// `GeneratedOrigin.node_path`) so the failing node inside the generated
/// declaration is attributable without virtual documents (ticket D2).
#[test]
fn generated_method_body_error_note_carries_the_node_path() {
    let code = r#"
annotation bad_gen() on type {
  comptime post(target, ctx) {
    extend target {
      method broken() -> int { missing_helper() }
    }
  }
}

@bad_gen()
type Point { id: int }
"#;
    ShapeTest::new(code).expect_semantic_diagnostic_related_locations(
        "error in generated declaration `Point.broken`",
        &[("generated node extend:Point/method:broken", 9)],
    );
}

/// Control: a healthy generated method produces NO generated-declaration
/// diagnostic — the provenance-carrying error is raised only when a
/// diagnostic actually originates on a generated declaration.
#[test]
fn healthy_generated_method_produces_no_generated_decl_diagnostic() {
    let code = r#"
annotation gen() on type {
  comptime post(target, ctx) {
    extend target {
      method answer() -> int { 42 }
    }
  }
}

@gen()
type Point { id: int }
"#;
    ShapeTest::new(code).expect_no_semantic_diagnostic_contains("error in generated declaration");
}
